use anyhow::{Context, Result};
use jsonschema::JSONSchema;
use serde_json::Value;
use std::path::Path;

pub struct Validator {
    run_schema: JSONSchema,
    template_schema: JSONSchema,
    playlist_schema: JSONSchema,
    result_schema: JSONSchema,
}

const RUN_SCHEMA: &str = include_str!("../../../specs/run.schema.json");
const TEMPLATE_SCHEMA: &str = include_str!("../../../specs/run_template.schema.json");
const PLAYLIST_SCHEMA: &str = include_str!("../../../specs/playlist.schema.json");
const RESULT_SCHEMA: &str = include_str!("../../../specs/run_result.schema.json");

impl Validator {
    pub fn new() -> Result<Self> {
        let run_schema_value: Value = serde_json::from_str(RUN_SCHEMA)?;
        let template_schema_value: Value = serde_json::from_str(TEMPLATE_SCHEMA)?;
        let playlist_schema_value: Value = serde_json::from_str(PLAYLIST_SCHEMA)?;
        let result_schema_value: Value = serde_json::from_str(RESULT_SCHEMA)?;

        let run_schema = JSONSchema::compile(&run_schema_value)
            .map_err(|e| anyhow::anyhow!("Invalid run schema: {}", e))?;
        let template_schema = JSONSchema::compile(&template_schema_value)
            .map_err(|e| anyhow::anyhow!("Invalid template schema: {}", e))?;
        let playlist_schema = JSONSchema::compile(&playlist_schema_value)
            .map_err(|e| anyhow::anyhow!("Invalid playlist schema: {}", e))?;
        let result_schema = JSONSchema::compile(&result_schema_value)
            .map_err(|e| anyhow::anyhow!("Invalid result schema: {}", e))?;

        Ok(Self {
            run_schema,
            template_schema,
            playlist_schema,
            result_schema,
        })
    }

    /// Validate a Run JSON
    pub fn validate_run(&self, value: &Value) -> Result<()> {
        let result = self.run_schema.validate(value);
        if let Err(errors) = result {
            let error_msgs: Vec<String> = errors.map(|e| format!("  - {}", e)).collect();
            anyhow::bail!("Run validation failed:\n{}", error_msgs.join("\n"));
        }
        Ok(())
    }

    /// Validate a RunTemplate JSON
    pub fn validate_template(&self, value: &Value) -> Result<()> {
        let result = self.template_schema.validate(value);
        if let Err(errors) = result {
            let error_msgs: Vec<String> = errors.map(|e| format!("  - {}", e)).collect();
            anyhow::bail!("Template validation failed:\n{}", error_msgs.join("\n"));
        }
        Ok(())
    }

    pub fn validate_playlist(&self, value: &Value) -> Result<()> {
        let result = self.playlist_schema.validate(value);
        if let Err(errors) = result {
            let error_msgs: Vec<String> = errors.map(|e| format!("  - {}", e)).collect();
            anyhow::bail!("Playlist validation failed:\n{}", error_msgs.join("\n"));
        }
        Ok(())
    }

    pub fn validate_result(&self, value: &Value) -> Result<()> {
        let result = self.result_schema.validate(value);
        if let Err(errors) = result {
            let error_msgs: Vec<String> = errors.map(|e| format!("  - {}", e)).collect();
            anyhow::bail!("Result validation failed:\n{}", error_msgs.join("\n"));
        }
        Ok(())
    }

    pub fn validate_file(&self, path: &Path) -> Result<ValidationType> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read file: {}", path.display()))?;
        let value: Value = serde_json::from_str(&content)
            .with_context(|| format!("Invalid JSON in file: {}", path.display()))?;

        self.validate_auto(&value)
    }

    pub fn validate_auto(&self, value: &Value) -> Result<ValidationType> {
        if value.get("result_id").is_some() {
            self.validate_result(value)?;
            return Ok(ValidationType::Result);
        }
        if value.get("run_id").is_some() {
            self.validate_run(value)?;
            return Ok(ValidationType::Run);
        }
        if value.get("template_id").is_some() {
            self.validate_template(value)?;
            return Ok(ValidationType::Template);
        }
        if value.get("playlist_id").is_some() {
            self.validate_playlist(value)?;
            return Ok(ValidationType::Playlist);
        }

        anyhow::bail!("Could not determine JSON type (expected result_id, run_id, template_id, or playlist_id)")
    }
}

