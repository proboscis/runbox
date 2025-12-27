use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A template for creating Runs with variable bindings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunTemplate {
    pub template_version: u32,
    pub template_id: String,
    pub name: String,
    pub exec: TemplateExec,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bindings: Option<Bindings>,
    pub code_state: TemplateCodeState,
}

/// Execution specification with template variables
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateExec {
    /// Command and arguments (template variables like "{i}" allowed)
    pub argv: Vec<String>,
    /// Working directory relative to repo root
    pub cwd: String,
    /// Environment variables (template variables allowed)
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Timeout in seconds (0 = unlimited)
    #[serde(default)]
    pub timeout_sec: u64,
}

/// Variable bindings configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bindings {
    /// Default values for variables
    #[serde(default)]
    pub defaults: HashMap<String, serde_json::Value>,
    /// Variables to prompt user for at runtime
    #[serde(default)]
    pub interactive: Vec<String>,
}

/// Code state for template (commit TBD at runtime)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateCodeState {
    /// Cloneable repository URL
    pub repo_url: String,
}

impl RunTemplate {
    /// Validate the template
    pub fn validate(&self) -> Result<(), TemplateValidationError> {
        if !self.template_id.starts_with("tpl_") {
            return Err(TemplateValidationError::InvalidTemplateId(
                self.template_id.clone(),
            ));
        }

        if self.exec.argv.is_empty() {
            return Err(TemplateValidationError::EmptyArgv);
        }

        Ok(())
    }

    /// Extract template variables from argv and env
    pub fn extract_variables(&self) -> Vec<String> {
        let mut vars = Vec::new();
        let re = regex::Regex::new(r"\{(\w+)\}").unwrap();

        for arg in &self.exec.argv {
            for cap in re.captures_iter(arg) {
                vars.push(cap[1].to_string());
            }
        }

        for value in self.exec.env.values() {
            for cap in re.captures_iter(value) {
                vars.push(cap[1].to_string());
            }
        }

        vars.sort();
        vars.dedup();
        vars
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TemplateValidationError {
    #[error("Invalid template_id: {0} (must start with 'tpl_')")]
    InvalidTemplateId(String),
    #[error("argv must not be empty")]
    EmptyArgv,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_variables() {
        let template = RunTemplate {
            template_version: 0,
            template_id: "tpl_test".to_string(),
            name: "Test".to_string(),
            exec: TemplateExec {
                argv: vec![
                    "python".to_string(),
                    "-m".to_string(),
                    "runner".to_string(),
                    "--i".to_string(),
                    "{i}".to_string(),
                    "--seed".to_string(),
                    "{seed}".to_string(),
                ],
                cwd: ".".to_string(),
                env: HashMap::from([("OUTPUT".to_string(), "{output_dir}".to_string())]),
                timeout_sec: 0,
            },
            bindings: None,
            code_state: TemplateCodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
            },
        };

        let vars = template.extract_variables();
        assert_eq!(vars, vec!["i", "output_dir", "seed"]);
    }
}
