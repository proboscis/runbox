# Runbox Test Coverage Summary

## Methodology

**Scope**: This analysis covers public methods and functions callable by external code across all crates. Includes:
- Public struct methods (`impl Foo { pub fn bar() }`)
- Public free functions (`pub fn baz()`)
- Trait method implementations (`impl Trait for Foo`)
- Manual Display implementations (counted as public API)
- Python module functions (runbox-py)

**Excluded**:
- Private/crate-internal helpers (`pub(crate)`, non-pub)
- All Default implementations (both derived and manual `impl Default`)
- Derived trait implementations (Clone, Serialize via derive macro)
- Test setup code (struct construction without method calls)

**Test Classification**:
- "Directly tested" = test function explicitly calls the method
- "Transitively used" = called as part of setup but not the test subject (not counted)

## Overview

- **Total Test Functions**: 27 (verified via `cargo test`)
- **CLI Commands**: 18 (0 tested, 0% coverage)
- **Core Public Methods**: 94
- **Core Methods Tested**: 28
- **Core Coverage**: 30%
- **Python API Functions**: 9 (0 tested, 0% coverage)

## CLI Commands Coverage

| Command | Tested | Notes |
|---------|--------|-------|
| `run` | :x: | Template resolution, binding, git context, runtime spawning |
| `ps` | :x: | Lists runs with reconciliation |
| `stop` | :x: | Stops running processes via runtime adapter |
| `logs` | :x: | Shows log output with follow mode |
| `attach` | :x: | Attaches to tmux sessions |
| `history` | :x: | Lists run history |
| `show` | :x: | Shows run details |
| `replay` | :x: | Worktree-based replay |
| `validate` | :x: | JSON schema validation |
| `template list` | :x: | Lists templates |
| `template show` | :x: | Shows template details |
| `template create` | :x: | Creates template from JSON file |
| `template delete` | :x: | Deletes template |
| `playlist list` | :x: | Lists playlists |
| `playlist show` | :x: | Shows playlist details |
| `playlist create` | :x: | Creates playlist from JSON file |
| `playlist add` | :x: | Adds template to playlist |
| `playlist remove` | :x: | Removes template from playlist |

**CLI Coverage**: 0/18 commands (0%)

## Python API Coverage (runbox-py)

| Function | Tested | Notes |
|----------|--------|-------|
| `run_template` | :x: | Run template with bindings |
| `list_templates` | :x: | List all templates |
| `get_template` | :x: | Get template by ID |
| `list_runs` | :x: | List all runs |
| `get_run` | :x: | Get run by ID |
| `replay` | :x: | Replay a previous run |
| `list_playlists` | :x: | List all playlists |
| `get_playlist` | :x: | Get playlist by ID |
| `validate` | :x: | Validate JSON string |

**Python API Coverage**: 0/9 functions (0%)

## Core Library Coverage

### storage.rs

**Test Functions** (5): `test_run_storage`, `test_short_id`, `test_normalize_for_match`, `test_resolve_run_id`, `test_resolve_ambiguous_id`

| Method/Function | Tested | Test Case | Notes |
|-----------------|--------|-----------|-------|
| `short_id` (free fn) | :white_check_mark: | `test_short_id` | Short ID extraction |
| `Storage::new` | :x: | - | Tests use `with_base_dir` instead |
| `Storage::with_base_dir` | :white_check_mark: | `test_run_storage` | Directory creation |
| `Storage::base_dir` | :x: | - | Getter |
| `Storage::save_run` | :white_check_mark: | `test_run_storage` | Save and verify |
| `Storage::load_run` | :white_check_mark: | `test_run_storage` | Load and verify |
| `Storage::list_runs` | :white_check_mark: | `test_run_storage` | List with limit |
| `Storage::delete_run` | :white_check_mark: | `test_run_storage` | Delete and verify |
| `Storage::log_path` | :x: | - | Path builder |
| `Storage::logs_dir` | :x: | - | Path builder |
| `Storage::save_template` | :x: | - | Not tested |
| `Storage::load_template` | :x: | - | Not tested |
| `Storage::list_templates` | :x: | - | Not tested |
| `Storage::delete_template` | :x: | - | Not tested |
| `Storage::save_playlist` | :x: | - | Not tested |
| `Storage::load_playlist` | :x: | - | Not tested |
| `Storage::list_playlists` | :x: | - | Not tested |
| `Storage::delete_playlist` | :x: | - | Not tested |
| `Storage::resolve_run_id` | :white_check_mark: | `test_resolve_run_id`, `test_resolve_ambiguous_id` | Short ID resolution |
| `Storage::resolve_template_id` | :x: | - | Not tested |
| `Storage::resolve_playlist_id` | :x: | - | Not tested |

