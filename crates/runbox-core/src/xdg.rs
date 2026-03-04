//! XDG Base Directory Specification implementation
//!
//! Provides XDG-compliant paths that work consistently across platforms.
//! On macOS, this intentionally does NOT use ~/Library/* paths - it uses
//! ~/.local/share, ~/.local/state, ~/.config, and ~/.cache just like Linux.
//!
//! See: https://specifications.freedesktop.org/basedir-spec/basedir-spec-latest.html

use std::env;
use std::path::PathBuf;

/// Get the home directory
fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from)
}

/// Returns $XDG_DATA_HOME or ~/.local/share
///
/// Used for: user-specific data files (templates, playlists, records)
pub fn xdg_data_home() -> PathBuf {
    if let Ok(path) = env::var("XDG_DATA_HOME") {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    home_dir()
        .map(|h| h.join(".local").join("share"))
        .unwrap_or_else(|| PathBuf::from(".local/share"))
}

/// Returns $XDG_STATE_HOME or ~/.local/state
///
/// Used for: state files that persist between restarts but aren't important
/// enough for backup (SQLite index, logs, task state)
pub fn xdg_state_home() -> PathBuf {
    if let Ok(path) = env::var("XDG_STATE_HOME") {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    home_dir()
        .map(|h| h.join(".local").join("state"))
        .unwrap_or_else(|| PathBuf::from(".local/state"))
}

/// Returns $XDG_CONFIG_HOME or ~/.config
///
/// Used for: user-specific configuration files
pub fn xdg_config_home() -> PathBuf {
    if let Ok(path) = env::var("XDG_CONFIG_HOME") {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    home_dir()
        .map(|h| h.join(".config"))
        .unwrap_or_else(|| PathBuf::from(".config"))
}

/// Returns $XDG_CACHE_HOME or ~/.cache
///
/// Used for: user-specific non-essential cached data
pub fn xdg_cache_home() -> PathBuf {
    if let Ok(path) = env::var("XDG_CACHE_HOME") {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    home_dir()
        .map(|h| h.join(".cache"))
        .unwrap_or_else(|| PathBuf::from(".cache"))
}

/// Get the runbox data directory ($XDG_DATA_HOME/runbox)
///
/// Contains: templates/, playlists/, records/
pub fn runbox_data_dir() -> PathBuf {
    xdg_data_home().join("runbox")
}

/// Get the runbox state directory ($XDG_STATE_HOME/runbox)
///
/// Contains: runbox.db, logs/
pub fn runbox_state_dir() -> PathBuf {
    xdg_state_home().join("runbox")
}

/// Get the runbox config directory ($XDG_CONFIG_HOME/runbox)
///
/// Contains: config.toml
pub fn runbox_config_dir() -> PathBuf {
    xdg_config_home().join("runbox")
}

/// Get the runbox cache directory ($XDG_CACHE_HOME/runbox)
///
/// Contains: temporary files, build caches
pub fn runbox_cache_dir() -> PathBuf {
    xdg_cache_home().join("runbox")
}

/// Check if the old macOS storage location exists
///
/// Returns the path if ~/Library/Application Support/runbox exists
pub fn legacy_macos_dir() -> Option<PathBuf> {
    home_dir().and_then(|h| {
        let legacy = h.join("Library").join("Application Support").join("runbox");
        if legacy.exists() {
            Some(legacy)
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_xdg_data_home_default() {
        // Save current value
        let saved = env::var("XDG_DATA_HOME").ok();

        // Unset XDG_DATA_HOME
        env::remove_var("XDG_DATA_HOME");

        let path = xdg_data_home();

        // Should be ~/.local/share
        if let Some(home) = home_dir() {
            assert_eq!(path, home.join(".local").join("share"));
        }

        // Restore
        if let Some(val) = saved {
            env::set_var("XDG_DATA_HOME", val);
        }
    }

    #[test]
    fn test_xdg_data_home_custom() {
        // Save current value
        let saved = env::var("XDG_DATA_HOME").ok();

        // Set custom value
        env::set_var("XDG_DATA_HOME", "/custom/data");

        let path = xdg_data_home();
        assert_eq!(path, PathBuf::from("/custom/data"));

        // Restore
        match saved {
            Some(val) => env::set_var("XDG_DATA_HOME", val),
            None => env::remove_var("XDG_DATA_HOME"),
        }
    }

    #[test]
    fn test_xdg_state_home_default() {
        let saved = env::var("XDG_STATE_HOME").ok();
        env::remove_var("XDG_STATE_HOME");

        let path = xdg_state_home();

        if let Some(home) = home_dir() {
            assert_eq!(path, home.join(".local").join("state"));
        }

        if let Some(val) = saved {
            env::set_var("XDG_STATE_HOME", val);
        }
    }

    #[test]
    fn test_xdg_config_home_default() {
        let saved = env::var("XDG_CONFIG_HOME").ok();
        env::remove_var("XDG_CONFIG_HOME");

        let path = xdg_config_home();

        if let Some(home) = home_dir() {
            assert_eq!(path, home.join(".config"));
        }

        if let Some(val) = saved {
            env::set_var("XDG_CONFIG_HOME", val);
        }
    }

    #[test]
    fn test_xdg_cache_home_default() {
        let saved = env::var("XDG_CACHE_HOME").ok();
        env::remove_var("XDG_CACHE_HOME");

        let path = xdg_cache_home();

        if let Some(home) = home_dir() {
            assert_eq!(path, home.join(".cache"));
        }

        if let Some(val) = saved {
            env::set_var("XDG_CACHE_HOME", val);
        }
    }

    #[test]
    fn test_macos_never_uses_library() {
        // Even on macOS, default paths should not contain "Library"
        let saved_data = env::var("XDG_DATA_HOME").ok();
        let saved_state = env::var("XDG_STATE_HOME").ok();
        let saved_config = env::var("XDG_CONFIG_HOME").ok();
        let saved_cache = env::var("XDG_CACHE_HOME").ok();

        env::remove_var("XDG_DATA_HOME");
        env::remove_var("XDG_STATE_HOME");
        env::remove_var("XDG_CONFIG_HOME");
        env::remove_var("XDG_CACHE_HOME");

        let paths = [
            xdg_data_home(),
            xdg_state_home(),
            xdg_config_home(),
            xdg_cache_home(),
        ];

        for path in &paths {
            let path_str = path.to_string_lossy();
            assert!(
                !path_str.contains("Library"),
                "Path {} should not contain 'Library'",
                path_str
            );
        }

        // Restore
        if let Some(val) = saved_data {
            env::set_var("XDG_DATA_HOME", val);
        }
        if let Some(val) = saved_state {
            env::set_var("XDG_STATE_HOME", val);
        }
        if let Some(val) = saved_config {
            env::set_var("XDG_CONFIG_HOME", val);
        }
        if let Some(val) = saved_cache {
            env::set_var("XDG_CACHE_HOME", val);
        }
    }

    #[test]
    fn test_runbox_data_dir() {
        let saved = env::var("XDG_DATA_HOME").ok();
        env::remove_var("XDG_DATA_HOME");

        let path = runbox_data_dir();

        if let Some(home) = home_dir() {
            assert_eq!(path, home.join(".local").join("share").join("runbox"));
        }

        if let Some(val) = saved {
            env::set_var("XDG_DATA_HOME", val);
        }
    }

    #[test]
    fn test_runbox_state_dir() {
        let saved = env::var("XDG_STATE_HOME").ok();
        env::remove_var("XDG_STATE_HOME");

        let path = runbox_state_dir();

        if let Some(home) = home_dir() {
            assert_eq!(path, home.join(".local").join("state").join("runbox"));
        }

        if let Some(val) = saved {
            env::set_var("XDG_STATE_HOME", val);
        }
    }

    #[test]
    fn test_empty_env_var_falls_back_to_default() {
        let saved = env::var("XDG_DATA_HOME").ok();

        // Set to empty string
        env::set_var("XDG_DATA_HOME", "");

        let path = xdg_data_home();

        // Should fall back to default, not return empty path
        if let Some(home) = home_dir() {
            assert_eq!(path, home.join(".local").join("share"));
        }

        match saved {
            Some(val) => env::set_var("XDG_DATA_HOME", val),
            None => env::remove_var("XDG_DATA_HOME"),
        }
    }
}
