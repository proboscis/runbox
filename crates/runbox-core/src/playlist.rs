use serde::{Deserialize, Serialize};

use crate::runnable::stable_short_id;

/// A collection of RunTemplate references
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub playlist_id: String,
    pub name: String,
    pub items: Vec<PlaylistItem>,
}

/// Reference to a RunTemplate with optional display override
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistItem {
    pub template_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl PlaylistItem {
    /// Generate a short ID for this playlist item based on hash of playlist_id, index, and template_id.
    /// The short ID is deterministic, unique within a playlist, and stable across Rust versions (uses SHA256).
    ///
    /// This uses the same algorithm as `Runnable::PlaylistItem.short_id()` to ensure consistency
    /// between `runbox playlist show` and `runbox run <short_id>`.
    pub fn short_id(&self, playlist_id: &str, index: usize) -> String {
        // Use the same format as Runnable::PlaylistItem.short_id()
        // Format: "playlist_item\0" + playlist_id + "\0" + index + "\0" + template_id
        let mut data = b"playlist_item\0".to_vec();
        data.extend_from_slice(playlist_id.as_bytes());
        data.push(0);
        data.extend_from_slice(index.to_string().as_bytes());
        data.push(0);
        data.extend_from_slice(self.template_id.as_bytes());
        stable_short_id(&data)
    }
}

impl Playlist {
    /// Create a new empty playlist
    pub fn new(id: &str, name: &str) -> Self {
        Self {
            playlist_id: id.to_string(),
            name: name.to_string(),
            items: Vec::new(),
        }
    }

    /// Add a template reference
    pub fn add(&mut self, template_id: &str, label: Option<&str>) {
        self.items.push(PlaylistItem {
            template_id: template_id.to_string(),
            label: label.map(|s| s.to_string()),
        });
    }

    /// Validate the playlist
    pub fn validate(&self) -> Result<(), PlaylistValidationError> {
        if !self.playlist_id.starts_with("pl_") {
            return Err(PlaylistValidationError::InvalidPlaylistId(
                self.playlist_id.clone(),
            ));
        }

        for item in &self.items {
            if !item.template_id.starts_with("tpl_") {
                return Err(PlaylistValidationError::InvalidTemplateId(
                    item.template_id.clone(),
                ));
            }
        }

        Ok(())
    }

    /// Resolve an item by index (numeric string) or short ID
    /// Returns the index and a reference to the item
    pub fn resolve_item(&self, selector: &str) -> Option<(usize, &PlaylistItem)> {
        // Try to parse as numeric index first
        // Only use index if it's a valid index (within bounds)
        if let Ok(index) = selector.parse::<usize>() {
            if let Some(item) = self.items.get(index) {
                return Some((index, item));
            }
            // Index out of bounds - fall through to try as short ID
        }

        // Try to match as short ID (prefix match)
        let selector_lower = selector.to_lowercase();
        for (idx, item) in self.items.iter().enumerate() {
            let item_short_id = item.short_id(&self.playlist_id, idx);
            if item_short_id.starts_with(&selector_lower) {
                return Some((idx, item));
            }
        }

        None
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PlaylistValidationError {
    #[error("Invalid playlist_id: {0} (must start with 'pl_')")]
    InvalidPlaylistId(String),
    #[error("Invalid template_id in item: {0} (must start with 'tpl_')")]
    InvalidTemplateId(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_playlist_serialization() {
        let mut playlist = Playlist::new("pl_daily", "Daily Tasks");
        playlist.add("tpl_runner", Some("Runner"));
        playlist.add("tpl_eval", None);

        let json = serde_json::to_string_pretty(&playlist).unwrap();
        let parsed: Playlist = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.items.len(), 2);
        assert_eq!(parsed.items[0].label, Some("Runner".to_string()));
        assert_eq!(parsed.items[1].label, None);
    }

    #[test]
    fn test_short_id_generation() {
        let item = PlaylistItem {
            template_id: "tpl_echo".to_string(),
            label: Some("Echo Hello".to_string()),
        };

        let short_id = item.short_id("pl_daily", 0);

        // Should be 8 hex chars
        assert_eq!(short_id.len(), 8);
        assert!(short_id.chars().all(|c| c.is_ascii_hexdigit()));

        // Same inputs should produce same output (deterministic)
        let short_id2 = item.short_id("pl_daily", 0);
        assert_eq!(short_id, short_id2);

        // Different index should produce different short ID
        let short_id3 = item.short_id("pl_daily", 1);
        assert_ne!(short_id, short_id3);

        // Different playlist should produce different short ID
        let short_id4 = item.short_id("pl_other", 0);
        assert_ne!(short_id, short_id4);
    }

    #[test]
    fn test_resolve_item_by_index() {
        let mut playlist = Playlist::new("pl_daily", "Daily Tasks");
        playlist.add("tpl_echo", Some("Echo"));
        playlist.add("tpl_train", Some("Train"));

        // Resolve by index
        let result = playlist.resolve_item("0");
        assert!(result.is_some());
        let (idx, item) = result.unwrap();
        assert_eq!(idx, 0);
        assert_eq!(item.template_id, "tpl_echo");

        let result = playlist.resolve_item("1");
        assert!(result.is_some());
        let (idx, item) = result.unwrap();
        assert_eq!(idx, 1);
        assert_eq!(item.template_id, "tpl_train");

        // Out of bounds
        let result = playlist.resolve_item("2");
        assert!(result.is_none());
    }

    #[test]
    fn test_resolve_item_by_short_id() {
        let mut playlist = Playlist::new("pl_daily", "Daily Tasks");
        playlist.add("tpl_echo", Some("Echo"));
        playlist.add("tpl_train", Some("Train"));

        // Get the short ID of first item
        let short_id = playlist.items[0].short_id(&playlist.playlist_id, 0);

        // Full short ID match
        let result = playlist.resolve_item(&short_id);
        assert!(result.is_some());
        let (idx, item) = result.unwrap();
        assert_eq!(idx, 0);
        assert_eq!(item.template_id, "tpl_echo");

        // Prefix match (first 4 chars)
        let prefix = &short_id[..4];
        let result = playlist.resolve_item(prefix);
        assert!(result.is_some());
        let (idx, _) = result.unwrap();
        assert_eq!(idx, 0);
    }
}
