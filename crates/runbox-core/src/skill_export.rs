//! Skill export functionality - generates platform-specific installation guides

use crate::skill::{format_skill_file, ExportResult, Platform, Skill, SkillError};
use std::fs;
use std::path::Path;

impl Skill {
    pub fn export(&self, output_dir: &Path) -> Result<ExportResult, SkillError> {
        fs::create_dir_all(output_dir)
            .map_err(|e| SkillError::WriteError(output_dir.to_path_buf(), e))?;

        let skill_path = output_dir.join("SKILL.md");
        let skill_content = format_skill_file(&self.metadata, &self.content);
        fs::write(&skill_path, &skill_content)
            .map_err(|e| SkillError::WriteError(skill_path.clone(), e))?;

        if !self.references.is_empty() {
            let refs_dir = output_dir.join("references");
            fs::create_dir_all(&refs_dir)
                .map_err(|e| SkillError::WriteError(refs_dir.clone(), e))?;

            for ref_path in &self.references {
                let src = self.path.join("references").join(ref_path);
                let dst = refs_dir.join(ref_path);
                if let Some(parent) = dst.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|e| SkillError::WriteError(parent.to_path_buf(), e))?;
                }
                if src.exists() {
                    fs::copy(&src, &dst)
                        .map_err(|e| SkillError::CopyError(src.clone(), dst.clone(), e))?;
                }
            }
        }

        if !self.examples.is_empty() {
            let examples_dir = output_dir.join("examples");
            fs::create_dir_all(&examples_dir)
                .map_err(|e| SkillError::WriteError(examples_dir.clone(), e))?;

            for ex_path in &self.examples {
                let src = self.path.join("examples").join(ex_path);
                let dst = examples_dir.join(ex_path);
                if let Some(parent) = dst.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|e| SkillError::WriteError(parent.to_path_buf(), e))?;
                }
                if src.exists() {
                    fs::copy(&src, &dst)
                        .map_err(|e| SkillError::CopyError(src.clone(), dst.clone(), e))?;
                }
            }
        }

        let install_dir = output_dir.join("install");
        fs::create_dir_all(&install_dir)
            .map_err(|e| SkillError::WriteError(install_dir.clone(), e))?;

        for platform in Platform::all() {
            let guide = generate_install_guide(&self.metadata.name, platform);
            let guide_path = install_dir.join(format!("{}.md", platform.slug()));
            fs::write(&guide_path, guide)
                .map_err(|e| SkillError::WriteError(guide_path.clone(), e))?;
        }

        let install_md = generate_unified_install(&self.metadata.name);
        let install_path = output_dir.join("INSTALL.md");
        fs::write(&install_path, install_md)
            .map_err(|e| SkillError::WriteError(install_path.clone(), e))?;

        let install_sh = generate_install_script(&self.metadata.name);
        let script_path = output_dir.join("install.sh");
        fs::write(&script_path, install_sh)
            .map_err(|e| SkillError::WriteError(script_path.clone(), e))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&script_path)
                .map_err(|e| SkillError::WriteError(script_path.clone(), e))?
                .permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script_path, perms)
                .map_err(|e| SkillError::WriteError(script_path.clone(), e))?;
        }

        Ok(ExportResult {
            output_dir: output_dir.to_path_buf(),
            skill_file: skill_path,
            references_count: self.references.len(),
            examples_count: self.examples.len(),
        })
    }
}

fn generate_install_guide(skill_name: &str, platform: &Platform) -> String {
    match platform {
        Platform::ClaudeCode => format!(
            r#"# Installing {skill_name} for Claude Code

## Location

Skills for Claude Code are stored in:
```
~/.claude/skills/{skill_name}/
```

## Installation

```bash
mkdir -p ~/.claude/skills/{skill_name}
cp SKILL.md ~/.claude/skills/{skill_name}/
cp -r references/ ~/.claude/skills/{skill_name}/ 2>/dev/null || true
cp -r examples/ ~/.claude/skills/{skill_name}/ 2>/dev/null || true
```

## Verification

```bash
ls -la ~/.claude/skills/{skill_name}/
```

Claude Code will automatically load the skill based on trigger phrases in the description.
"#
        ),

        Platform::OpenCode => format!(
            r#"# Installing {skill_name} for OpenCode

## Location

Skills for OpenCode are stored in:
```
~/.opencode/skills/{skill_name}/
```

## Installation

```bash
mkdir -p ~/.opencode/skills/{skill_name}
cp SKILL.md ~/.opencode/skills/{skill_name}/
cp -r references/ ~/.opencode/skills/{skill_name}/ 2>/dev/null || true
cp -r examples/ ~/.opencode/skills/{skill_name}/ 2>/dev/null || true
```

## Verification

```bash
ls -la ~/.opencode/skills/{skill_name}/
```

OpenCode will automatically detect and load installed skills.
"#
        ),

        Platform::GeminiCli => format!(
            r#"# Installing {skill_name} for Gemini CLI

## Project-level Installation

Create a `GEMINI.md` file in your project root:

```bash
cp SKILL.md ./GEMINI.md
```

## Notes

Gemini CLI custom instruction support is still evolving.
Check the latest documentation for current approaches.
"#
        ),

        Platform::Codex => format!(
            r#"# Installing {skill_name} for OpenAI Codex CLI

## Project-level Installation

Create an `AGENTS.md` file:

```bash
cp SKILL.md ./AGENTS.md
```

## Notes

Check if your Codex CLI version supports custom instructions.
"#
        ),

        Platform::Cursor => format!(
            r#"# Installing {skill_name} for Cursor

## Global Installation

```bash
mkdir -p ~/.cursor/rules
cp SKILL.md ~/.cursor/rules/{skill_name}.md
```

## Project-level Installation

```bash
mkdir -p .cursor/rules
cp SKILL.md .cursor/rules/{skill_name}.md
```

## Alternative: .cursorrules

```bash
cat SKILL.md >> .cursorrules
```

Cursor automatically includes rules from the rules directory.
"#
        ),
    }
}

