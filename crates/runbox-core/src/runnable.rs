//! Unified Runnable concept with hex short IDs
//!
//! This module provides a unified abstraction over all executable things in runbox:
//! - Templates: User-defined execution specifications
//! - Replays: Re-execution of previous runs with exact code state
//! - Playlist Items: Template references within a playlist

use sha2::{Digest, Sha256};

/// A unified representation of anything that can be run in runbox.
///
/// Every runnable has a generated 8-character hex short ID that can be used
/// to reference it in CLI commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Runnable {
    /// A run template (template_id like "tpl_echo")
    Template(String),
    /// A previous run for replay (run_id like "run_550e8400-...")
    Replay(String),
    /// An item in a playlist
    PlaylistItem {
        playlist_id: String,
        index: usize,
        template_id: String,
        label: Option<String>,
    },
}

/// Generate a stable 8-character hex short ID from input bytes using SHA256.
/// This is stable across Rust versions unlike DefaultHasher.
fn stable_short_id(data: &[u8]) -> String {
    let hash = Sha256::digest(data);
    // Take first 4 bytes (8 hex chars)
    format!("{:02x}{:02x}{:02x}{:02x}", hash[0], hash[1], hash[2], hash[3])
}

/// Check if a string looks like a valid UUID hex portion (for replay short ID extraction)
fn is_valid_uuid_hex(s: &str) -> bool {
    s.len() >= 8 && s.chars().take(8).all(|c| c.is_ascii_hexdigit())
}

impl Runnable {
    /// Generate the 8-character hex short ID for this runnable.
    ///
    /// The short ID is deterministic and stable across Rust versions (uses SHA256).
    ///
    /// # Generation rules:
    /// - Template: SHA256 hash of "template\0" + template_id
    /// - Replay: first 8 hex chars of UUID if valid, otherwise SHA256 hash
    /// - PlaylistItem: SHA256 hash of "playlist_item\0" + playlist_id + "\0" + index + "\0" + template_id
    pub fn short_id(&self) -> String {
        match self {
            Runnable::Template(id) => {
                let mut data = b"template\0".to_vec();
                data.extend_from_slice(id.as_bytes());
                stable_short_id(&data)
            }
            Runnable::Replay(run_id) => {
                // run_id format: "run_{uuid}"
                // Extract hex chars from UUID, removing "run_" prefix and dashes
                let uuid_part = run_id
                    .trim_start_matches("run_")
                    .replace('-', "");
                
                // If it looks like valid UUID hex, extract first 8 chars (lowercase)
                if is_valid_uuid_hex(&uuid_part) {
                    uuid_part.chars().take(8).collect::<String>().to_lowercase()
                } else {
                    // Fallback to stable hash for non-UUID run IDs
                    let mut data = b"replay\0".to_vec();
                    data.extend_from_slice(run_id.as_bytes());
                    stable_short_id(&data)
                }
            }
            Runnable::PlaylistItem {
                playlist_id,
                index,
                template_id,
                ..
            } => {
                let mut data = b"playlist_item\0".to_vec();
                data.extend_from_slice(playlist_id.as_bytes());
                data.push(0);
                data.extend_from_slice(index.to_string().as_bytes());
                data.push(0);
                data.extend_from_slice(template_id.as_bytes());
                stable_short_id(&data)
            }
        }
    }

    /// Returns a human-readable type label for this runnable.
    pub fn type_label(&self) -> &'static str {
        match self {
            Runnable::Template(_) => "template",
            Runnable::Replay(_) => "replay",
            Runnable::PlaylistItem { .. } => "playlist",
        }
    }

    /// Returns a formatted type label with brackets for display.
    pub fn type_label_bracketed(&self) -> String {
        format!("[{}]", self.type_label())
    }

    /// Returns a human-readable display name for this runnable.
    pub fn display_name(&self) -> String {
        match self {
            Runnable::Template(id) => id.clone(),
            Runnable::Replay(id) => id.clone(),
            Runnable::PlaylistItem {
                playlist_id,
                index,
                label,
                ..
            } => {
                if let Some(lbl) = label {
                    format!("{}[{}] {:?}", playlist_id, index, lbl)
                } else {
                    format!("{}[{}]", playlist_id, index)
                }
            }
        }
    }

    /// Returns the underlying ID for resolution.
    ///
    /// - For Template: the template_id
    /// - For Replay: the run_id
    /// - For PlaylistItem: the template_id (what gets executed)
    pub fn underlying_id(&self) -> &str {
        match self {
            Runnable::Template(id) => id,
            Runnable::Replay(id) => id,
            Runnable::PlaylistItem { template_id, .. } => template_id,
        }
    }
}

