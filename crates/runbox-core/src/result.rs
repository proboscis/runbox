use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Execution result for a completed Run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    /// Unique identifier for the result (format: result_{uuid})
    pub result_id: String,
    /// Reference to the associated run
    pub run_id: String,
    /// Execution details
    pub execution: Execution,
    /// Output references (stdout/stderr)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Output>,
    /// List of artifacts produced
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<Artifact>,
}

/// Execution timing and result information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Execution {
    /// When execution started (ISO8601)
    pub started_at: DateTime<Utc>,
    /// When execution finished (ISO8601)
    pub finished_at: DateTime<Utc>,
    /// Process exit code
    pub exit_code: i32,
    /// Duration in milliseconds
    pub duration_ms: i64,
}

/// References to stdout/stderr content
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Output {
    /// Reference to stdout content
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout_ref: Option<String>,
    /// Reference to stderr content
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr_ref: Option<String>,
}

/// An artifact produced during execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    /// Human-readable name of the artifact
    pub name: String,
    /// Original path where artifact was produced
    pub path: String,
    /// Reference to stored artifact content
    #[serde(rename = "ref")]
    pub ref_: String,
}

impl RunResult {
    /// Create a new RunResult with generated UUID
    pub fn new(
        run_id: String,
        started_at: DateTime<Utc>,
        finished_at: DateTime<Utc>,
        exit_code: i32,
    ) -> Self {
        let duration_ms = (finished_at - started_at).num_milliseconds();
        Self {
            result_id: format!("result_{}", uuid::Uuid::new_v4()),
            run_id,
            execution: Execution {
                started_at,
                finished_at,
                exit_code,
                duration_ms,
            },
            output: None,
            artifacts: Vec::new(),
        }
    }

    /// Get short ID (first 8 chars of UUID portion)
    pub fn short_id(&self) -> &str {
        // result_id format: "result_{uuid}"
        if self.result_id.len() >= 15 {
            &self.result_id[7..15]
        } else {
            &self.result_id
        }
    }

    /// Set output references
    pub fn with_output(mut self, stdout_ref: Option<String>, stderr_ref: Option<String>) -> Self {
        if stdout_ref.is_some() || stderr_ref.is_some() {
            self.output = Some(Output {
                stdout_ref,
                stderr_ref,
            });
        }
        self
    }

    /// Add an artifact
    pub fn add_artifact(&mut self, name: String, path: String, ref_: String) {
        self.artifacts.push(Artifact { name, path, ref_ });
    }

    /// Validate the RunResult
    pub fn validate(&self) -> Result<(), ResultValidationError> {
        // result_id format
        if !self.result_id.starts_with("result_") {
            return Err(ResultValidationError::InvalidResultId(self.result_id.clone()));
        }

        // run_id format
        if !self.run_id.starts_with("run_") {
            return Err(ResultValidationError::InvalidRunId(self.run_id.clone()));
        }

        // Duration should be non-negative
        if self.execution.duration_ms < 0 {
            return Err(ResultValidationError::NegativeDuration(
                self.execution.duration_ms,
            ));
        }

        // finished_at should be >= started_at
        if self.execution.finished_at < self.execution.started_at {
            return Err(ResultValidationError::InvalidTimeRange);
        }

        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ResultValidationError {
    #[error("Invalid result_id: {0}")]
    InvalidResultId(String),
    #[error("Invalid run_id: {0}")]
    InvalidRunId(String),
    #[error("Duration must be non-negative: {0}")]
    NegativeDuration(i64),
    #[error("finished_at must be >= started_at")]
    InvalidTimeRange,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_result_creation() {
        let started = Utc::now();
        let finished = started + chrono::Duration::seconds(5);

        let result = RunResult::new(
            "run_550e8400-e29b-41d4-a716-446655440000".to_string(),
            started,
            finished,
            0,
        );

        assert!(result.result_id.starts_with("result_"));
        assert_eq!(result.execution.exit_code, 0);
        assert_eq!(result.execution.duration_ms, 5000);
        assert!(result.output.is_none());
        assert!(result.artifacts.is_empty());
    }

    #[test]
    fn test_run_result_with_output() {
        let started = Utc::now();
        let finished = started + chrono::Duration::seconds(1);

        let result = RunResult::new(
            "run_test".to_string(),
            started,
            finished,
            0,
        )
        .with_output(
            Some("blobs/stdout_abc123".to_string()),
            Some("blobs/stderr_abc123".to_string()),
        );

        assert!(result.output.is_some());
        let output = result.output.unwrap();
        assert_eq!(output.stdout_ref, Some("blobs/stdout_abc123".to_string()));
        assert_eq!(output.stderr_ref, Some("blobs/stderr_abc123".to_string()));
    }

    #[test]
    fn test_run_result_add_artifact() {
        let started = Utc::now();
        let finished = started + chrono::Duration::seconds(1);

        let mut result = RunResult::new("run_test".to_string(), started, finished, 0);
        result.add_artifact(
            "build-output".to_string(),
            "/path/to/output.tar.gz".to_string(),
            "blobs/artifact_abc123".to_string(),
        );

        assert_eq!(result.artifacts.len(), 1);
        assert_eq!(result.artifacts[0].name, "build-output");
    }

    #[test]
    fn test_run_result_serialization() {
        let started = Utc::now();
        let finished = started + chrono::Duration::seconds(2);

        let mut result = RunResult::new(
            "run_550e8400-e29b-41d4-a716-446655440000".to_string(),
            started,
            finished,
            0,
        );
        result = result.with_output(Some("blobs/stdout".to_string()), None);
        result.add_artifact(
            "log".to_string(),
            "/tmp/log.txt".to_string(),
            "blobs/artifact_123".to_string(),
        );

        let json = serde_json::to_string_pretty(&result).unwrap();
        let parsed: RunResult = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.result_id, result.result_id);
        assert_eq!(parsed.run_id, result.run_id);
        assert_eq!(parsed.execution.exit_code, 0);
        assert!(parsed.output.is_some());
        assert_eq!(parsed.artifacts.len(), 1);
    }

    #[test]
    fn test_short_id() {
        let started = Utc::now();
        let finished = started + chrono::Duration::seconds(1);

        let mut result = RunResult::new("run_test".to_string(), started, finished, 0);
        result.result_id = "result_550e8400-e29b-41d4-a716-446655440000".to_string();

        assert_eq!(result.short_id(), "550e8400");
    }

    #[test]
    fn test_validation_success() {
        let started = Utc::now();
        let finished = started + chrono::Duration::seconds(1);

        let result = RunResult::new(
            "run_550e8400-e29b-41d4-a716-446655440000".to_string(),
            started,
            finished,
            0,
        );

        assert!(result.validate().is_ok());
    }

    #[test]
    fn test_validation_invalid_result_id() {
        let started = Utc::now();
        let finished = started + chrono::Duration::seconds(1);

        let mut result = RunResult::new("run_test".to_string(), started, finished, 0);
        result.result_id = "invalid_id".to_string();

        assert!(matches!(
            result.validate(),
            Err(ResultValidationError::InvalidResultId(_))
        ));
    }

    #[test]
    fn test_validation_invalid_run_id() {
        let started = Utc::now();
        let finished = started + chrono::Duration::seconds(1);

        let result = RunResult::new("invalid_run".to_string(), started, finished, 0);

        assert!(matches!(
            result.validate(),
            Err(ResultValidationError::InvalidRunId(_))
        ));
    }
}
