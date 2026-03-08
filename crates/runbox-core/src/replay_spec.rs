use std::collections::HashMap;

use crate::{CodeState, Patch, Record, Run};

/// Everything needed to replay an execution in a git worktree or runtime.
///
/// This is source-agnostic and can be constructed from either a `Run` or a `Record`.
#[derive(Debug, Clone)]
pub struct ReplaySpec {
    pub id: String,
    pub argv: Vec<String>,
    pub cwd: String,
    pub env: HashMap<String, String>,
    pub timeout_sec: u64,
    pub code_state: CodeState,
}

impl From<&Run> for ReplaySpec {
    fn from(run: &Run) -> Self {
        Self {
            id: run.run_id.clone(),
            argv: run.exec.argv.clone(),
            cwd: run.exec.cwd.clone(),
            env: run.exec.env.clone(),
            timeout_sec: run.exec.timeout_sec,
            code_state: run.code_state.clone(),
        }
    }
}

impl From<&Record> for ReplaySpec {
    fn from(record: &Record) -> Self {
        Self {
            id: record.record_id.clone(),
            argv: record.command.argv.clone(),
            cwd: record.command.cwd.clone(),
            env: record.command.env.clone(),
            timeout_sec: 0,
            code_state: CodeState {
                repo_url: record.git_state.repo_url.clone(),
                base_commit: record.git_state.commit.clone(),
                patch: record.git_state.patch_ref.as_ref().map(|ref_| Patch {
                    ref_: ref_.clone(),
                    sha256: String::new(),
                }),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::{RecordCommand, RecordGitState};

    use super::*;

    #[test]
    fn test_replay_spec_from_run() {
        let run = Run::new(
            crate::Exec {
                argv: vec!["echo".to_string(), "hello".to_string()],
                cwd: ".".to_string(),
                env: HashMap::from([("FOO".to_string(), "bar".to_string())]),
                timeout_sec: 42,
            },
            CodeState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                base_commit: "a1b2c3d4e5f6789012345678901234567890abcd".to_string(),
                patch: Some(Patch {
                    ref_: "refs/patches/run_test".to_string(),
                    sha256: "deadbeef".to_string(),
                }),
            },
        );

        let spec = ReplaySpec::from(&run);
        assert_eq!(spec.id, run.run_id);
        assert_eq!(spec.argv, vec!["echo", "hello"]);
        assert_eq!(spec.cwd, ".");
        assert_eq!(spec.env.get("FOO"), Some(&"bar".to_string()));
        assert_eq!(spec.timeout_sec, 42);
        assert_eq!(spec.code_state.base_commit, run.code_state.base_commit);
        assert_eq!(
            spec.code_state
                .patch
                .as_ref()
                .map(|patch| patch.ref_.as_str()),
            Some("refs/patches/run_test")
        );
    }

    #[test]
    fn test_replay_spec_from_record() {
        let record = Record::new(
            RecordGitState {
                repo_url: "git@github.com:org/repo.git".to_string(),
                commit: "a1b2c3d4e5f6789012345678901234567890abcd".to_string(),
                patch_ref: Some("refs/patches/rec_test".to_string()),
            },
            RecordCommand {
                argv: vec!["python".to_string(), "train.py".to_string()],
                cwd: "src".to_string(),
                env: HashMap::from([("CUDA_VISIBLE_DEVICES".to_string(), "0".to_string())]),
            },
        );

        let spec = ReplaySpec::from(&record);
        assert_eq!(spec.id, record.record_id);
        assert_eq!(spec.argv, vec!["python", "train.py"]);
        assert_eq!(spec.cwd, "src");
        assert_eq!(spec.env.get("CUDA_VISIBLE_DEVICES"), Some(&"0".to_string()));
        assert_eq!(spec.timeout_sec, 0);
        assert_eq!(spec.code_state.repo_url, record.git_state.repo_url);
        assert_eq!(spec.code_state.base_commit, record.git_state.commit);
        assert_eq!(
            spec.code_state
                .patch
                .as_ref()
                .map(|patch| patch.ref_.as_str()),
            Some("refs/patches/rec_test")
        );
    }
}