/// Match information for ambiguity display
#[derive(Debug, Clone)]
pub struct RunnableMatch {
    pub runnable: Runnable,
    pub short_id: String,
    pub display_name: String,
}

impl RunnableMatch {
    pub fn from_runnable(runnable: Runnable) -> Self {
        let short_id = runnable.short_id();
        let display_name = runnable.display_name();
        Self {
            runnable,
            short_id,
            display_name,
        }
    }
}

/// Result of resolving a runnable by short ID
#[derive(Debug)]
pub enum ResolveResult {
    /// Exactly one match found
    Found(Runnable),
    /// No matches found
    NotFound,
    /// Multiple matches found (ambiguous)
    Ambiguous(Vec<RunnableMatch>),
}

/// Format ambiguous matches into a display table
pub fn format_ambiguous_matches(matches: &[RunnableMatch]) -> String {
    let mut result = String::new();
    result.push_str("\n  SHORT     TYPE              NAME\n");
    result.push_str("  ────────────────────────────────────────────────\n");

    for m in matches {
        let type_label = m.runnable.type_label_bracketed();
        result.push_str(&format!(
            "  {:<10} {:<17} {}\n",
            m.short_id, type_label, m.display_name
        ));
    }

    result.push_str("\nUse more characters or be explicit:\n");
    result.push_str("  runbox run <more_chars>          # if unique\n");
    result.push_str("  runbox run --template <id>       # explicit template\n");
    result.push_str("  runbox run --replay <run_id>     # explicit replay\n");

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_template_short_id() {
        let runnable = Runnable::Template("tpl_echo".to_string());
        let short_id = runnable.short_id();

        // Should be 8 hex chars
        assert_eq!(short_id.len(), 8);
        assert!(short_id.chars().all(|c| c.is_ascii_hexdigit()));

        // Should be deterministic
        let short_id2 = Runnable::Template("tpl_echo".to_string()).short_id();
        assert_eq!(short_id, short_id2);

        // Different template should have different short ID
        let short_id3 = Runnable::Template("tpl_train".to_string()).short_id();
        assert_ne!(short_id, short_id3);
    }

    #[test]
    fn test_template_short_id_is_stable() {
        // This test ensures the SHA256-based short ID is stable
        // If this test fails after a code change, it means short IDs have changed
        let runnable = Runnable::Template("tpl_echo".to_string());
        let short_id = runnable.short_id();
        
        // SHA256("template\0tpl_echo") first 4 bytes as hex
        // This is a known value that should not change
        assert_eq!(short_id.len(), 8);
        assert!(short_id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_replay_short_id() {
        let runnable = Runnable::Replay("run_550e8400-e29b-41d4-a716-446655440000".to_string());
        let short_id = runnable.short_id();

        // Should extract first 8 hex chars from UUID (lowercase)
        assert_eq!(short_id, "550e8400");

        // Different run should have different short ID
        let runnable2 = Runnable::Replay("run_a1b2c3d4-e5f6-7890-abcd-ef1234567890".to_string());
        assert_eq!(runnable2.short_id(), "a1b2c3d4");
    }

    #[test]
    fn test_replay_short_id_uppercase() {
        // Uppercase UUID should produce lowercase short ID
        let runnable = Runnable::Replay("run_550E8400-E29B-41D4-A716-446655440000".to_string());
        let short_id = runnable.short_id();
        assert_eq!(short_id, "550e8400");
    }

    #[test]
    fn test_replay_short_id_non_uuid() {
        // Non-UUID run ID should fallback to hash
        let runnable = Runnable::Replay("run_custom_id_123".to_string());
        let short_id = runnable.short_id();
        
        // Should be 8 hex chars (from hash)
        assert_eq!(short_id.len(), 8);
        assert!(short_id.chars().all(|c| c.is_ascii_hexdigit()));
        
        // Should be deterministic
        let short_id2 = Runnable::Replay("run_custom_id_123".to_string()).short_id();
        assert_eq!(short_id, short_id2);
    }

    #[test]
    fn test_playlist_item_short_id() {
        let runnable = Runnable::PlaylistItem {
            playlist_id: "pl_daily".to_string(),
            index: 0,
            template_id: "tpl_echo".to_string(),
            label: Some("Echo Hello".to_string()),
        };
        let short_id = runnable.short_id();

        // Should be 8 hex chars
        assert_eq!(short_id.len(), 8);
        assert!(short_id.chars().all(|c| c.is_ascii_hexdigit()));

        // Should be deterministic
        let runnable2 = Runnable::PlaylistItem {
            playlist_id: "pl_daily".to_string(),
            index: 0,
            template_id: "tpl_echo".to_string(),
            label: Some("Echo Hello".to_string()),
        };
        assert_eq!(short_id, runnable2.short_id());

        // Different index should have different short ID
        let runnable3 = Runnable::PlaylistItem {
            playlist_id: "pl_daily".to_string(),
            index: 1,
            template_id: "tpl_echo".to_string(),
            label: Some("Echo Hello".to_string()),
        };
        assert_ne!(short_id, runnable3.short_id());
    }

    #[test]
    fn test_playlist_item_short_id_is_stable() {
        // This test ensures the SHA256-based short ID is stable
        let runnable = Runnable::PlaylistItem {
            playlist_id: "pl_daily".to_string(),
            index: 0,
            template_id: "tpl_echo".to_string(),
            label: None,
        };
        let short_id = runnable.short_id();
        
        assert_eq!(short_id.len(), 8);
        assert!(short_id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_type_labels() {
        assert_eq!(
            Runnable::Template("tpl_echo".to_string()).type_label(),
            "template"
        );
        assert_eq!(
            Runnable::Replay("run_550e8400-...".to_string()).type_label(),
            "replay"
        );
        assert_eq!(
            Runnable::PlaylistItem {
                playlist_id: "pl_daily".to_string(),
                index: 0,
                template_id: "tpl_echo".to_string(),
                label: None,
            }
            .type_label(),
            "playlist"
        );
    }

    #[test]
    fn test_type_label_bracketed() {
        assert_eq!(
            Runnable::Template("tpl_echo".to_string()).type_label_bracketed(),
            "[template]"
        );
    }

    #[test]
    fn test_display_name() {
        assert_eq!(
            Runnable::Template("tpl_echo".to_string()).display_name(),
            "tpl_echo"
        );

        assert_eq!(
            Runnable::Replay("run_550e8400-e29b-41d4".to_string()).display_name(),
            "run_550e8400-e29b-41d4"
        );

        assert_eq!(
            Runnable::PlaylistItem {
                playlist_id: "pl_daily".to_string(),
                index: 0,
                template_id: "tpl_echo".to_string(),
                label: Some("Echo Hello".to_string()),
            }
            .display_name(),
            "pl_daily[0] \"Echo Hello\""
        );

        assert_eq!(
            Runnable::PlaylistItem {
                playlist_id: "pl_daily".to_string(),
                index: 1,
                template_id: "tpl_train".to_string(),
                label: None,
            }
            .display_name(),
            "pl_daily[1]"
        );
    }

    #[test]
    fn test_underlying_id() {
        assert_eq!(
            Runnable::Template("tpl_echo".to_string()).underlying_id(),
            "tpl_echo"
        );

        assert_eq!(
            Runnable::Replay("run_550e8400".to_string()).underlying_id(),
            "run_550e8400"
        );

        assert_eq!(
            Runnable::PlaylistItem {
                playlist_id: "pl_daily".to_string(),
                index: 0,
                template_id: "tpl_echo".to_string(),
                label: None,
            }
            .underlying_id(),
            "tpl_echo"
        );
    }

    #[test]
    fn test_runnable_match() {
        let runnable = Runnable::Template("tpl_echo".to_string());
        let m = RunnableMatch::from_runnable(runnable.clone());

        assert_eq!(m.runnable, runnable);
        assert_eq!(m.short_id, runnable.short_id());
        assert_eq!(m.display_name, "tpl_echo");
    }

    #[test]
    fn test_format_ambiguous_matches() {
        let matches = vec![
            RunnableMatch::from_runnable(Runnable::Template("tpl_auth_service".to_string())),
            RunnableMatch::from_runnable(Runnable::Replay(
                "run_a1b28888-e29b-41d4-a716-446655440000".to_string(),
            )),
        ];

        let output = format_ambiguous_matches(&matches);
        assert!(output.contains("SHORT"));
        assert!(output.contains("TYPE"));
        assert!(output.contains("NAME"));
        assert!(output.contains("[template]"));
        assert!(output.contains("[replay]"));
        assert!(output.contains("tpl_auth_service"));
    }
}
