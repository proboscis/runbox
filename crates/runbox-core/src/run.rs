use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// A fully-resolved, reproducible execution record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub run_version: u32,
    pub run_id: String,

    // Existing (required)
    pub exec: Exec,
    pub code_state: CodeState,

    // New fields for execution management
    #[serde(default)]
    pub status: RunStatus,

    #[serde(default)]
    pub runtime: Runtime,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_ref: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_ref: Option<LogRef>,

    #[serde(default)]
    pub timeline: Timeline,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
}

/// Run execution status
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    #[default]
    Pending,
    Running,
    Exited,
    Failed,
    Killed,
}

impl std::fmt::Display for RunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunStatus::Pending => write!(f, "pending"),
            RunStatus::Running => write!(f, "running"),
            RunStatus::Exited => write!(f, "exited"),
            RunStatus::Failed => write!(f, "failed"),
            RunStatus::Killed => write!(f, "killed"),
        }
    }
}

/// Runtime environment for execution
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Runtime {
    #[default]
    Background,
    Tmux,
    Zellij,
}

impl std::fmt::Display for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Runtime::Background => write!(f, "background"),
            Runtime::Tmux => write!(f, "tmux"),
            Runtime::Zellij => write!(f, "zellij"),
        }
    }
}

impl std::str::FromStr for Runtime {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "bg" | "background" => Ok(Runtime::Background),
            "tmux" => Ok(Runtime::Tmux),
            "zellij" => Ok(Runtime::Zellij),
            _ => Err(format!("Unknown runtime: {}. Valid values: bg, tmux, zellij", s)),
        }
    }
}

/// Log file reference
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogRef {
    pub path: PathBuf,
}

/// Timeline tracking for run execution
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Timeline {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
}

/// Execution specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Exec {
    /// Command and arguments (non-empty, fully resolved)
    pub argv: Vec<String>,
    /// Working directory relative to repo root
    pub cwd: String,
    /// Environment variables
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Timeout in seconds (0 = unlimited)
    #[serde(default)]
    pub timeout_sec: u64,
}

/// Git code state for reproduction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeState {
    /// Cloneable repository URL
    pub repo_url: String,
    /// Full commit SHA (40 chars)
    pub base_commit: String,
    /// Optional patch for uncommitted changes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch: Option<Patch>,
}

/// Patch reference for uncommitted changes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Patch {
    /// Git ref (refs/patches/{run_id})
    #[serde(rename = "ref")]
    pub ref_: String,
    /// SHA-256 hash of patch content
    pub sha256: String,
}

impl Run {
    /// Create a new Run with generated UUID
    pub fn new(exec: Exec, code_state: CodeState) -> Self {
        let run_id = format!("run_{}", uuid::Uuid::new_v4());
        Self {
            run_version: 0,
            run_id,
            exec,
            code_state,
            status: RunStatus::Pending,
            runtime: Runtime::Background,
            session_ref: None,
            log_ref: None,
            timeline: Timeline {
                created_at: Some(Utc::now()),
                started_at: None,
                ended_at: None,
            },
            exit_code: None,
            pid: None,
        }
    }

    /// Create a new Run with specific runtime
    pub fn new_with_runtime(exec: Exec, code_state: CodeState, runtime: Runtime) -> Self {
        let mut run = Self::new(exec, code_state);
        run.runtime = runtime;
        run
    }

    /// Mark the run as started
    pub fn mark_started(&mut self) {
        self.status = RunStatus::Running;
        self.timeline.started_at = Some(Utc::now());
    }

    /// Mark the run as completed with exit code
    pub fn mark_completed(&mut self, exit_code: i32) {
        self.status = if exit_code == 0 {
            RunStatus::Exited
        } else {
            RunStatus::Failed
        };
        self.exit_code = Some(exit_code);
        self.timeline.ended_at = Some(Utc::now());
    }

    /// Mark the run as killed
    pub fn mark_killed(&mut self) {
        self.status = RunStatus::Killed;
        self.timeline.ended_at = Some(Utc::now());
    }

