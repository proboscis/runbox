//! Skill management for AI coding assistants
//!
//! Skills are reusable instruction sets that teach AI assistants how to use
//! specific tools or follow specific patterns. This module handles loading
//! skills from various AI assistant platforms and exporting them in a
//! portable format.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Platform-specific skill storage locations and formats
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// Claude Code (~/.claude/skills/)
    ClaudeCode,
    /// OpenCode (~/.opencode/skills/)
    OpenCode,
    /// Gemini CLI
    GeminiCli,
    /// OpenAI Codex CLI
    Codex,
    /// Cursor IDE
    Cursor,
}

impl Platform {
    /// Get the default skill directory for this platform
    pub fn skill_dir(&self) -> Option<PathBuf> {
        let home = dirs::home_dir()?;
        match self {
            Platform::ClaudeCode => Some(home.join(".claude").join("skills")),
            Platform::OpenCode => Some(home.join(".opencode").join("skills")),
            Platform::GeminiCli => None, // Project-level GEMINI.md
            Platform::Codex => None,     // AGENTS.md
            Platform::Cursor => Some(home.join(".cursor").join("rules")),
        }
    }

    /// Get the name of this platform
    pub fn name(&self) -> &'static str {
        match self {
            Platform::ClaudeCode => "Claude Code",
            Platform::OpenCode => "OpenCode",
            Platform::GeminiCli => "Gemini CLI",
            Platform::Codex => "Codex",
            Platform::Cursor => "Cursor",
        }
    }

    /// Get the slug for this platform (used in filenames)
    pub fn slug(&self) -> &'static str {
        match self {
            Platform::ClaudeCode => "claude-code",
            Platform::OpenCode => "opencode",
            Platform::GeminiCli => "gemini",
            Platform::Codex => "codex",
            Platform::Cursor => "cursor",
        }
    }

    /// All supported platforms
    pub fn all() -> &'static [Platform] {
        &[
            Platform::ClaudeCode,
            Platform::OpenCode,
            Platform::GeminiCli,
            Platform::Codex,
            Platform::Cursor,
        ]
    }
}

/// Skill metadata from YAML frontmatter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMetadata {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_yaml::Value>,
}

/// A complete skill with metadata and content
#[derive(Debug, Clone)]
pub struct Skill {
    /// Skill metadata from frontmatter
    pub metadata: SkillMetadata,
    /// Main skill content (markdown body)
    pub content: String,
    /// Path to the skill directory
    pub path: PathBuf,
    /// Reference files (relative paths within skill dir)
    pub references: Vec<PathBuf>,
    /// Example files (relative paths within skill dir)
    pub examples: Vec<PathBuf>,
}

impl Skill {
    /// Load a skill from a directory
    pub fn load(path: &Path) -> Result<Self, SkillError> {
        let skill_file = path.join("SKILL.md");
        if !skill_file.exists() {
            return Err(SkillError::NotFound(path.to_path_buf()));
        }

        let content = fs::read_to_string(&skill_file)
            .map_err(|e| SkillError::ReadError(skill_file.clone(), e))?;

        let (metadata, body) = parse_frontmatter(&content)?;

        // Find reference files
        let references = find_files_in_dir(&path.join("references"));
        let examples = find_files_in_dir(&path.join("examples"));

        Ok(Self {
            metadata,
            content: body,
            path: path.to_path_buf(),
            references,
            examples,
        })
    }

    /// Get the skill name (from metadata)
    pub fn name(&self) -> &str {
        &self.metadata.name
    }
}

/// Result of a skill export operation
#[derive(Debug)]
pub struct ExportResult {
    pub output_dir: PathBuf,
    pub skill_file: PathBuf,
    pub references_count: usize,
    pub examples_count: usize,
}

/// Skill-related errors
#[derive(Debug, Error)]
pub enum SkillError {
    #[error("Skill not found: {0}")]
    NotFound(PathBuf),

    #[error("Failed to read {0}: {1}")]
    ReadError(PathBuf, std::io::Error),

    #[error("Failed to write {0}: {1}")]
    WriteError(PathBuf, std::io::Error),

