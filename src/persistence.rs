//! Persistence - Save and restore terminal sessions

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Tab state for persistence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabState {
    pub id: String,
    pub title: String,
    pub working_directory: Option<String>,
}

/// Session state for persistence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    pub tabs: Vec<TabState>,
    pub active_index: usize,
    pub version: u32,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            tabs: Vec::new(),
            active_index: 0,
            version: 1,
        }
    }
}

/// Session manager for persistence
#[allow(dead_code)]
pub struct SessionManager {
    session_path: PathBuf,
}

#[allow(dead_code)]
impl SessionManager {
    pub fn new() -> Self {
        let session_path = if let Some(config_dir) = dirs::config_dir() {
            config_dir.join("terminal").join("session.json")
        } else {
            PathBuf::from("session.json")
        };

        Self { session_path }
    }

    pub fn save_session(&self, tabs: Vec<TabState>) {
        let session = SessionState {
            tabs,
            active_index: 0,
            version: 1,
        };

        if let Some(parent) = self.session_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        if let Ok(content) = serde_json::to_string_pretty(&session) {
            let _ = std::fs::write(&self.session_path, content);
        }
    }

    pub fn load_session(&self) -> Option<SessionState> {
        if !self.session_path.exists() {
            return None;
        }

        let content = std::fs::read_to_string(&self.session_path).ok()?;
        serde_json::from_str(&content).ok()
    }

    pub fn clear_session(&self) {
        let _ = std::fs::remove_file(&self.session_path);
    }
}
