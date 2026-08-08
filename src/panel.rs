//! Panel system - Tabs and split views

use crate::persistence::TabState;
use crate::terminal::{Terminal, TerminalBounds};
use crate::pty::PtyProcess;
use std::sync::{Arc, Mutex};
use std::thread;

/// A single terminal tab
pub struct Tab {
    pub id: String,
    pub title: String,
    pub terminal: Arc<Mutex<Terminal>>,
    pub pty: Option<Arc<PtyProcess>>,
    pub search_mode: bool,
    pub search_query: String,
    pub selecting: bool,
    pub working_directory: Option<String>,
    pub process_exited: bool,
}

impl Tab {
    pub fn new(title: String) -> Self {
        Self::new_with_cwd(title, None)
    }

    pub fn new_with_cwd(title: String, working_dir: Option<String>) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        let cols = 120;
        let rows = 35;
        let cell_width = 9.0;
        let cell_height = 18.0;

        let bounds = TerminalBounds {
            cell_width,
            cell_height,
            width: cols as f32 * cell_width,
            height: rows as f32 * cell_height,
        };

        let terminal = Arc::new(Mutex::new(Terminal::new(bounds)));

        let pty = match PtyProcess::new_with_cwd(cols, rows, working_dir.clone()) {
            Ok(pty) => Some(Arc::new(pty)),
            Err(e) => {
                log::error!("Failed to create PTY for tab {}: {}", title, e);
                None
            }
        };

        if let Some(ref pty) = pty {
            let reader = pty.reader();
            let terminal_clone = terminal.clone();
            thread::spawn(move || {
                let mut buffer = [0u8; 8192];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(n) => { terminal_clone.lock().unwrap().write_output(&buffer[..n]); }
                        Err(_) => break,
                    }
                }
            });
        }

        Self {
            id,
            title,
            terminal,
            pty,
            search_mode: false,
            search_query: String::new(),
            selecting: false,
            working_directory: None,
            process_exited: false,
        }
    }

    pub fn from_state(state: TabState) -> Self {
        let mut tab = Self::new_with_cwd(state.title, state.working_directory.clone());
        tab.id = state.id;
        tab.working_directory = state.working_directory;
        tab
    }

    pub fn to_state(&self) -> TabState {
        TabState {
            id: self.id.clone(),
            title: self.title.clone(),
            working_directory: self.working_directory.clone(),
        }
    }
}

/// Panel with tabs and optional split
pub struct Panel {
    pub tabs: Vec<Tab>,
    pub active_index: usize,
    pub split: Option<SplitPanel>,
}

/// Split panel configuration
pub struct SplitPanel {
    pub first: Box<Panel>,
    pub second: Box<Panel>,
    pub active_side: SplitSide,
}

/// Which side of the split is active
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitSide {
    First,
    Second,
}

impl Panel {
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            active_index: 0,
            split: None,
        }
    }

    pub fn add_tab(&mut self, tab: Tab) {
        self.tabs.push(tab);
        self.active_index = self.tabs.len() - 1;
    }

    pub fn remove_tab(&mut self, index: usize) {
        if self.tabs.len() <= 1 {
            return; // Keep at least one tab
        }
        self.tabs.remove(index);
        if self.active_index >= self.tabs.len() {
            self.active_index = self.tabs.len() - 1;
        }
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        if let Some(ref split) = self.split {
            match split.active_side {
                SplitSide::First => split.first.active_tab(),
                SplitSide::Second => split.second.active_tab(),
            }
        } else {
            self.tabs.get(self.active_index)
        }
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        if let Some(ref mut split) = self.split {
            match split.active_side {
                SplitSide::First => split.first.active_tab_mut(),
                SplitSide::Second => split.second.active_tab_mut(),
            }
        } else {
            self.tabs.get_mut(self.active_index)
        }
    }

    /// Split the panel horizontally (top/bottom)
    pub fn split_horizontal(&mut self) {
        if self.tabs.is_empty() {
            return;
        }

        let active_tab = self.tabs.remove(self.active_index);
        let new_tab = Tab::new(format!("Terminal {}", self.tabs.len() + 2));

        let mut first = Panel::new();
        first.add_tab(active_tab);

        let mut second = Panel::new();
        second.add_tab(new_tab);

        self.split = Some(SplitPanel {
            first: Box::new(first),
            second: Box::new(second),
            active_side: SplitSide::First,
        });
    }

    /// Split the panel vertically (left/right)
    pub fn split_vertical(&mut self) {
        if self.tabs.is_empty() {
            return;
        }

        let active_tab = self.tabs.remove(self.active_index);
        let new_tab = Tab::new(format!("Terminal {}", self.tabs.len() + 2));

        let mut first = Panel::new();
        first.add_tab(active_tab);

        let mut second = Panel::new();
        second.add_tab(new_tab);

        self.split = Some(SplitPanel {
            first: Box::new(first),
            second: Box::new(second),
            active_side: SplitSide::First,
        });
    }

    /// Close the split and merge back to single panel
    pub fn close_split(&mut self) {
        if let Some(split) = self.split.take() {
            // Merge all tabs from both sides
            let mut all_tabs = Vec::new();
            all_tabs.extend(split.first.tabs);
            all_tabs.extend(split.second.tabs);

            if all_tabs.is_empty() {
                all_tabs.push(Tab::new("Terminal 1".to_string()));
            }

            self.tabs = all_tabs;
            self.active_index = 0;
        }
    }

    /// Check if panel has a split
    pub fn has_split(&self) -> bool {
        self.split.is_some()
    }
}