    /// Check if the run is currently running
    pub fn is_running(&self) -> bool {
        self.status == RunStatus::Running
    }

    /// Check if the run has finished (exited, failed, or killed)
    pub fn is_finished(&self) -> bool {
        matches!(
            self.status,
            RunStatus::Exited | RunStatus::Failed | RunStatus::Killed
        )
    }

    /// Validate the Run
    pub fn validate(&self) -> Result<(), ValidationError> {
        // run_id format
        if !self.run_id.starts_with("run_") {
            return Err(ValidationError::InvalidRunId(self.run_id.clone()));
        }

        // argv non-empty
        if self.exec.argv.is_empty() {
            return Err(ValidationError::EmptyArgv);
        }

        // base_commit format (40 hex chars)
        if self.code_state.base_commit.len() != 40
            || !self.code_state.base_commit.chars().all(|c| c.is_ascii_hexdigit())
        {
            return Err(ValidationError::InvalidCommit(self.code_state.base_commit.clone()));
        }

        // patch ref format
        if let Some(patch) = &self.code_state.patch {
            if !patch.ref_.starts_with("refs/patches/") {
                return Err(ValidationError::InvalidPatchRef(patch.ref_.clone()));
            }
        }

        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("Invalid run_id: {0}")]
    InvalidRunId(String),
    #[error("argv must not be empty")]
    EmptyArgv,
    #[error("Invalid commit hash: {0}")]
    InvalidCommit(String),
    #[error("Invalid patch ref: {0}")]
    InvalidPatchRef(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_serialization() {
        let run = Run {
            run_version: 0,
            run_id: "run_550e8400-e29b-41d4-a716-446655440000".to_string(),
            exec: Exec {
                argv: vec!["echo".to_string(), "hello".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
                timeout_sec: 0,
            },
            code_state: CodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                base_commit: "a1b2c3d4e5f6789012345678901234567890abcd".to_string(),
                patch: None,
            },
            status: RunStatus::Pending,
            runtime: Runtime::Background,
            session_ref: None,
            log_ref: None,
            timeline: Timeline::default(),
            exit_code: None,
            pid: None,
        };

        let json = serde_json::to_string_pretty(&run).unwrap();
        let parsed: Run = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.run_id, run.run_id);
        assert_eq!(parsed.status, RunStatus::Pending);
    }

    #[test]
    fn test_run_status_transitions() {
        let mut run = Run::new(
            Exec {
                argv: vec!["test".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
                timeout_sec: 0,
            },
            CodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                base_commit: "a1b2c3d4e5f6789012345678901234567890abcd".to_string(),
                patch: None,
            },
        );

        assert_eq!(run.status, RunStatus::Pending);
        assert!(run.timeline.created_at.is_some());
        assert!(run.timeline.started_at.is_none());

        run.mark_started();
        assert_eq!(run.status, RunStatus::Running);
        assert!(run.is_running());
        assert!(run.timeline.started_at.is_some());

        run.mark_completed(0);
        assert_eq!(run.status, RunStatus::Exited);
        assert!(run.is_finished());
        assert_eq!(run.exit_code, Some(0));
        assert!(run.timeline.ended_at.is_some());
    }

    #[test]
    fn test_run_failed_status() {
        let mut run = Run::new(
            Exec {
                argv: vec!["test".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
                timeout_sec: 0,
            },
            CodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                base_commit: "a1b2c3d4e5f6789012345678901234567890abcd".to_string(),
                patch: None,
            },
        );

        run.mark_started();
        run.mark_completed(1);
        assert_eq!(run.status, RunStatus::Failed);
        assert_eq!(run.exit_code, Some(1));
    }

    #[test]
    fn test_runtime_from_str() {
        assert_eq!("bg".parse::<Runtime>().unwrap(), Runtime::Background);
        assert_eq!("background".parse::<Runtime>().unwrap(), Runtime::Background);
        assert_eq!("tmux".parse::<Runtime>().unwrap(), Runtime::Tmux);
        assert_eq!("zellij".parse::<Runtime>().unwrap(), Runtime::Zellij);
        assert!("invalid".parse::<Runtime>().is_err());
    }
}