impl Default for Validator {
    fn default() -> Self {
        Self::new().expect("Failed to create validator")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationType {
    Run,
    Template,
    Playlist,
    Result,
}

impl std::fmt::Display for ValidationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValidationType::Run => write!(f, "Run"),
            ValidationType::Template => write!(f, "RunTemplate"),
            ValidationType::Playlist => write!(f, "Playlist"),
            ValidationType::Result => write!(f, "RunResult"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_run() {
        let validator = Validator::new().unwrap();
        let run_json = serde_json::json!({
            "run_version": 0,
            "run_id": "run_550e8400-e29b-41d4-a716-446655440000",
            "exec": {
                "argv": ["echo", "hello"],
                "cwd": "."
            },
            "code_state": {
                "repo_url": "git@github.com:org/repo.git",
                "base_commit": "a1b2c3d4e5f6789012345678901234567890abcd"
            }
        });

        assert!(validator.validate_run(&run_json).is_ok());
    }

    #[test]
    fn test_validate_template() {
        let validator = Validator::new().unwrap();
        let template_json = serde_json::json!({
            "template_version": 0,
            "template_id": "tpl_test",
            "name": "Test Template",
            "exec": {
                "argv": ["echo", "{msg}"],
                "cwd": "."
            },
            "code_state": {
                "repo_url": "git@github.com:org/repo.git"
            }
        });

        assert!(validator.validate_template(&template_json).is_ok());
    }

    #[test]
    fn test_validate_playlist() {
        let validator = Validator::new().unwrap();
        let playlist_json = serde_json::json!({
            "playlist_id": "pl_daily",
            "name": "Daily Tasks",
            "items": [
                {"template_id": "tpl_runner"},
                {"template_id": "tpl_eval", "label": "Evaluation"}
            ]
        });

        assert!(validator.validate_playlist(&playlist_json).is_ok());
    }

    #[test]
    fn test_validate_result() {
        let validator = Validator::new().unwrap();
        let result_json = serde_json::json!({
            "result_id": "result_550e8400-e29b-41d4-a716-446655440000",
            "run_id": "run_550e8400-e29b-41d4-a716-446655440000",
            "execution": {
                "started_at": "2025-01-01T00:00:00Z",
                "finished_at": "2025-01-01T00:00:05Z",
                "exit_code": 0,
                "duration_ms": 5000
            }
        });

        assert!(validator.validate_result(&result_json).is_ok());
    }

    #[test]
    fn test_validate_result_with_output_and_artifacts() {
        let validator = Validator::new().unwrap();
        let result_json = serde_json::json!({
            "result_id": "result_550e8400-e29b-41d4-a716-446655440000",
            "run_id": "run_550e8400-e29b-41d4-a716-446655440000",
            "execution": {
                "started_at": "2025-01-01T00:00:00Z",
                "finished_at": "2025-01-01T00:00:05Z",
                "exit_code": 0,
                "duration_ms": 5000
            },
            "output": {
                "stdout_ref": "blobs/abc123",
                "stderr_ref": "blobs/def456"
            },
            "artifacts": [
                {"name": "build-output", "path": "/tmp/out.tar.gz", "ref": "blobs/xyz789"}
            ]
        });

        assert!(validator.validate_result(&result_json).is_ok());
    }

    #[test]
    fn test_auto_detect() {
        let validator = Validator::new().unwrap();

        let result_json = serde_json::json!({
            "result_id": "result_550e8400-e29b-41d4-a716-446655440000",
            "run_id": "run_test",
            "execution": {
                "started_at": "2025-01-01T00:00:00Z",
                "finished_at": "2025-01-01T00:00:05Z",
                "exit_code": 0,
                "duration_ms": 5000
            }
        });
        assert_eq!(
            validator.validate_auto(&result_json).unwrap(),
            ValidationType::Result
        );

        let run_json = serde_json::json!({"run_id": "run_550e8400-e29b-41d4-a716-446655440000", "exec": {"argv": ["x"], "cwd": "."}, "code_state": {"repo_url": "x", "base_commit": "a1b2c3d4e5f6789012345678901234567890abcd"}});
        assert_eq!(
            validator.validate_auto(&run_json).unwrap(),
            ValidationType::Run
        );

        let template_json = serde_json::json!({"template_id": "tpl_x", "name": "x", "exec": {"cwd": "."}, "code_state": {"repo_url": "x"}});
        assert_eq!(
            validator.validate_auto(&template_json).unwrap(),
            ValidationType::Template
        );

        let playlist_json = serde_json::json!({"playlist_id": "pl_x", "name": "x"});
        assert_eq!(
            validator.validate_auto(&playlist_json).unwrap(),
            ValidationType::Playlist
        );
    }
}
