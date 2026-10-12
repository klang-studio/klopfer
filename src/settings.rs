//! Settings shared by all Klopfer instances, stored in the user's config directory.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Where recordings go; `None` means the default folder
    pub recordings_dir: Option<PathBuf>,
    /// The sample browser's folder; `None` means the default folder
    pub browser_dir: Option<PathBuf>,
}

fn settings_file() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("Klopfer").join("settings.json"))
}

impl Settings {
    /// Loads the settings, falling back to the defaults if there are none or they're unreadable.
    pub fn load() -> Self {
        settings_file()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let file = settings_file().ok_or("no config directory")?;
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(file, json).map_err(|e| format!("cannot save settings: {e}"))
    }

    pub fn browser_dir(&self) -> PathBuf {
        self.browser_dir
            .clone()
            .or_else(dirs::audio_dir)
            .or_else(dirs::home_dir)
            .unwrap_or_default()
    }

    pub fn recordings_dir(&self) -> PathBuf {
        self.recordings_dir.clone().unwrap_or_else(|| {
            dirs::document_dir()
                .or_else(dirs::home_dir)
                .unwrap_or_default()
                .join("Klopfer")
                .join("Recordings")
        })
    }
}
