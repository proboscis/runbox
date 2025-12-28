use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// A fully-resolved, reproducible execution record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub run_version: u32,
    pub run_id: String,
    pub exec: Exec,
    pub code_state: CodeState,

    // Execution state (optional for backwards compatibility)
    #[serde(default)]
    pub status: RunStatus,
    #[serde(default)]
    pub runtime: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handle: Option<RuntimeHandle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_ref: Option<LogRef>,
    #[serde(default)]
    pub timeline: Timeline,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

/// Run execution status
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    #[default]
    Pending,
    Running,
    Exited,
    Failed,
    Killed,
    Unknown,
}

impl std::fmt::Display for RunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunStatus::Pending => write!(f, "pending"),
            RunStatus::Running => write!(f, "running"),
            RunStatus::Exited => write!(f, "exited"),
            RunStatus::Failed => write!(f, "failed"),
            RunStatus::Killed => write!(f, "killed"),
            RunStatus::Unknown => write!(f, "unknown"),
        }
    }
}

/// Runtime-specific handle data
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RuntimeHandle {
    Background { pid: u32, pgid: u32 },
    Tmux { session: String, window: String },
    Zellij { session: String, tab: String },
}

/// Reference to log file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogRef {
    pub path: PathBuf,
}

/// Timeline of run execution
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
            runtime: String::new(),
            handle: None,
            log_ref: None,
            timeline: Timeline {
                created_at: Some(Utc::now()),
                started_at: None,
                ended_at: None,
            },
            exit_code: None,
        }
    }

    /// Get short ID (first 8 chars of UUID part)
    pub fn short_id(&self) -> &str {
        // run_id format is "run_<uuid>", so skip "run_" prefix
        let uuid_part = self.run_id.strip_prefix("run_").unwrap_or(&self.run_id);
        // Return first 8 chars or full string if shorter
        if uuid_part.len() >= 8 {
            &uuid_part[..8]
        } else {
            uuid_part
        }
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
            runtime: String::new(),
            handle: None,
            log_ref: None,
            timeline: Timeline::default(),
            exit_code: None,
        };

        let json = serde_json::to_string_pretty(&run).unwrap();
        let parsed: Run = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.run_id, run.run_id);
    }

    #[test]
    fn test_run_with_handle() {
        let run = Run {
            run_version: 0,
            run_id: "run_550e8400-e29b-41d4-a716-446655440000".to_string(),
            exec: Exec {
                argv: vec!["python".to_string(), "train.py".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
                timeout_sec: 0,
            },
            code_state: CodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                base_commit: "a1b2c3d4e5f6789012345678901234567890abcd".to_string(),
                patch: None,
            },
            status: RunStatus::Running,
            runtime: "tmux".to_string(),
            handle: Some(RuntimeHandle::Tmux {
                session: "runbox".to_string(),
                window: "550e8400".to_string(),
            }),
            log_ref: Some(LogRef {
                path: PathBuf::from("/Users/me/.local/share/runbox/logs/run_550e8400.log"),
            }),
            timeline: Timeline {
                created_at: Some(Utc::now()),
                started_at: Some(Utc::now()),
                ended_at: None,
            },
            exit_code: None,
        };

        let json = serde_json::to_string_pretty(&run).unwrap();
        let parsed: Run = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.status, RunStatus::Running);
        assert!(matches!(parsed.handle, Some(RuntimeHandle::Tmux { .. })));
    }

    #[test]
    fn test_short_id() {
        let run = Run {
            run_version: 0,
            run_id: "run_550e8400-e29b-41d4-a716-446655440000".to_string(),
            exec: Exec {
                argv: vec!["echo".to_string()],
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
            runtime: String::new(),
            handle: None,
            log_ref: None,
            timeline: Timeline::default(),
            exit_code: None,
        };

        assert_eq!(run.short_id(), "550e8400");
    }

    #[test]
    fn test_backwards_compatibility() {
        // Test that old JSON without new fields can still be deserialized
        let old_json = r#"{
            "run_version": 0,
            "run_id": "run_test123",
            "exec": {
                "argv": ["echo", "hello"],
                "cwd": "."
            },
            "code_state": {
                "repo_url": "git@github.com:org/repo.git",
                "base_commit": "a1b2c3d4e5f6789012345678901234567890abcd"
            }
        }"#;

        let run: Run = serde_json::from_str(old_json).unwrap();
        assert_eq!(run.run_id, "run_test123");
        assert_eq!(run.status, RunStatus::Pending);
        assert!(run.handle.is_none());
    }
}
