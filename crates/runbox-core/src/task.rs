//! Task - Live process management
//!
//! A Task represents a running or recently-run process.
//! Tasks are transient - they exist while a process is running and are
//! cleaned up after completion. The persistent data lives in Record.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A live task (running process)
///
/// Tasks are transient and linked to a Record for persistence.
/// The ID format is `task_<uuid>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// Task format version
    pub task_version: u32,
    /// Unique identifier (task_<uuid>)
    pub task_id: String,
    /// Associated record ID
    pub record_id: String,
    
    // === Runtime State ===
    /// Runtime type (background, tmux, zellij)
    pub runtime: TaskRuntime,
    /// Current status
    pub status: TaskStatus,
    /// Runtime-specific handle for process control
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handle: Option<TaskHandle>,
    
    // === Timeline ===
    /// When the task was created
    pub created_at: DateTime<Utc>,
    /// When the task started running
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    /// When the task ended (completed, failed, killed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
    
    // === Process Info ===
    /// Exit code (when completed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Path to log file
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_path: Option<PathBuf>,
}

/// Task runtime type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskRuntime {
    /// Background process with process group
    Background,
    /// Tmux window
    Tmux,
    /// Zellij tab
    Zellij,
}

impl std::fmt::Display for TaskRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskRuntime::Background => write!(f, "background"),
            TaskRuntime::Tmux => write!(f, "tmux"),
            TaskRuntime::Zellij => write!(f, "zellij"),
        }
    }
}

impl std::str::FromStr for TaskRuntime {
    type Err = String;
    
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "background" | "bg" => Ok(TaskRuntime::Background),
            "tmux" => Ok(TaskRuntime::Tmux),
            "zellij" => Ok(TaskRuntime::Zellij),
            _ => Err(format!("Invalid runtime: {}. Valid: background, tmux, zellij", s)),
        }
    }
}

/// Task status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    /// Task created but not yet started
    #[default]
    Pending,
    /// Task is running
    Running,
    /// Task completed successfully (exit 0)
    Completed,
    /// Task failed (non-zero exit)
    Failed,
    /// Task was killed/stopped
    Killed,
    /// Task status unknown (process lost)
    Unknown,
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskStatus::Pending => write!(f, "pending"),
            TaskStatus::Running => write!(f, "running"),
            TaskStatus::Completed => write!(f, "completed"),
            TaskStatus::Failed => write!(f, "failed"),
            TaskStatus::Killed => write!(f, "killed"),
            TaskStatus::Unknown => write!(f, "unknown"),
        }
    }
}

/// Runtime-specific handle for process control
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum TaskHandle {
    /// Background process handle
    Background {
        /// Process ID
        pid: u32,
        /// Process group ID (for signal propagation)
        pgid: u32,
    },
    /// Tmux session handle
    Tmux {
        /// Tmux session name
        session: String,
        /// Tmux window name
        window: String,
    },
    /// Zellij session handle
    Zellij {
        /// Zellij session name
        session: String,
        /// Zellij tab name
        tab: String,
    },
}

impl Task {
    /// Create a new Task with generated ID
    pub fn new(record_id: String, runtime: TaskRuntime) -> Self {
        let task_id = format!("task_{}", uuid::Uuid::new_v4());
        Self {
            task_version: 0,
            task_id,
            record_id,
            runtime,
            status: TaskStatus::Pending,
            handle: None,
            created_at: Utc::now(),
            started_at: None,
            ended_at: None,
            exit_code: None,
            log_path: None,
        }
    }
    
    /// Get the short ID (first 8 hex characters of UUID)
    pub fn short_id(&self) -> &str {
        // task_id format: "task_{uuid}"
        if self.task_id.len() >= 13 {
            &self.task_id[5..13]
        } else {
            &self.task_id
        }
    }
    
    /// Mark the task as started with a handle
    pub fn mark_started(&mut self, handle: TaskHandle) {
        self.status = TaskStatus::Running;
        self.handle = Some(handle);
        self.started_at = Some(Utc::now());
    }
    
    /// Mark the task as completed
    pub fn mark_completed(&mut self, exit_code: i32) {
        self.status = if exit_code == 0 {
            TaskStatus::Completed
        } else {
            TaskStatus::Failed
        };
        self.exit_code = Some(exit_code);
        self.ended_at = Some(Utc::now());
    }
    
    /// Mark the task as killed
    pub fn mark_killed(&mut self) {
        self.status = TaskStatus::Killed;
        self.ended_at = Some(Utc::now());
    }
    
    /// Mark the task as unknown (process lost)
    pub fn mark_unknown(&mut self, reason: &str) {
        self.status = TaskStatus::Unknown;
        self.ended_at = Some(Utc::now());
        // Note: reason could be stored in a separate field if needed
        let _ = reason;
    }
    
    /// Check if the task is still running
    pub fn is_running(&self) -> bool {
        self.status == TaskStatus::Running
    }
    
