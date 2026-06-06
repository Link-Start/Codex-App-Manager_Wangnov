//! Persisted app settings — chiefly the update source, so the user can point the
//! updater at the mirror, OpenAI directly, or a custom URL instead of a
//! hard-coded domain. Stored as JSON in the manager's data dir (outside any
//! Codex bundle), mirroring `provenance::ProvenanceStore`.

use serde::{Deserialize, Serialize};

use crate::errors::AppError;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    /// "auto" | "mirror" | "official" | "custom"
    pub source: String,
    pub custom_url: String,
    pub auto_check: bool,
    pub ask_before: bool,
    /// Always true — surfaced read-only. We never install an unsigned bundle.
    pub signed_only: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            source: "auto".to_string(),
            custom_url: String::new(),
            auto_check: true,
            ask_before: true,
            signed_only: true,
        }
    }
}

fn store_path() -> Option<std::path::PathBuf> {
    directories::ProjectDirs::from("io.github", "wangnov", "codexappmanager")
        .map(|dirs| dirs.data_dir().join("settings.json"))
}

impl AppSettings {
    pub fn load() -> Self {
        let Some(path) = store_path() else {
            return Self::default();
        };
        let mut s: Self = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        s.signed_only = true; // enforce regardless of what is on disk
        s
    }

    pub fn save(&self) -> Result<(), AppError> {
        let path = store_path().ok_or_else(|| AppError::Internal("no data directory".into()))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| AppError::Internal(format!("create data dir: {e}")))?;
        }
        let json = serde_json::to_vec_pretty(self)
            .map_err(|e| AppError::Internal(format!("serialize settings: {e}")))?;
        std::fs::write(&path, json).map_err(|e| AppError::Internal(format!("write settings: {e}")))
    }
}