fn generate_unified_install(skill_name: &str) -> String {
    format!(
        r#"# Installing {skill_name}

This skill can be installed into multiple AI coding assistants.

## Quick Install

Run the auto-installer:

```bash
./install.sh
```

## Platform-Specific Guides

| Platform | Guide | Location |
|----------|-------|----------|
| Claude Code | [install/claude-code.md](install/claude-code.md) | `~/.claude/skills/{skill_name}/` |
| OpenCode | [install/opencode.md](install/opencode.md) | `~/.opencode/skills/{skill_name}/` |
| Gemini CLI | [install/gemini.md](install/gemini.md) | Project-level GEMINI.md |
| Codex | [install/codex.md](install/codex.md) | AGENTS.md |
| Cursor | [install/cursor.md](install/cursor.md) | `~/.cursor/rules/` |

## Contents

- `SKILL.md` - Main skill file
- `references/` - Reference documentation (if any)
- `examples/` - Example files (if any)
- `install/` - Platform-specific guides
- `install.sh` - Auto-install script
"#
    )
}

fn generate_install_script(skill_name: &str) -> String {
    format!(
        r##"#!/usr/bin/env bash
# Auto-install script for {skill_name} skill

set -e

SKILL_NAME="{skill_name}"
SCRIPT_DIR="$(cd "$(dirname "${{BASH_SOURCE[0]}}")" && pwd)"

echo "Installing skill: $SKILL_NAME"
echo ""

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

installed=0

# Claude Code
if [ -d "$HOME/.claude" ] || command -v claude &> /dev/null; then
    echo -e "${{GREEN}}Found Claude Code${{NC}}"
    mkdir -p "$HOME/.claude/skills/$SKILL_NAME"
    cp "$SCRIPT_DIR/SKILL.md" "$HOME/.claude/skills/$SKILL_NAME/"
    [ -d "$SCRIPT_DIR/references" ] && cp -r "$SCRIPT_DIR/references" "$HOME/.claude/skills/$SKILL_NAME/" 2>/dev/null || true
    [ -d "$SCRIPT_DIR/examples" ] && cp -r "$SCRIPT_DIR/examples" "$HOME/.claude/skills/$SKILL_NAME/" 2>/dev/null || true
    echo "  Installed to: ~/.claude/skills/$SKILL_NAME/"
    installed=$((installed + 1))
fi

# OpenCode
if [ -d "$HOME/.opencode" ] || command -v opencode &> /dev/null; then
    echo -e "${{GREEN}}Found OpenCode${{NC}}"
    mkdir -p "$HOME/.opencode/skills/$SKILL_NAME"
    cp "$SCRIPT_DIR/SKILL.md" "$HOME/.opencode/skills/$SKILL_NAME/"
    [ -d "$SCRIPT_DIR/references" ] && cp -r "$SCRIPT_DIR/references" "$HOME/.opencode/skills/$SKILL_NAME/" 2>/dev/null || true
    [ -d "$SCRIPT_DIR/examples" ] && cp -r "$SCRIPT_DIR/examples" "$HOME/.opencode/skills/$SKILL_NAME/" 2>/dev/null || true
    echo "  Installed to: ~/.opencode/skills/$SKILL_NAME/"
    installed=$((installed + 1))
fi

# Cursor
if [ -d "$HOME/.cursor" ] || [ -d "/Applications/Cursor.app" ]; then
    echo -e "${{GREEN}}Found Cursor${{NC}}"
    mkdir -p "$HOME/.cursor/rules"
    cp "$SCRIPT_DIR/SKILL.md" "$HOME/.cursor/rules/$SKILL_NAME.md"
    echo "  Installed to: ~/.cursor/rules/$SKILL_NAME.md"
    installed=$((installed + 1))
fi

echo ""
if [ $installed -eq 0 ]; then
    echo -e "${{YELLOW}}No supported AI coding assistants found.${{NC}}"
    echo "See INSTALL.md for manual installation."
    exit 1
else
    echo -e "${{GREEN}}Installed to $installed platform(s)${{NC}}"
fi
"##
    )
}
