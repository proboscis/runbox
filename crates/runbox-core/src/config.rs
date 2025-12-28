//! Configuration module with layered configuration resolution
//!
//! Priority order (highest to lowest):
//! 1. CLI flags
//! 2. Git config (local)
//! 3. Global config file (~/.config/runbox/config.toml)
//! 4. Default values

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;

/// Replay-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayConfig {
    /// Directory for worktrees
    #[serde(default)]
    pub worktree_dir: Option<PathBuf>,
    /// Whether to cleanup worktrees after execution
    #[serde(default)]
    pub cleanup: bool,
    /// Whether to reuse existing worktrees if commit matches
    #[serde(default = "default_reuse")]
    pub reuse: bool,
}

impl Default for ReplayConfig {
    fn default() -> Self {
        Self {
            worktree_dir: None,
            cleanup: false,
            reuse: true, // Default is to reuse worktrees
        }
    }
}

fn default_reuse() -> bool {
    true
}

/// Logging configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LoggingConfig {
    /// Verbosity level (0=normal, 1=-v, 2=-vv, 3=-vvv)
    #[serde(default)]
    pub verbosity: u8,
}

/// Global runbox configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RunboxConfig {
    #[serde(default)]
    pub replay: ReplayConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
}

impl RunboxConfig {
    /// Load global config from ~/.config/runbox/config.toml
    pub fn load_global() -> Result<Self> {
        let config_path = Self::global_config_path();
        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)
                .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;
            let config: RunboxConfig = toml::from_str(&content)
                .with_context(|| format!("Failed to parse config file: {}", config_path.display()))?;
            Ok(config)
        } else {
            Ok(Self::default())
        }
    }

    /// Get the path to the global config file
    pub fn global_config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("runbox")
            .join("config.toml")
    }
}

/// Source of a configuration value
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    CliFlag,
    GitConfig,
    GlobalConfig,
    Default,
}

impl std::fmt::Display for ConfigSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigSource::CliFlag => write!(f, "CLI flag"),
            ConfigSource::GitConfig => write!(f, "git config"),
            ConfigSource::GlobalConfig => write!(f, "global config"),
            ConfigSource::Default => write!(f, "default"),
        }
    }
}

/// Result of configuration resolution with source tracking
#[derive(Debug, Clone)]
pub struct ResolvedValue<T> {
    pub value: T,
    pub source: ConfigSource,
}

impl<T> ResolvedValue<T> {
    pub fn new(value: T, source: ConfigSource) -> Self {
        Self { value, source }
    }
}

/// Configuration resolver with layered priority
pub struct ConfigResolver {
    repo_root: Option<PathBuf>,
    global_config: RunboxConfig,
}

impl ConfigResolver {
    /// Create a new resolver for a repository
    pub fn new(repo_root: Option<PathBuf>) -> Result<Self> {
        let global_config = RunboxConfig::load_global().unwrap_or_default();
        Ok(Self {
            repo_root,
            global_config,
        })
    }

    /// Get a git config value
    fn get_git_config(&self, key: &str) -> Option<String> {
        let repo_root = self.repo_root.as_ref()?;
        let output = Command::new("git")
            .current_dir(repo_root)
            .args(["config", "--get", key])
            .output()
            .ok()?;

        if output.status.success() {
            Some(String::from_utf8(output.stdout).ok()?.trim().to_string())
        } else {
            None
        }
    }

    /// Resolve worktree directory with layered configuration
    pub fn resolve_worktree_dir(&self, cli_value: Option<&PathBuf>) -> ResolvedValue<PathBuf> {
        // 1. CLI flag (highest priority)
        if let Some(dir) = cli_value {
            return ResolvedValue::new(dir.clone(), ConfigSource::CliFlag);
        }

        // 2. Git config (local)
        if let Some(dir) = self.get_git_config("runbox.worktreeDir") {
            return ResolvedValue::new(PathBuf::from(dir), ConfigSource::GitConfig);
        }

        // 3. Global config
        if let Some(dir) = &self.global_config.replay.worktree_dir {
            // Expand ~ if present
            let expanded = if dir.starts_with("~") {
                dirs::home_dir()
                    .map(|home| home.join(dir.strip_prefix("~").unwrap_or(dir)))
                    .unwrap_or_else(|| dir.clone())
            } else {
                dir.clone()
            };
            return ResolvedValue::new(expanded, ConfigSource::GlobalConfig);
        }

        // 4. Default
        let default_dir = self
            .repo_root
            .as_ref()
            .map(|r| r.join(".git-worktrees").join("replay"))
            .unwrap_or_else(|| PathBuf::from(".git-worktrees/replay"));
        ResolvedValue::new(default_dir, ConfigSource::Default)
    }