**Coverage**: 7/21 methods (33%)

**Quality Notes**:
- :white_check_mark: Happy-path CRUD for runs tested
- :x: No error path tests (file not found, invalid JSON)
- :x: `list_runs` ordering and limit behavior not verified
- :x: Template/playlist operations untested

### template.rs

**Test Functions** (1): `test_extract_variables`

| Method | Tested | Test Case | Notes |
|--------|--------|-----------|-------|
| `RunTemplate::validate` | :x: | - | ID and argv validation |
| `RunTemplate::extract_variables` | :white_check_mark: | `test_extract_variables` | Variable extraction |

**Coverage**: 1/2 methods (50%)

**Quality Notes**:
- :white_check_mark: Variable extraction from argv and env tested
- :x: No validation failure tests

### playlist.rs

**Test Functions** (1): `test_playlist_serialization`

| Method | Tested | Test Case | Notes |
|--------|--------|-----------|-------|
| `Playlist::new` | :white_check_mark: | `test_playlist_serialization` | Constructor |
| `Playlist::add` | :white_check_mark: | `test_playlist_serialization` | Add items |
| `Playlist::validate` | :x: | - | ID validation |

**Coverage**: 2/3 methods (67%)

**Quality Notes**:
- :white_check_mark: Serialization round-trip tested
- :x: No validation failure tests

### run.rs

**Test Functions** (4): `test_run_serialization`, `test_short_id`, `test_run_status_display`, `test_backwards_compatibility`

| Method | Tested | Test Case | Notes |
|--------|--------|-----------|-------|
| `Run::new` | :x: | - | Tests construct Run manually, not via `new()` |
| `Run::short_id` | :white_check_mark: | `test_short_id` | Short ID extraction |
| `Run::validate` | :x: | - | Multiple validation checks |
| `RunStatus::fmt` (Display) | :white_check_mark: | `test_run_status_display` | All status variants |

**Coverage**: 2/4 methods (50%)

**Quality Notes**:
- :white_check_mark: Short ID extraction tested
- :white_check_mark: All RunStatus Display variants tested
- :white_check_mark: Backwards compatibility explicitly tested (serde)
- :x: `Run::new` UUID generation not tested
- :x: No validation failure tests

### binding.rs

**Test Functions** (3): `test_substitute_variables`, `test_resolve_with_defaults`, `test_resolve_with_provided`

| Method | Tested | Test Case | Notes |
|--------|--------|-----------|-------|
| `BindingResolver::new` | :white_check_mark: | `test_resolve_with_defaults` | Constructor |
| `BindingResolver::with_bindings` | :white_check_mark: | `test_resolve_with_provided` | CLI bindings |
| `BindingResolver::with_interactive` | :x: | - | Callback setup |
| `BindingResolver::resolve` | :white_check_mark: | `test_resolve_with_defaults`, `test_resolve_with_provided` | Core resolution |
| `BindingResolver::build_run` | :x: | - | Run construction |

**Coverage**: 3/5 methods (60%)

**Quality Notes**:
- :white_check_mark: Default vs provided binding priority tested
- :x: Interactive callback path not tested
- :x: Missing variable error not tested

