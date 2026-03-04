//! Record - Persistent execution record
//!
//! A Record represents a completed or historical command execution.
//! It contains the git state, command, and result but NOT runtime state.
//! Records are stored as JSON files and never deleted (immutable history).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A persistent execution record
///
/// Records are immutable after creation - they represent historical data.
/// The ID format is `rec_<uuid>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    /// Record format version for forward compatibility
    pub record_version: u32,
    /// Unique identifier (rec_<uuid>)
    pub record_id: String,

    // === Git State ===
    /// Git repository state for reproducibility
    pub git_state: RecordGitState,

    // === Execution Specification ===
    /// The command that was executed
    pub command: RecordCommand,

    // === Result (filled after execution completes) ===
    /// Exit code (None if not yet completed or interrupted)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// When the command started
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    /// When the command ended
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,

    // === Metadata ===
    /// When this record was created
    pub created_at: DateTime<Utc>,
    /// Reference to log file (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_ref: Option<String>,
    /// User-defined tags for filtering
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Source of the record (e.g., "runbox", "doeff")
    #[serde(default = "default_source")]
    pub source: String,
}

fn default_source() -> String {
    "runbox".to_string()
}

/// Git state for a record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordGitState {
    /// Repository URL (cloneable)
    pub repo_url: String,
    /// Full commit SHA (40 hex chars)
    pub commit: String,
    /// Optional patch reference for uncommitted changes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch_ref: Option<String>,
}

/// Command specification in a record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordCommand {
    /// Full command line (argv)
    pub argv: Vec<String>,
    /// Working directory (relative to repo root)
    pub cwd: String,
    /// Environment variables
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
}

impl Record {
    /// Create a new Record with generated ID
    pub fn new(git_state: RecordGitState, command: RecordCommand) -> Self {
        let record_id = format!("rec_{}", uuid::Uuid::new_v4());
        Self {
            record_version: 0,
            record_id,
            git_state,
            command,
            exit_code: None,
            started_at: None,
            ended_at: None,
            created_at: Utc::now(),
            log_ref: None,
            tags: Vec::new(),
            source: "runbox".to_string(),
        }
    }

    /// Create a Record with a specific ID (for external tool integration)
    pub fn with_id(record_id: String, git_state: RecordGitState, command: RecordCommand) -> Self {
        Self {
            record_version: 0,
            record_id,
            git_state,
            command,
            exit_code: None,
            started_at: None,
            ended_at: None,
            created_at: Utc::now(),
            log_ref: None,
            tags: Vec::new(),
            source: "runbox".to_string(),
        }
    }

    /// Get the short ID (first 8 hex characters of UUID)
    pub fn short_id(&self) -> &str {
        // record_id format: "rec_{uuid}"
        if self.record_id.len() >= 12 {
            &self.record_id[4..12]
        } else {
            &self.record_id
        }
    }

    /// Mark the record as started
    pub fn mark_started(&mut self) {
        self.started_at = Some(Utc::now());
    }

    /// Mark the record as completed with exit code
    pub fn mark_completed(&mut self, exit_code: i32) {
        self.exit_code = Some(exit_code);
        self.ended_at = Some(Utc::now());
    }

    /// Check if this record has completed (has exit code)
    pub fn is_completed(&self) -> bool {
        self.exit_code.is_some()
    }

    /// Get the duration in milliseconds (if completed)
    pub fn duration_ms(&self) -> Option<i64> {
        match (self.started_at, self.ended_at) {
            (Some(start), Some(end)) => Some((end - start).num_milliseconds()),
            _ => None,
        }
    }

    /// Validate the record
    pub fn validate(&self) -> Result<(), RecordValidationError> {
        // record_id format
        if !self.record_id.starts_with("rec_") {
            return Err(RecordValidationError::InvalidRecordId(
                self.record_id.clone(),
            ));
        }

        // command.argv non-empty
        if self.command.argv.is_empty() {
            return Err(RecordValidationError::EmptyCommand);
        }

        // git_state.commit format (40 hex chars)
        if self.git_state.commit.len() != 40
            || !self.git_state.commit.chars().all(|c| c.is_ascii_hexdigit())
        {
            return Err(RecordValidationError::InvalidCommit(
                self.git_state.commit.clone(),
            ));
        }

        Ok(())
    }
}