    /// Resolve cleanup setting
    pub fn resolve_cleanup(&self, cli_cleanup: Option<bool>) -> ResolvedValue<bool> {
        // 1. CLI flag
        if let Some(cleanup) = cli_cleanup {
            return ResolvedValue::new(cleanup, ConfigSource::CliFlag);
        }

        // 2. Git config
        if let Some(cleanup) = self.get_git_config("runbox.worktreeCleanup") {
            if let Ok(value) = cleanup.parse::<bool>() {
                return ResolvedValue::new(value, ConfigSource::GitConfig);
            }
        }

        // 3. Global config
        if self.global_config.replay.cleanup {
            return ResolvedValue::new(true, ConfigSource::GlobalConfig);
        }

        // 4. Default (false = keep worktrees)
        ResolvedValue::new(false, ConfigSource::Default)
    }

    /// Resolve reuse setting
    pub fn resolve_reuse(&self, cli_reuse: Option<bool>) -> ResolvedValue<bool> {
        // 1. CLI flag
        if let Some(reuse) = cli_reuse {
            return ResolvedValue::new(reuse, ConfigSource::CliFlag);
        }

        // 2. Git config
        if let Some(reuse) = self.get_git_config("runbox.worktreeReuse") {
            if let Ok(value) = reuse.parse::<bool>() {
                return ResolvedValue::new(value, ConfigSource::GitConfig);
            }
        }

        // 3. Global config (has default true)
        return ResolvedValue::new(self.global_config.replay.reuse, ConfigSource::GlobalConfig);
    }

    /// Resolve verbosity level
    pub fn resolve_verbosity(&self, cli_verbosity: u8) -> ResolvedValue<u8> {
        // CLI always wins for verbosity since 0 is also a valid value
        if cli_verbosity > 0 {
            return ResolvedValue::new(cli_verbosity, ConfigSource::CliFlag);
        }

        // Git config
        if let Some(verbosity) = self.get_git_config("runbox.verbosity") {
            if let Ok(value) = verbosity.parse::<u8>() {
                return ResolvedValue::new(value, ConfigSource::GitConfig);
            }
        }

        // Global config
        if self.global_config.logging.verbosity > 0 {
            return ResolvedValue::new(
                self.global_config.logging.verbosity,
                ConfigSource::GlobalConfig,
            );
        }

        // Default
        ResolvedValue::new(0, ConfigSource::Default)
    }
}

/// Verbose logger that respects verbosity levels
#[derive(Debug, Clone)]
pub struct VerboseLogger {
    verbosity: u8,
}

impl VerboseLogger {
    pub fn new(verbosity: u8) -> Self {
        Self { verbosity }
    }

    /// Log at verbosity level 1 (-v)
    pub fn log_v(&self, category: &str, message: &str) {
        if self.verbosity >= 1 {
            eprintln!("[{}] {}", category, message);
        }
    }

    /// Log at verbosity level 2 (-vv)
    pub fn log_vv(&self, category: &str, message: &str) {
        if self.verbosity >= 2 {
            eprintln!("[{}] {}", category, message);
        }
    }

    /// Log at verbosity level 3 (-vvv)
    pub fn log_vvv(&self, category: &str, message: &str) {
        if self.verbosity >= 3 {
            eprintln!("[{}] {}", category, message);
        }
    }

    /// Get current verbosity level
    pub fn verbosity(&self) -> u8 {
        self.verbosity
    }

    /// Log configuration resolution at appropriate level
    pub fn log_config_resolution<T: std::fmt::Display>(
        &self,
        name: &str,
        resolved: &ResolvedValue<T>,
    ) {
        self.log_v("config", &format!("{}: {} (from: {})", name, resolved.value, resolved.source));
    }

    /// Log all config layers checked (at -vv level)
    pub fn log_config_layers(&self, name: &str, cli: Option<&str>, git: Option<&str>, global: Option<&str>, used: &str, source: ConfigSource) {
        self.log_vv("config", &format!("checking CLI flag --{}: {}", name, cli.unwrap_or("not set")));
        self.log_vv("config", &format!("checking git config runbox.{}: {}", name, git.unwrap_or("not set")));
        self.log_vv("config", &format!("checking global config: {}", global.unwrap_or("not set")));
        self.log_vv("config", &format!("→ using: {} (source: {})", used, source));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = RunboxConfig::default();
        assert!(config.replay.worktree_dir.is_none());
        assert!(!config.replay.cleanup);
        assert!(config.replay.reuse);
        assert_eq!(config.logging.verbosity, 0);
    }

    #[test]
    fn test_config_source_display() {
        assert_eq!(format!("{}", ConfigSource::CliFlag), "CLI flag");
        assert_eq!(format!("{}", ConfigSource::GitConfig), "git config");
        assert_eq!(format!("{}", ConfigSource::GlobalConfig), "global config");
        assert_eq!(format!("{}", ConfigSource::Default), "default");
    }

    #[test]
    fn test_parse_toml_config() {
        let toml_content = r#"
[replay]
worktree_dir = "~/.runbox/worktrees"
cleanup = false
reuse = true

[logging]
verbosity = 1
"#;
        let config: RunboxConfig = toml::from_str(toml_content).unwrap();
        assert_eq!(
            config.replay.worktree_dir,
            Some(PathBuf::from("~/.runbox/worktrees"))
        );
        assert!(!config.replay.cleanup);
        assert!(config.replay.reuse);
        assert_eq!(config.logging.verbosity, 1);
    }
}
