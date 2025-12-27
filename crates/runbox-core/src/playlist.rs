use serde::{Deserialize, Serialize};

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
}