/// Validation errors for Record
#[derive(Debug, thiserror::Error)]
pub enum RecordValidationError {
    #[error("Invalid record_id: {0} (must start with 'rec_')")]
    InvalidRecordId(String),
    #[error("Command argv must not be empty")]
    EmptyCommand,
    #[error("Invalid commit hash: {0} (must be 40 hex characters)")]
    InvalidCommit(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_creation() {
        let record = Record::new(
            RecordGitState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                commit: "a1b2c3d4e5f6789012345678901234567890abcd".to_string(),
                patch_ref: None,
            },
            RecordCommand {
                argv: vec!["echo".to_string(), "hello".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
            },
        );

        assert!(record.record_id.starts_with("rec_"));
        assert_eq!(record.short_id().len(), 8);
        assert!(!record.is_completed());
    }

    #[test]
    fn test_record_lifecycle() {
        let mut record = Record::new(
            RecordGitState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                commit: "a1b2c3d4e5f6789012345678901234567890abcd".to_string(),
                patch_ref: None,
            },
            RecordCommand {
                argv: vec!["echo".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
            },
        );

        assert!(!record.is_completed());
        assert!(record.started_at.is_none());

        record.mark_started();
        assert!(record.started_at.is_some());

        record.mark_completed(0);
        assert!(record.is_completed());
        assert_eq!(record.exit_code, Some(0));
        assert!(record.ended_at.is_some());
    }

    #[test]
    fn test_record_serialization() {
        let mut record = Record::new(
            RecordGitState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                commit: "a1b2c3d4e5f6789012345678901234567890abcd".to_string(),
                patch_ref: Some("refs/patches/rec_123".to_string()),
            },
            RecordCommand {
                argv: vec!["python".to_string(), "train.py".to_string()],
                cwd: "src".to_string(),
                env: [("CUDA_VISIBLE_DEVICES".to_string(), "0".to_string())]
                    .into_iter()
                    .collect(),
            },
        );
        record.tags = vec!["ml".to_string(), "training".to_string()];
        record.source = "doeff".to_string();

        let json = serde_json::to_string_pretty(&record).unwrap();
        let parsed: Record = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.record_id, record.record_id);
        assert_eq!(parsed.git_state.repo_url, "git@github.com:org/repo.git");
        assert_eq!(parsed.command.argv, vec!["python", "train.py"]);
        assert_eq!(parsed.tags, vec!["ml", "training"]);
        assert_eq!(parsed.source, "doeff");
    }

    #[test]
    fn test_record_validation() {
        // Valid record
        let record = Record::new(
            RecordGitState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                commit: "a1b2c3d4e5f6789012345678901234567890abcd".to_string(),
                patch_ref: None,
            },
            RecordCommand {
                argv: vec!["echo".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
            },
        );
        assert!(record.validate().is_ok());

        // Invalid record_id
        let mut invalid = record.clone();
        invalid.record_id = "invalid".to_string();
        assert!(matches!(
            invalid.validate(),
            Err(RecordValidationError::InvalidRecordId(_))
        ));

        // Empty command
        let mut invalid = record.clone();
        invalid.command.argv = vec![];
        assert!(matches!(
            invalid.validate(),
            Err(RecordValidationError::EmptyCommand)
        ));

        // Invalid commit
        let mut invalid = record.clone();
        invalid.git_state.commit = "short".to_string();
        assert!(matches!(
            invalid.validate(),
            Err(RecordValidationError::InvalidCommit(_))
        ));
    }

    #[test]
    fn test_record_with_custom_id() {
        let record = Record::with_id(
            "rec_my-custom-id".to_string(),
            RecordGitState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                commit: "a1b2c3d4e5f6789012345678901234567890abcd".to_string(),
                patch_ref: None,
            },
            RecordCommand {
                argv: vec!["echo".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
            },
        );

        assert_eq!(record.record_id, "rec_my-custom-id");
    }
}
