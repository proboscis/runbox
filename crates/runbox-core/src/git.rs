use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::VerboseLogger;
use crate::{CodeState, Patch};

/// Information about an existing git worktree
#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    /// Path to the worktree
    pub path: PathBuf,
    /// HEAD commit of the worktree
    pub commit: String,
    /// Branch name (if any)
    pub branch: Option<String>,
}

/// Result of worktree-based replay execution
#[derive(Debug)]
pub struct WorktreeReplayResult {
    /// Path to the worktree where execution occurred
    pub worktree_path: PathBuf,
    /// Whether the worktree was newly created or reused
    pub reused: bool,
}

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

    /// Restore code state for replay (DEPRECATED: use restore_code_state_in_worktree instead)
    pub fn restore_code_state(&self, code_state: &CodeState) -> Result<()> {
        // Checkout to the base commit
        self.checkout(&code_state.base_commit)?;

        // Apply patch if present
        if let Some(patch) = &code_state.patch {
            self.apply_patch(&patch.ref_)?;
        }

        Ok(())
    }

    // === Worktree Operations ===

    /// List all existing worktrees
    pub fn list_worktrees(&self, logger: &VerboseLogger) -> Result<Vec<WorktreeInfo>> {
        logger.log_vvv("git", "listing worktrees");

        let output = Command::new("git")
            .current_dir(&self.repo_root)
            .args(["worktree", "list", "--porcelain"])
            .output()
            .context("Failed to list worktrees")?;

        if !output.status.success() {
            anyhow::bail!(
                "Failed to list worktrees: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let stdout = String::from_utf8(output.stdout)?;
        let mut worktrees = Vec::new();
        let mut current_path: Option<PathBuf> = None;
        let mut current_commit: Option<String> = None;
        let mut current_branch: Option<String> = None;

        for line in stdout.lines() {
            if line.starts_with("worktree ") {
                // Save previous worktree if complete
                if let (Some(path), Some(commit)) = (current_path.take(), current_commit.take()) {
                    worktrees.push(WorktreeInfo {
                        path,
                        commit,
                        branch: current_branch.take(),
                    });
                }
                current_path = Some(PathBuf::from(line.strip_prefix("worktree ").unwrap()));
            } else if line.starts_with("HEAD ") {
                current_commit = Some(line.strip_prefix("HEAD ").unwrap().to_string());
            } else if line.starts_with("branch ") {
                current_branch = Some(line.strip_prefix("branch ").unwrap().to_string());
            }
        }

        // Don't forget the last one
        if let (Some(path), Some(commit)) = (current_path, current_commit) {
            worktrees.push(WorktreeInfo {
                path,
                commit,
                branch: current_branch,
            });
        }

        logger.log_vvv("git", &format!("found {} worktrees", worktrees.len()));
        Ok(worktrees)
    }

    /// Find an existing worktree that matches the given base commit
    pub fn find_worktree_by_commit(
        &self,
        base_commit: &str,
        worktree_base_dir: &Path,
        logger: &VerboseLogger,
    ) -> Result<Option<WorktreeInfo>> {
        logger.log_vv(
            "worktree",
            &format!("checking existing worktrees for commit {}...", &base_commit[..8.min(base_commit.len())]),
        );

        let canonical_base_dir = std::fs::canonicalize(worktree_base_dir)
            .unwrap_or_else(|_| worktree_base_dir.to_path_buf());

        let worktrees = self.list_worktrees(logger)?;

        for wt in worktrees {
            let canonical_worktree_path =
                std::fs::canonicalize(&wt.path).unwrap_or_else(|_| wt.path.clone());
            // Only consider worktrees under our base directory
            if !canonical_worktree_path.starts_with(&canonical_base_dir) {
                continue;
            }

            // Check if commit matches (allow prefix match)
            if wt.commit.starts_with(base_commit) || base_commit.starts_with(&wt.commit) {
                logger.log_vv(
                    "worktree",
                    &format!("found matching worktree at {}", wt.path.display()),
                );
                return Ok(Some(wt));
            }
        }

        logger.log_vv("worktree", "no matching worktree found");
        Ok(None)
    }

    /// Create a new worktree at the specified path and commit
    pub fn create_worktree(
        &self,
        worktree_path: &Path,
        commit: &str,
        logger: &VerboseLogger,
    ) -> Result<()> {
        logger.log_vv(
            "worktree",
            &format!("creating new at {}", worktree_path.display()),
        );

        // Ensure parent directory exists
        if let Some(parent) = worktree_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }

        logger.log_vvv(
            "git",
            &format!("git worktree add --detach {} {}", worktree_path.display(), commit),
        );

        let output = Command::new("git")
            .current_dir(&self.repo_root)
            .args([
                "worktree",
                "add",
                "--detach",
                worktree_path.to_str().unwrap(),
                commit,
            ])
            .output()
            .context("Failed to create worktree")?;

        if !output.status.success() {
            anyhow::bail!(
                "Failed to create worktree: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        logger.log_vv("git", &format!("checkout base_commit: {}", commit));
        Ok(())
    }

    /// Remove a worktree
    pub fn remove_worktree(&self, worktree_path: &Path, logger: &VerboseLogger) -> Result<()> {
        logger.log_vv(
            "worktree",
            &format!("removing {}", worktree_path.display()),
        );

        logger.log_vvv(
            "git",
            &format!("git worktree remove --force {}", worktree_path.display()),
        );

        let output = Command::new("git")
            .current_dir(&self.repo_root)
            .args([
                "worktree",
                "remove",
                "--force",
                worktree_path.to_str().unwrap(),
            ])
            .output()
            .context("Failed to remove worktree")?;

        if !output.status.success() {
            anyhow::bail!(
                "Failed to remove worktree: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }

    /// Apply patch in a specific worktree
    pub fn apply_patch_in_worktree(
        &self,
        worktree_path: &Path,
        ref_name: &str,
        logger: &VerboseLogger,
    ) -> Result<()> {
        logger.log_vv("git", &format!("applying patch: {}", ref_name));

        // First fetch the patch content from the main repo
        let patch_content = self.get_patch_content(ref_name)?;

        logger.log_vvv("git", &format!("patch content length: {} bytes", patch_content.len()));

        let mut child = Command::new("git")
            .current_dir(worktree_path)
            .args(["apply"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .context("Failed to apply patch in worktree")?;

        {
            use std::io::Write;
            let stdin = child.stdin.as_mut().context("Failed to get stdin")?;
            stdin.write_all(patch_content.as_bytes())?;
        }

        let status = child.wait()?;
        if !status.success() {
            anyhow::bail!("Failed to apply patch in worktree");
        }

        Ok(())
    }

    /// Restore code state in an isolated worktree (non-destructive)
    ///
    /// This is the safe replacement for `restore_code_state` that creates/reuses
    /// a worktree instead of modifying the current repository.
    pub fn restore_code_state_in_worktree(
        &self,
        code_state: &CodeState,
        run_id: &str,
        worktree_base_dir: &Path,
        reuse_existing: bool,
        logger: &VerboseLogger,
    ) -> Result<WorktreeReplayResult> {
        let worktree_path = worktree_base_dir.join(run_id);

        // Check for existing worktree with same commit
        if reuse_existing {
            if let Some(existing) = self.find_worktree_by_commit(
                &code_state.base_commit,
                worktree_base_dir,
                logger,
            )? {
                logger.log_v(
                    "worktree",
                    &format!("reusing existing worktree at {}", existing.path.display()),
                );

                // If there's a patch, we might need to reset and reapply
                // For now, assume the worktree is in a good state if commit matches
                return Ok(WorktreeReplayResult {
                    worktree_path: existing.path,
                    reused: true,
                });
            }
        }

        // Check if worktree path already exists (maybe from a previous run with same ID)
        if worktree_path.exists() {
            logger.log_vv(
                "worktree",
                &format!("path {} already exists, checking if it's a worktree", worktree_path.display()),
            );

            // Check if it's already registered as a worktree
            let worktrees = self.list_worktrees(logger)?;
            let is_worktree = worktrees.iter().any(|wt| wt.path == worktree_path);

            if is_worktree {
                // Remove it first
                self.remove_worktree(&worktree_path, logger)?;
            } else {
                // Just a directory, remove it
                std::fs::remove_dir_all(&worktree_path)
                    .with_context(|| format!("Failed to remove directory: {}", worktree_path.display()))?;
            }
        }

        // Create new worktree
        logger.log_v(
            "worktree",
            &format!("creating new at {}", worktree_path.display()),
        );
        self.create_worktree(&worktree_path, &code_state.base_commit, logger)?;

        // Apply patch if present
        if let Some(patch) = &code_state.patch {
            self.apply_patch_in_worktree(&worktree_path, &patch.ref_, logger)?;
        }

        Ok(WorktreeReplayResult {
            worktree_path,
            reused: false,
        })
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