### validation.rs

**Test Functions** (4): `test_validate_run`, `test_validate_template`, `test_validate_playlist`, `test_auto_detect`

| Method | Tested | Test Case | Notes |
|--------|--------|-----------|-------|
| `Validator::new` | :white_check_mark: | All tests | Schema compilation |
| `Validator::validate_run` | :white_check_mark: | `test_validate_run` | Run validation |
| `Validator::validate_template` | :white_check_mark: | `test_validate_template` | Template validation |
| `Validator::validate_playlist` | :white_check_mark: | `test_validate_playlist` | Playlist validation |
| `Validator::validate_file` | :x: | - | File loading |
| `Validator::validate_auto` | :white_check_mark: | `test_auto_detect` | Type detection |
| `ValidationType::fmt` (Display) | :x: | - | Type name display |

**Coverage**: 5/7 methods (71%)

**Quality Notes**:
- :white_check_mark: All schema types tested with valid input
- :x: No validation failure tests (missing required fields, invalid types)
- :x: `validate_auto` unknown-type error path not tested
- :x: File-based validation not tested
- :x: `ValidationType` Display not tested

### config.rs

**Test Functions** (3): `test_default_config`, `test_config_source_display`, `test_parse_toml_config`

| Method | Tested | Test Case | Notes |
|--------|--------|-----------|-------|
| `RunboxConfig::load_global` | :x: | - | File loading |
| `RunboxConfig::global_config_path` | :x: | - | Path resolution |
| `ConfigSource::fmt` (Display) | :white_check_mark: | `test_config_source_display` | All source variants |
| `ResolvedValue::new` | :x: | - | Constructor |
| `ConfigResolver::new` | :x: | - | Constructor |
| `ConfigResolver::resolve_worktree_dir` | :x: | - | Layered resolution |
| `ConfigResolver::resolve_cleanup` | :x: | - | Layered resolution |
| `ConfigResolver::resolve_reuse` | :x: | - | Layered resolution |
| `ConfigResolver::resolve_verbosity` | :x: | - | Layered resolution |
| `VerboseLogger::new` | :x: | - | Constructor |
| `VerboseLogger::log_v` | :x: | - | Verbosity 1 logging |
| `VerboseLogger::log_vv` | :x: | - | Verbosity 2 logging |
| `VerboseLogger::log_vvv` | :x: | - | Verbosity 3 logging |
| `VerboseLogger::verbosity` | :x: | - | Getter |
| `VerboseLogger::log_config_resolution` | :x: | - | Config logging |
| `VerboseLogger::log_config_layers` | :x: | - | Layer logging |

**Coverage**: 1/16 methods (6%)

**Quality Notes**:
- :white_check_mark: ConfigSource Display tested for all variants
- :white_check_mark: TOML parsing tested (via serde)
- :x: No ConfigResolver tests (layered priority)
- :x: No VerboseLogger tests

### git.rs

**Test Functions** (1): `test_sha256_hash`

| Method | Tested | Test Case | Notes |
|--------|--------|-----------|-------|
| `GitContext::from_current_dir` | :x: | - | Repo detection |
| `GitContext::from_path` | :x: | - | Repo from path |
| `GitContext::repo_root` | :x: | - | Getter |
| `GitContext::get_remote_url` | :x: | - | Git remote |
| `GitContext::get_head_commit` | :x: | - | Git HEAD |
| `GitContext::has_uncommitted_changes` | :x: | - | Git status |
| `GitContext::get_diff` | :x: | - | Git diff |
| `GitContext::sha256_hash` | :white_check_mark: | `test_sha256_hash` | Hash computation |
| `GitContext::create_and_push_patch` | :x: | - | Patch creation |
| `GitContext::push_patch_ref` | :x: | - | Ref push |
| `GitContext::get_patch_content` | :x: | - | Patch retrieval |
| `GitContext::build_code_state` | :x: | - | Code state builder |
| `GitContext::checkout` | :x: | - | Git checkout |
| `GitContext::apply_patch` | :x: | - | Patch application |
| `GitContext::restore_code_state` | :x: | - | Legacy restore |
| `GitContext::list_worktrees` | :x: | - | Worktree listing |
| `GitContext::find_worktree_by_commit` | :x: | - | Worktree search |
| `GitContext::create_worktree` | :x: | - | Worktree creation |
| `GitContext::remove_worktree` | :x: | - | Worktree removal |
| `GitContext::apply_patch_in_worktree` | :x: | - | Patch in worktree |
| `GitContext::restore_code_state_in_worktree` | :x: | - | Main replay |

