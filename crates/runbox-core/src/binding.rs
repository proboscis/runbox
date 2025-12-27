use crate::{CodeState, Exec, Run, RunTemplate};
use anyhow::Result;
use std::collections::HashMap;

/// Callback for interactive binding resolution
pub type InteractiveCallback = Box<dyn Fn(&str, Option<&serde_json::Value>) -> Result<String>>;

/// Resolver for template bindings
pub struct BindingResolver {
    /// Provided bindings (key=value from CLI)
    provided: HashMap<String, String>,
    /// Callback for interactive prompts
    interactive_callback: Option<InteractiveCallback>,
}

impl BindingResolver {
    /// Create a new resolver with provided bindings
    pub fn new() -> Self {
        Self {
            provided: HashMap::new(),
            interactive_callback: None,
        }
    }

    /// Add provided bindings from key=value strings
    pub fn with_bindings(mut self, bindings: Vec<String>) -> Self {
        for binding in bindings {
            if let Some((key, value)) = binding.split_once('=') {
                self.provided.insert(key.to_string(), value.to_string());
            }
        }
        self
    }

    /// Set the interactive callback
    pub fn with_interactive(mut self, callback: InteractiveCallback) -> Self {
        self.interactive_callback = Some(callback);
        self
    }

    /// Resolve all bindings for a template
    pub fn resolve(&self, template: &RunTemplate) -> Result<HashMap<String, String>> {
        let variables = template.extract_variables();
        let mut resolved = HashMap::new();

        let bindings = template.bindings.as_ref();
        let defaults = bindings.map(|b| &b.defaults);
        let empty_vec = Vec::new();
        let interactive = bindings.map(|b| &b.interactive).unwrap_or(&empty_vec);

        for var in variables {
            // Priority: provided > interactive > defaults
            if let Some(value) = self.provided.get(&var) {
                resolved.insert(var, value.clone());
            } else if interactive.contains(&var) {
                // Interactive prompt
                let default = defaults.and_then(|d| d.get(&var));
                let value = self.prompt_interactive(&var, default)?;
                resolved.insert(var, value);
            } else if let Some(defaults) = defaults {
                if let Some(default) = defaults.get(&var) {
                    resolved.insert(var, json_value_to_string(default));
                } else {
                    anyhow::bail!("Missing binding for variable: {}", var);
                }
            } else {
                anyhow::bail!("Missing binding for variable: {}", var);
            }
        }

        Ok(resolved)
    }

    /// Prompt for interactive value
    fn prompt_interactive(&self, var: &str, default: Option<&serde_json::Value>) -> Result<String> {
        if let Some(callback) = &self.interactive_callback {
            callback(var, default)
        } else if let Some(default) = default {
            // Non-interactive mode with default
            Ok(json_value_to_string(default))
        } else {
            anyhow::bail!(
                "Interactive variable '{}' requires a value (use --binding {}=VALUE)",
                var,
                var
            )
        }
    }

    /// Build a Run from a template with resolved bindings
    pub fn build_run(
        &self,
        template: &RunTemplate,
        code_state: CodeState,
    ) -> Result<Run> {
        let bindings = self.resolve(template)?;

        // Resolve argv
        let argv: Vec<String> = template
            .exec
            .argv
            .iter()
            .map(|arg| substitute_variables(arg, &bindings))
            .collect();

        // Resolve env
        let env: HashMap<String, String> = template
            .exec
            .env
            .iter()
            .map(|(k, v)| (k.clone(), substitute_variables(v, &bindings)))
            .collect();

        let exec = Exec {
            argv,
            cwd: template.exec.cwd.clone(),
            env,
            timeout_sec: template.exec.timeout_sec,
        };

        Ok(Run::new(exec, code_state))
    }
}

impl Default for BindingResolver {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert JSON value to string
fn json_value_to_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        _ => value.to_string(),
    }
}

/// Substitute variables in a string
fn substitute_variables(s: &str, bindings: &HashMap<String, String>) -> String {
    let mut result = s.to_string();
    for (key, value) in bindings {
        result = result.replace(&format!("{{{}}}", key), value);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Bindings, TemplateCodeState, TemplateExec};

    #[test]
    fn test_substitute_variables() {
        let mut bindings = HashMap::new();
        bindings.insert("i".to_string(), "42".to_string());
        bindings.insert("name".to_string(), "test".to_string());

        assert_eq!(
            substitute_variables("--index={i}", &bindings),
            "--index=42"
        );
        assert_eq!(
            substitute_variables("{name}_{i}", &bindings),
            "test_42"
        );
    }

    #[test]
    fn test_resolve_with_defaults() {
        let template = RunTemplate {
            template_version: 0,
            template_id: "tpl_test".to_string(),
            name: "Test".to_string(),
            exec: TemplateExec {
                argv: vec!["echo".to_string(), "{msg}".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
                timeout_sec: 0,
            },
            bindings: Some(Bindings {
                defaults: HashMap::from([("msg".to_string(), serde_json::json!("hello"))]),
                interactive: vec![],
            }),
            code_state: TemplateCodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
            },
        };

        let resolver = BindingResolver::new();
        let resolved = resolver.resolve(&template).unwrap();

        assert_eq!(resolved.get("msg").unwrap(), "hello");
    }

    #[test]
    fn test_resolve_with_provided() {
        let template = RunTemplate {
            template_version: 0,
            template_id: "tpl_test".to_string(),
            name: "Test".to_string(),
            exec: TemplateExec {
                argv: vec!["echo".to_string(), "{msg}".to_string()],
                cwd: ".".to_string(),
                env: HashMap::new(),
                timeout_sec: 0,
            },
            bindings: Some(Bindings {
                defaults: HashMap::from([("msg".to_string(), serde_json::json!("hello"))]),
                interactive: vec![],
            }),
            code_state: TemplateCodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
            },
        };

        let resolver = BindingResolver::new().with_bindings(vec!["msg=world".to_string()]);
        let resolved = resolver.resolve(&template).unwrap();

        assert_eq!(resolved.get("msg").unwrap(), "world");
    }
}
