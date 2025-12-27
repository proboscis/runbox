use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Command;

use crate::{CodeState, Patch};

/// Git context for capturing repository state
pub struct GitContext {
    repo_root: std::path::PathBuf,
}

impl GitContext {
    /// Create a new GitContext from the current directory
    pub fn from_current_dir() -> Result<Self> {
        let output = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .context("Failed to run git")?;

        if !output.status.success() {
            anyhow::bail!("Not a git repository");
        }

        let repo_root = String::from_utf8(output.stdout)?
            .trim()
            .to_string()
            .into();

        Ok(Self { repo_root })
    }

    /// Create a new GitContext from a specific path
    pub fn from_path(path: &Path) -> Result<Self> {
        let output = Command::new("git")
            .current_dir(path)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .context("Failed to run git")?;

        if !output.status.success() {
            anyhow::bail!("Not a git repository");
        }

        let repo_root = String::from_utf8(output.stdout)?
            .trim()
            .to_string()
            .into();

        Ok(Self { repo_root })
    }

    /// Get the repository root
    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    /// Get the remote URL (origin)
    pub fn get_remote_url(&self) -> Result<String> {
        let output = Command::new("git")
            .current_dir(&self.repo_root)
            .args(["remote", "get-url", "origin"])
            .output()
            .context("Failed to get remote URL")?;

        if !output.status.success() {
            anyhow::bail!("No origin remote found");
        }

        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    }

    /// Get the current HEAD commit SHA
    pub fn get_head_commit(&self) -> Result<String> {
        let output = Command::new("git")
            .current_dir(&self.repo_root)
            .args(["rev-parse", "HEAD"])
            .output()
            .context("Failed to get HEAD commit")?;

        if !output.status.success() {
            anyhow::bail!("Failed to get HEAD commit");
        }

        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    }

    /// Check if there are uncommitted changes
    pub fn has_uncommitted_changes(&self) -> Result<bool> {
        let output = Command::new("git")
            .current_dir(&self.repo_root)
            .args(["status", "--porcelain"])
            .output()
            .context("Failed to check git status")?;

        if !output.status.success() {
            anyhow::bail!("Failed to check git status");
        }

        Ok(!output.stdout.is_empty())
    }

    /// Get diff of uncommitted changes (including staged)
    pub fn get_diff(&self) -> Result<String> {
        // Get both staged and unstaged changes
        let output = Command::new("git")
            .current_dir(&self.repo_root)
            .args(["diff", "HEAD"])
            .output()
            .context("Failed to get diff")?;

        if !output.status.success() {
            anyhow::bail!("Failed to get diff");
        }

        Ok(String::from_utf8(output.stdout)?)
    }

    /// Compute SHA-256 hash of content
    pub fn sha256_hash(content: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        let result = hasher.finalize();
        format!("{:x}", result)
    }

    /// Create a patch and push it to refs/patches/{run_id}
    pub fn create_and_push_patch(&self, run_id: &str) -> Result<Option<Patch>> {
        if !self.has_uncommitted_changes()? {
            return Ok(None);
        }

        let diff = self.get_diff()?;
        if diff.is_empty() {
            return Ok(None);
        }

        let sha256 = Self::sha256_hash(&diff);
        let ref_name = format!("refs/patches/{}", run_id);

        // Create a temporary commit with the current changes
        // First, stash the changes, create a commit, then restore
        // Actually, we'll use a different approach: create a blob and a ref pointing to it

        // Create a blob with the diff content
        let mut child = Command::new("git")
            .current_dir(&self.repo_root)
            .args(["hash-object", "-w", "--stdin"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .context("Failed to create blob")?;

        {
            use std::io::Write;
            let stdin = child.stdin.as_mut().context("Failed to get stdin")?;
            stdin.write_all(diff.as_bytes())?;
        }

        let output = child.wait_with_output()?;
        if !output.status.success() {
            anyhow::bail!("Failed to create blob");
        }

        let blob_sha = String::from_utf8(output.stdout)?.trim().to_string();

        // Create a ref pointing to the blob
        let output = Command::new("git")
            .current_dir(&self.repo_root)
            .args(["update-ref", &ref_name, &blob_sha])
            .output()
            .context("Failed to create ref")?;

        if !output.status.success() {
            anyhow::bail!(
                "Failed to create ref: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(Some(Patch {
            ref_: ref_name,
            sha256,
        }))
    }

    /// Push patch ref to remote
    pub fn push_patch_ref(&self, ref_name: &str) -> Result<()> {
        let output = Command::new("git")
            .current_dir(&self.repo_root)
            .args(["push", "origin", ref_name])
            .output()
            .context("Failed to push patch ref")?;

        if !output.status.success() {
            anyhow::bail!(
                "Failed to push patch ref: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }

    /// Get patch content from a ref
    pub fn get_patch_content(&self, ref_name: &str) -> Result<String> {
        let output = Command::new("git")
            .current_dir(&self.repo_root)
            .args(["cat-file", "-p", ref_name])
            .output()
            .context("Failed to read patch content")?;

        if !output.status.success() {
            anyhow::bail!("Failed to read patch content");
        }

        Ok(String::from_utf8(output.stdout)?)
    }

    /// Build CodeState from current repository
    pub fn build_code_state(&self, run_id: &str) -> Result<CodeState> {
        let repo_url = self.get_remote_url()?;
        let base_commit = self.get_head_commit()?;
        let patch = self.create_and_push_patch(run_id)?;

        Ok(CodeState {
            repo_url,
            base_commit,
            patch,
        })
    }

    /// Checkout to a specific commit
    pub fn checkout(&self, commit: &str) -> Result<()> {
        let output = Command::new("git")
            .current_dir(&self.repo_root)
            .args(["checkout", commit])
            .output()
            .context("Failed to checkout")?;

        if !output.status.success() {
            anyhow::bail!(
                "Failed to checkout: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }

    /// Apply a patch
    pub fn apply_patch(&self, ref_name: &str) -> Result<()> {
        let patch_content = self.get_patch_content(ref_name)?;

        let mut child = Command::new("git")
            .current_dir(&self.repo_root)
            .args(["apply"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .context("Failed to apply patch")?;

        {
            use std::io::Write;
            let stdin = child.stdin.as_mut().context("Failed to get stdin")?;
            stdin.write_all(patch_content.as_bytes())?;
        }

        let status = child.wait()?;
        if !status.success() {
            anyhow::bail!("Failed to apply patch");
        }

        Ok(())
    }

    /// Restore code state for replay
    pub fn restore_code_state(&self, code_state: &CodeState) -> Result<()> {
        // Checkout to the base commit
        self.checkout(&code_state.base_commit)?;

        // Apply patch if present
        if let Some(patch) = &code_state.patch {
            self.apply_patch(&patch.ref_)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_hash() {
        let hash = GitContext::sha256_hash("hello world");
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }
}