**Coverage**: 1/21 methods (5%)

**Quality Notes**:
- :white_check_mark: SHA256 hash correctness verified
- :x: **Critical gap**: All git operations untested
- :x: Requires temp repo setup for proper testing

### runtime/mod.rs

**Test Functions** (1): `test_registry_default_adapters`

| Method | Tested | Test Case | Notes |
|--------|--------|-----------|-------|
| `RuntimeRegistry::new` | :white_check_mark: | `test_registry_default_adapters` | Default adapters |
| `RuntimeRegistry::get` | :white_check_mark: | `test_registry_default_adapters` | Adapter lookup |
| `RuntimeRegistry::available` | :x: | - | List adapters |

**Coverage**: 2/3 methods (67%)

### runtime/background.rs

**Test Functions** (2): `test_spawn_and_stop`, `test_spawn_with_output`

| Method | Tested | Test Case | Notes |
|--------|--------|-----------|-------|
| `BackgroundAdapter::new` | :white_check_mark: | `test_spawn_and_stop` | Constructor |
| `RuntimeAdapter::name` | :x: | - | Trait method |
| `RuntimeAdapter::spawn` | :white_check_mark: | `test_spawn_and_stop`, `test_spawn_with_output` | Process spawning |
| `RuntimeAdapter::stop` | :white_check_mark: | `test_spawn_and_stop` | Process termination |
| `RuntimeAdapter::attach` | :x: | - | Returns error (by design) |
| `RuntimeAdapter::is_alive` | :white_check_mark: | `test_spawn_and_stop` | Status check |

**Coverage**: 4/6 methods (67%)

**Quality Notes**:
- :white_check_mark: Full lifecycle tested (spawn, alive, stop)
- :white_check_mark: Output redirection verified
- :x: `attach` error return not tested (returns error by design)
- :x: `name` trait method not tested

### runtime/tmux.rs

**Test Functions** (2): `test_window_name`, `test_shell_escape`

| Method | Tested | Test Case | Notes |
|--------|--------|-----------|-------|
| `TmuxAdapter::new` | :x: | - | Constructor |
| `RuntimeAdapter::name` | :x: | - | Trait method |
| `RuntimeAdapter::spawn` | :x: | - | Requires tmux |
| `RuntimeAdapter::stop` | :x: | - | Requires tmux |
| `RuntimeAdapter::attach` | :x: | - | Requires tmux |
| `RuntimeAdapter::is_alive` | :x: | - | Requires tmux |

**Coverage**: 0/6 public methods (0%)

**Quality Notes**:
- :x: No public method tests (only private helpers tested)
- :x: Integration tests would require tmux

## Summary Table

| Module | Public Methods | Tested | Coverage |
|--------|----------------|--------|----------|
| storage.rs | 21 | 7 | 33% |
| template.rs | 2 | 1 | 50% |
| playlist.rs | 3 | 2 | 67% |
| run.rs | 4 | 2 | 50% |
| binding.rs | 5 | 3 | 60% |
| validation.rs | 7 | 5 | 71% |
| config.rs | 16 | 1 | 6% |
| git.rs | 21 | 1 | 5% |
| runtime/mod.rs | 3 | 2 | 67% |
| runtime/background.rs | 6 | 4 | 67% |
| runtime/tmux.rs | 6 | 0 | 0% |
| **Core Total** | **94** | **28** | **30%** |
| **CLI Commands** | **18** | **0** | **0%** |
| **Python API** | **9** | **0** | **0%** |
| **Grand Total** | **121** | **28** | **23%** |