    /// Check if the task has ended (any terminal state)
    pub fn is_ended(&self) -> bool {
        matches!(
            self.status,
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Killed | TaskStatus::Unknown
        )
    }
    
    /// Get the PID if this is a background task
    pub fn pid(&self) -> Option<u32> {
        match &self.handle {
            Some(TaskHandle::Background { pid, .. }) => Some(*pid),
            _ => None,
        }
    }
    
    /// Get the process group ID if this is a background task
    pub fn pgid(&self) -> Option<u32> {
        match &self.handle {
            Some(TaskHandle::Background { pgid, .. }) => Some(*pgid),
            _ => None,
        }
    }
    
    /// Get the duration in milliseconds (if started and ended)
    pub fn duration_ms(&self) -> Option<i64> {
        match (self.started_at, self.ended_at) {
            (Some(start), Some(end)) => Some((end - start).num_milliseconds()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_task_creation() {
        let task = Task::new("rec_123".to_string(), TaskRuntime::Background);
        
        assert!(task.task_id.starts_with("task_"));
        assert_eq!(task.record_id, "rec_123");
        assert_eq!(task.runtime, TaskRuntime::Background);
        assert_eq!(task.status, TaskStatus::Pending);
        assert!(task.handle.is_none());
    }
    
    #[test]
    fn test_task_lifecycle() {
        let mut task = Task::new("rec_123".to_string(), TaskRuntime::Background);
        
        assert_eq!(task.status, TaskStatus::Pending);
        assert!(!task.is_running());
        assert!(!task.is_ended());
        
        // Start
        task.mark_started(TaskHandle::Background { pid: 1234, pgid: 1234 });
        assert_eq!(task.status, TaskStatus::Running);
        assert!(task.is_running());
        assert!(!task.is_ended());
        assert_eq!(task.pid(), Some(1234));
        
        // Complete
        task.mark_completed(0);
        assert_eq!(task.status, TaskStatus::Completed);
        assert!(!task.is_running());
        assert!(task.is_ended());
        assert_eq!(task.exit_code, Some(0));
    }
    
    #[test]
    fn test_task_failure() {
        let mut task = Task::new("rec_123".to_string(), TaskRuntime::Background);
        
        task.mark_started(TaskHandle::Background { pid: 1234, pgid: 1234 });
        task.mark_completed(1);
        
        assert_eq!(task.status, TaskStatus::Failed);
        assert_eq!(task.exit_code, Some(1));
    }
    
    #[test]
    fn test_task_killed() {
        let mut task = Task::new("rec_123".to_string(), TaskRuntime::Tmux);
        
        task.mark_started(TaskHandle::Tmux {
            session: "runbox".to_string(),
            window: "run_123".to_string(),
        });
        task.mark_killed();
        
        assert_eq!(task.status, TaskStatus::Killed);
        assert!(task.is_ended());
    }
    
    #[test]
    fn test_task_serialization() {
        let mut task = Task::new("rec_123".to_string(), TaskRuntime::Background);
        task.mark_started(TaskHandle::Background { pid: 1234, pgid: 1234 });
        task.log_path = Some(PathBuf::from("/tmp/log.txt"));
        
        let json = serde_json::to_string_pretty(&task).unwrap();
        let parsed: Task = serde_json::from_str(&json).unwrap();
        
        assert_eq!(parsed.task_id, task.task_id);
        assert_eq!(parsed.record_id, "rec_123");
        assert_eq!(parsed.runtime, TaskRuntime::Background);
        assert_eq!(parsed.pid(), Some(1234));
    }
    
    #[test]
    fn test_runtime_parsing() {
        assert_eq!("background".parse::<TaskRuntime>().unwrap(), TaskRuntime::Background);
        assert_eq!("bg".parse::<TaskRuntime>().unwrap(), TaskRuntime::Background);
        assert_eq!("tmux".parse::<TaskRuntime>().unwrap(), TaskRuntime::Tmux);
        assert_eq!("zellij".parse::<TaskRuntime>().unwrap(), TaskRuntime::Zellij);
        assert!("invalid".parse::<TaskRuntime>().is_err());
    }
    
    #[test]
    fn test_status_display() {
        assert_eq!(TaskStatus::Pending.to_string(), "pending");
        assert_eq!(TaskStatus::Running.to_string(), "running");
        assert_eq!(TaskStatus::Completed.to_string(), "completed");
        assert_eq!(TaskStatus::Failed.to_string(), "failed");
        assert_eq!(TaskStatus::Killed.to_string(), "killed");
        assert_eq!(TaskStatus::Unknown.to_string(), "unknown");
    }
    
    #[test]
    fn test_short_id() {
        let task = Task::new("rec_123".to_string(), TaskRuntime::Background);
        assert_eq!(task.short_id().len(), 8);
    }
}
