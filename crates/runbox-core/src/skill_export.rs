//! Skill export functionality
//!
//! Generates platform-specific installation guides and exports skills
//! to a portable directory structure.

use crate::skill::{format_skill_file, Platform, Skill, SkillError, ExportResult};
use std::fs;
use std::path::Path;

impl Skill {
    /// Export the skill to a directory with platform-specific install guides
    pub fn export(&self, output_dir: &Path) -> Result<ExportResult, SkillError> {
        // Create output directory
        fs::create_dir_all(output_dir)
            .map_err(|e| SkillError::WriteError(output_dir.to_path_buf(), e))?;

        // Write SKILL.md
        let skill_path = output_dir.join("SKILL.md");
        let skill_content = format_skill_file(&self.metadata, &self.content);
        fs::write(&skill_path, &skill_content)
            .map_err(|e| SkillError::WriteError(skill_path.clone(), e))?;

        // Copy references
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

        // Copy examples
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

        // Generate install guides
        let install_dir = output_dir.join("install");
        fs::create_dir_all(&install_dir)
            .map_err(|e| SkillError::WriteError(install_dir.clone(), e))?;

        for platform in Platform::all() {
            let guide = generate_install_guide(&self.metadata.name, platform);
            let guide_path = install_dir.join(format!("{}.md", platform.slug()));
            fs::write(&guide_path, guide)
                .map_err(|e| SkillError::WriteError(guide_path.clone(), e))?;
        }

        // Generate unified INSTALL.md
        let install_md = generate_unified_install(&self.metadata.name);
        let install_path = output_dir.join("INSTALL.md");
        fs::write(&install_path, install_md)
            .map_err(|e| SkillError::WriteError(install_path.clone(), e))?;

        // Generate install.sh
        let install_sh = generate_install_script(&self.metadata.name);
        let script_path = output_dir.join("install.sh");
        fs::write(&script_path, install_sh)
            .map_err(|e| SkillError::WriteError(script_path.clone(), e))?;

        // Make install.sh executable on Unix
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

/// Generate a platform-specific installation guide
fn generate_install_guide(skill_name: &str, platform: &Platform) -> String {
    match platform {
        Platform::ClaudeCode => format!(
            r#"# Installing {} for Claude Code

## Location

Skills for Claude Code are stored in:
```
~/.claude/skills/{}/
```

## Installation Steps

1. **Create the skill directory:**
   ```bash
   mkdir -p ~/.claude/skills/{}
   ```

2. **Copy the skill files:**
   ```bash
   cp SKILL.md ~/.claude/skills/{}/
   cp -r references/ ~/.claude/skills/{}/ 2>/dev/null || true
   cp -r examples/ ~/.claude/skills/{}/ 2>/dev/null || true
   ```

3. **Verify installation:**
   ```bash
   ls -la ~/.claude/skills/{}/
   ```

## Skill Format

Claude Code skills use YAML frontmatter in a SKILL.md file:

```markdown
---
name: {}
description: When to use this skill
version: 1.0.0
---

# Skill content here
```

## Usage

Once installed, Claude Code will automatically load the skill based on the
trigger phrases in the description field.
"#,
            skill_name, skill_name, skill_name, skill_name, skill_name, skill_name, skill_name, skill_name
        ),

        Platform::OpenCode => format!(
            r#"# Installing {} for OpenCode

## Location

Skills for OpenCode are stored in:
```
~/.opencode/skills/{}/
```

## Installation Steps

1. **Create the skill directory:**
   ```bash
   mkdir -p ~/.opencode/skills/{}
   ```

2. **Copy the skill files:**
   ```bash
   cp SKILL.md ~/.opencode/skills/{}/
   cp -r references/ ~/.opencode/skills/{}/ 2>/dev/null || true
   cp -r examples/ ~/.opencode/skills/{}/ 2>/dev/null || true
   ```

3. **Verify installation:**
   ```bash
   ls -la ~/.opencode/skills/{}/
   ```

## Skill Format

OpenCode uses the same format as Claude Code - YAML frontmatter in SKILL.md:

```markdown
---
name: {}
description: When to use this skill
version: 1.0.0
---

# Skill content here
```

## Usage

OpenCode will automatically detect and load installed skills.
"#,
            skill_name, skill_name, skill_name, skill_name, skill_name, skill_name, skill_name, skill_name
        ),

        Platform::GeminiCli => format!(
            r#"# Installing {} for Gemini CLI

## Status

Gemini CLI custom instruction support is still evolving. This guide will be
updated as the platform matures.

## Current Options

### Option 1: GEMINI.md File (Project-level)

Create a `GEMINI.md` file in your project root:

```bash
cp SKILL.md ./GEMINI.md
```

### Option 2: System Instructions

Gemini CLI may support system instructions via configuration. Check the
latest Gemini CLI documentation for the current approach.

## Manual Usage

For now, you can reference the skill content manually when starting a
Gemini CLI session or include it in your project's context.

## Resources

- [Gemini CLI Documentation](https://github.com/google-gemini/gemini-cli)
"#,
            skill_name
        ),

        Platform::Codex => format!(
            r#"# Installing {} for OpenAI Codex CLI

## Status

The OpenAI Codex CLI instruction format is still being documented. This
guide will be updated as more information becomes available.

## Current Options

### Option 1: AGENTS.md File

Some OpenAI tools support an AGENTS.md file for custom instructions:

```bash
cp SKILL.md ./AGENTS.md
```

### Option 2: Configuration File

Check if your Codex CLI version supports a configuration file with
custom instructions.

## Manual Usage

You can include the skill content in your prompts or session initialization.

## Resources

- [OpenAI Codex Documentation](https://platform.openai.com/docs)
"#,
            skill_name
        ),

        Platform::Cursor => format!(
            r#"# Installing {} for Cursor

## Location

Cursor rules can be stored in two locations:

1. **Global rules:** `~/.cursor/rules/`
2. **Project rules:** `.cursor/rules/` (in your project directory)

## Installation Steps

### Global Installation

1. **Create the rules directory:**
   ```bash
   mkdir -p ~/.cursor/rules
   ```

2. **Copy the skill as a rule:**
   ```bash
   cp SKILL.md ~/.cursor/rules/{}.md
   ```

### Project-level Installation

1. **Create the project rules directory:**
   ```bash
   mkdir -p .cursor/rules
   ```

2. **Copy the skill:**
   ```bash
   cp SKILL.md .cursor/rules/{}.md
   ```

## Cursor Rules Format

Cursor uses markdown files without specific frontmatter requirements.
The skill content will work directly, though you may want to remove
the YAML frontmatter:

```bash
# Remove frontmatter and save
sed '1,/^---$/d' SKILL.md | sed '1,/^---$/d' > ~/.cursor/rules/{}.md
```

## Alternative: .cursorrules File

For simpler cases, you can append to a `.cursorrules` file in your project:

```bash
cat SKILL.md >> .cursorrules
```

## Usage

Cursor will automatically include rules from the rules directory
in its context.
"#,
            skill_name, skill_name, skill_name, skill_name
        ),
    }
}

/// Generate a unified installation guide
fn generate_unified_install(skill_name: &str) -> String {
    format!(
        r#"# Installing {}

This skill can be installed into multiple AI coding assistants. Choose your platform below.

## Quick Install (Auto-detect)

Run the install script to automatically detect your installed tools and install the skill:

```bash
./install.sh
```

## Platform-Specific Guides

| Platform | Guide | Location |
|----------|-------|----------|
| Claude Code | [install/claude-code.md](install/claude-code.md) | `~/.claude/skills/{}/` |
| OpenCode | [install/opencode.md](install/opencode.md) | `~/.opencode/skills/{}/` |
| Gemini CLI | [install/gemini.md](install/gemini.md) | Project-level GEMINI.md |
| Codex | [install/codex.md](install/codex.md) | AGENTS.md or configuration |
| Cursor | [install/cursor.md](install/cursor.md) | `~/.cursor/rules/` or `.cursor/rules/` |

## Manual Installation

For any platform, the core installation is:

1. Copy `SKILL.md` to the platform's skill directory
2. Copy the `references/` directory if it exists
3. Copy the `examples/` directory if it exists

## Skill Contents

- `SKILL.md` - Main skill file with instructions
- `references/` - Reference documentation and schemas
- `examples/` - Example configurations and usage patterns
- `install/` - Platform-specific installation guides

## Verification

After installation, verify by asking the AI assistant about the skill's topic.
The assistant should reference the skill content in its response.
"#,
        skill_name, skill_name, skill_name
    )
}

/// Generate an install script
fn generate_install_script(skill_name: &str) -> String {
    format!(
        r##"#!/usr/bin/env bash
# Install script for {} skill
# Auto-detects installed AI coding assistants and installs the skill

set -e

SKILL_NAME="{}"
SCRIPT_DIR="$(cd "$(dirname "${{BASH_SOURCE[0]}}")" && pwd)"

echo "Installing skill: $SKILL_NAME"
echo ""

# Colors for output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

installed=0

# Claude Code
CLAUDE_SKILLS_DIR="$HOME/.claude/skills"
if [ -d "$HOME/.claude" ] || command -v claude &> /dev/null; then
    echo -e "${{GREEN}}Found Claude Code${{NC}}"
    mkdir -p "$CLAUDE_SKILLS_DIR/$SKILL_NAME"
    cp "$SCRIPT_DIR/SKILL.md" "$CLAUDE_SKILLS_DIR/$SKILL_NAME/"
    [ -d "$SCRIPT_DIR/references" ] && cp -r "$SCRIPT_DIR/references" "$CLAUDE_SKILLS_DIR/$SKILL_NAME/" 2>/dev/null || true
    [ -d "$SCRIPT_DIR/examples" ] && cp -r "$SCRIPT_DIR/examples" "$CLAUDE_SKILLS_DIR/$SKILL_NAME/" 2>/dev/null || true
    echo "  Installed to: $CLAUDE_SKILLS_DIR/$SKILL_NAME/"
    installed=$((installed + 1))
fi

# OpenCode
OPENCODE_SKILLS_DIR="$HOME/.opencode/skills"
if [ -d "$HOME/.opencode" ] || [ -d "$HOME/.config/opencode" ] || command -v opencode &> /dev/null; then
    echo -e "${{GREEN}}Found OpenCode${{NC}}"
    mkdir -p "$OPENCODE_SKILLS_DIR/$SKILL_NAME"
    cp "$SCRIPT_DIR/SKILL.md" "$OPENCODE_SKILLS_DIR/$SKILL_NAME/"
    [ -d "$SCRIPT_DIR/references" ] && cp -r "$SCRIPT_DIR/references" "$OPENCODE_SKILLS_DIR/$SKILL_NAME/" 2>/dev/null || true
    [ -d "$SCRIPT_DIR/examples" ] && cp -r "$SCRIPT_DIR/examples" "$OPENCODE_SKILLS_DIR/$SKILL_NAME/" 2>/dev/null || true
    echo "  Installed to: $OPENCODE_SKILLS_DIR/$SKILL_NAME/"
    installed=$((installed + 1))
fi

# Cursor
CURSOR_RULES_DIR="$HOME/.cursor/rules"
if [ -d "$HOME/.cursor" ] || [ -d "/Applications/Cursor.app" ]; then
    echo -e "${{GREEN}}Found Cursor${{NC}}"
    mkdir -p "$CURSOR_RULES_DIR"
    cp "$SCRIPT_DIR/SKILL.md" "$CURSOR_RULES_DIR/$SKILL_NAME.md"
    echo "  Installed to: $CURSOR_RULES_DIR/$SKILL_NAME.md"
    installed=$((installed + 1))
fi

echo ""
if [ $installed -eq 0 ]; then
    echo -e "${{YELLOW}}No supported AI coding assistants found.${{NC}}"
    echo "See INSTALL.md for manual installation instructions."
    exit 1
else
    echo -e "${{GREEN}}Successfully installed to $installed platform(s)${{NC}}"
fi
"##,
        skill_name, skill_name
    )
}
