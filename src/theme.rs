//! Theme system - Configurable colors

use crate::terminal::AnsiColor;
use eframe::egui;
use serde::{Deserialize, Serialize};

/// Theme configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeConfig {
    pub name: String,
    pub bg: (u8, u8, u8),
    pub fg: (u8, u8, u8),
    pub cursor: (u8, u8, u8),
    pub selection: (u8, u8, u8, u8),
    pub selection_text: (u8, u8, u8),
    pub search_match: (u8, u8, u8, u8),
    pub search_active: (u8, u8, u8, u8),
    pub hyperlink: (u8, u8, u8),
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            name: "Dark".to_string(),
            bg: (30, 30, 30),
            fg: (204, 204, 204),
            cursor: (204, 204, 204),
            selection: (50, 100, 200, 100),
            selection_text: (255, 255, 255),
            search_match: (200, 200, 0, 80),
            search_active: (255, 255, 0, 150),
            hyperlink: (100, 180, 255),
        }
    }
}

/// Runtime theme with egui colors
#[allow(dead_code)]
pub struct Theme {
    pub bg: egui::Color32,
    pub fg: egui::Color32,
    pub cursor: egui::Color32,
    pub selection: egui::Color32,
    pub selection_text: egui::Color32,
    pub search_match: egui::Color32,
    pub search_active: egui::Color32,
    pub hyperlink: egui::Color32,
    config: ThemeConfig,
}

#[allow(dead_code)]
impl Theme {
    pub fn from_config(config: ThemeConfig) -> Self {
        Self {
            bg: egui::Color32::from_rgb(config.bg.0, config.bg.1, config.bg.2),
            fg: egui::Color32::from_rgb(config.fg.0, config.fg.1, config.fg.2),
            cursor: egui::Color32::from_rgb(config.cursor.0, config.cursor.1, config.cursor.2),
            selection: egui::Color32::from_rgba_premultiplied(config.selection.0, config.selection.1, config.selection.2, config.selection.3),
            selection_text: egui::Color32::from_rgb(config.selection_text.0, config.selection_text.1, config.selection_text.2),
            search_match: egui::Color32::from_rgba_premultiplied(config.search_match.0, config.search_match.1, config.search_match.2, config.search_match.3),
            search_active: egui::Color32::from_rgba_premultiplied(config.search_active.0, config.search_active.1, config.search_active.2, config.search_active.3),
            hyperlink: egui::Color32::from_rgb(config.hyperlink.0, config.hyperlink.1, config.hyperlink.2),
            config,
        }
    }

    pub fn load_or_default() -> Self {
        let config = if let Some(config_dir) = dirs::config_dir() {
            let theme_path = config_dir.join("terminal").join("theme.json");
            if theme_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&theme_path) {
                    serde_json::from_str(&content).unwrap_or_default()
                } else {
                    ThemeConfig::default()
                }
            } else {
                ThemeConfig::default()
            }
        } else {
            ThemeConfig::default()
        };
        Self::from_config(config)
    }

    pub fn save(&self) {
        if let Some(config_dir) = dirs::config_dir() {
            let dir = config_dir.join("terminal");
            let _ = std::fs::create_dir_all(&dir);
            let theme_path = dir.join("theme.json");
            if let Ok(content) = serde_json::to_string_pretty(&self.config) {
                let _ = std::fs::write(theme_path, content);
            }
        }
    }

    pub fn color_to_egui(&self, color: AnsiColor, is_foreground: bool) -> egui::Color32 {
        match color {
            AnsiColor::Default => if is_foreground { self.fg } else { self.bg },
            _ => {
                let (r, g, b) = color.to_rgb();
                egui::Color32::from_rgb(r, g, b)
            }
        }
    }
}