## High Priority Gaps

1. **CLI Integration Tests** - 0/18 commands tested
2. **Python API Tests** - 0/9 functions tested
3. **GitContext** - 1/21 methods tested (5%), critical for replay
4. **ConfigResolver** - 0/5 methods tested (config.rs overall: 1/16), untested layered config
5. **TmuxAdapter** - 0/6 public methods tested
6. **Validation failures** - All tests are happy-path only

## Recommendations

### Immediate (High Impact)

1. **CLI integration tests** - Add `assert_cmd = "2"` and `predicates = "3"` to `crates/runbox-cli/Cargo.toml` dev-dependencies; create `crates/runbox-cli/tests/integration.rs`:
   ```rust
   use assert_cmd::Command;
   use std::path::PathBuf;

   fn fixtures_dir() -> PathBuf {
       PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
   }

   #[test]
   fn test_validate_command() {
       Command::cargo_bin("runbox").unwrap()
           .args(["validate", fixtures_dir().join("valid_run.json").to_str().unwrap()])
           .assert().success();
   }
   ```

2. **GitContext tests** - Use `tempfile` crate to create temp git repos:
   ```rust
   use tempfile::TempDir;
   #[test]
   fn test_from_path() {
       let dir = TempDir::new().unwrap();
       std::process::Command::new("git").args(["init"]).current_dir(&dir).output().unwrap();
       let ctx = GitContext::from_path(dir.path()).unwrap();
       assert!(ctx.repo_root().exists());
   }
   ```

3. **ConfigResolver tests** - Create temp files and test layered priority:
   ```rust
   #[test]
   fn test_cli_overrides_global() {
       let resolver = ConfigResolver::new(None).unwrap();
       let result = resolver.resolve_worktree_dir(Some(&PathBuf::from("/cli/path")));
       assert_eq!(result.source, ConfigSource::CliFlag);
   }
   ```

4. **Validation failure tests** - Add invalid input tests:
   ```rust
   #[test]
   fn test_validate_run_missing_fields() {
       let validator = Validator::new().unwrap();
       let invalid = serde_json::json!({"run_id": "bad"});
       assert!(validator.validate_run(&invalid).is_err());
   }
   ```

### Short-term

5. **Template/playlist storage tests** - Copy `test_run_storage` pattern for templates and playlists
6. **Error path tests** - Test `load_run` with non-existent ID, `save_run` with invalid path
7. **TmuxAdapter tests** - Use `#[cfg(feature = "tmux-tests")]` for conditional compilation:
   ```rust
   #[cfg(feature = "tmux-tests")]
   #[test]
   fn test_tmux_spawn() { /* requires tmux installed */ }
   ```

### Infrastructure

8. **Add test fixtures directory** - Create `crates/runbox-cli/tests/fixtures/` with:
   - `valid_run.json` - minimal valid Run JSON
   - `valid_template.json` - minimal valid RunTemplate JSON
   - `invalid_run.json` - missing required fields for failure tests
9. **CI tmux setup** - Add to CI workflow:
   ```yaml
   - name: Install tmux
     run: sudo apt-get install -y tmux
   - name: Run tmux tests
     run: cargo test --features tmux-tests
   ```
10. **Python API tests** - Use `pyo3` testing patterns or integration tests with pytest

## Appendix: Test Verification

```
$ cargo test 2>&1 | grep "running .* tests"
running 0 tests      # runbox-cli
running 27 tests     # runbox-core
running 0 tests      # runbox-py
```

Total: **27 test functions** in `runbox-core`