    #[error("Failed to copy {0} to {1}: {2}")]
    CopyError(PathBuf, PathBuf, std::io::Error),

    #[error("Invalid frontmatter: {0}")]
    InvalidFrontmatter(String),

    #[error("Missing frontmatter in skill file")]
    MissingFrontmatter,
}

/// Parse YAML frontmatter from a markdown file
pub fn parse_frontmatter(content: &str) -> Result<(SkillMetadata, String), SkillError> {
    let content = content.trim();

    if !content.starts_with("---") {
        return Err(SkillError::MissingFrontmatter);
    }

    // Find the closing ---
    let rest = &content[3..];
    let end_pos = rest.find("\n---").ok_or(SkillError::MissingFrontmatter)?;

    let yaml_str = &rest[..end_pos].trim();
    let body = rest[end_pos + 4..].trim();

    let metadata: SkillMetadata = serde_yaml::from_str(yaml_str)
        .map_err(|e| SkillError::InvalidFrontmatter(e.to_string()))?;

    Ok((metadata, body.to_string()))
}

/// Format a skill file with frontmatter
pub fn format_skill_file(metadata: &SkillMetadata, content: &str) -> String {
    let yaml = serde_yaml::to_string(metadata).unwrap_or_default();
    format!("---\n{}---\n\n{}", yaml, content)
}

/// Find all files in a directory (recursively)
fn find_files_in_dir(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if dir.is_dir() {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Ok(rel) = path.strip_prefix(dir) {
                        files.push(rel.to_path_buf());
                    }
                } else if path.is_dir() {
                    let subfiles = find_files_in_dir(&path);
                    for subfile in subfiles {
                        if let Ok(rel) = path.strip_prefix(dir) {
                            files.push(rel.join(&subfile));
                        }
                    }
                }
            }
        }
    }
    files.sort();
    files
}

/// Find skills across all platforms
pub fn find_skills() -> Vec<(Platform, PathBuf, String)> {
    let mut skills = Vec::new();

    for platform in Platform::all() {
        if let Some(skill_dir) = platform.skill_dir() {
            if skill_dir.exists() {
                if let Ok(entries) = fs::read_dir(&skill_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_dir() && path.join("SKILL.md").exists() {
                            let name = path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .map(|s| s.to_string());
                            if let Some(name) = name {
                                skills.push((*platform, path, name));
                            }
                        }
                    }
                }
            }
        }
    }

    skills.sort_by(|a, b| a.2.cmp(&b.2));
    skills
}

/// Find a skill by name across all platforms
pub fn find_skill_by_name(name: &str) -> Option<(Platform, PathBuf)> {
    for platform in Platform::all() {
        if let Some(skill_dir) = platform.skill_dir() {
            let skill_path = skill_dir.join(name);
            if skill_path.exists() && skill_path.join("SKILL.md").exists() {
                return Some((*platform, skill_path));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frontmatter() {
        let content = r#"---
name: test-skill
description: A test skill
version: 1.0.0
---

# Test Skill

This is the skill content."#;

        let (metadata, body) = parse_frontmatter(content).unwrap();
        assert_eq!(metadata.name, "test-skill");
        assert_eq!(metadata.description, "A test skill");
        assert_eq!(metadata.version, Some("1.0.0".to_string()));
        assert!(body.contains("# Test Skill"));
    }

    #[test]
    fn test_parse_frontmatter_missing() {
        let content = "# No frontmatter here";
        assert!(matches!(
            parse_frontmatter(content),
            Err(SkillError::MissingFrontmatter)
        ));
    }

    #[test]
    fn test_platform_slug() {
        assert_eq!(Platform::ClaudeCode.slug(), "claude-code");
        assert_eq!(Platform::OpenCode.slug(), "opencode");
        assert_eq!(Platform::Cursor.slug(), "cursor");
    }

    #[test]
    fn test_platform_name() {
        assert_eq!(Platform::ClaudeCode.name(), "Claude Code");
        assert_eq!(Platform::OpenCode.name(), "OpenCode");
        assert_eq!(Platform::Cursor.name(), "Cursor");
    }
}
