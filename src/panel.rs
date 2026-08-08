//! Panel system - Tabs and split views

use crate::persistence::TabState;
use crate::terminal::{Terminal, TerminalBounds};
use crate::pty::PtyProcess;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

/// A single terminal tab
pub struct Tab {
    pub id: String,
    pub title: String,
    pub terminal: Arc<Mutex<Terminal>>,
    pub pty: Option<Arc<PtyProcess>>,
    pub selecting: bool,
    pub working_directory: Option<String>,
    pub process_exited: bool,
    /// Shared flag to signal the reader thread to stop.
    /// Set to true when the Tab is dropped.
    shutdown: Arc<AtomicBool>,
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
        let shutdown = Arc::new(AtomicBool::new(false));

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
            let shutdown_clone = shutdown.clone();
            thread::spawn(move || {
                let mut buffer = [0u8; 8192];
                while !shutdown_clone.load(Ordering::Relaxed) {
                    match reader.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(n) => {
                            let _ = terminal_clone.lock().unwrap_or_else(|e| e.into_inner())
                                .write_output(&buffer[..n]);
                        }
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
            selecting: false,
            working_directory: None,
            process_exited: false,
            shutdown,
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

impl Drop for Tab {
    fn drop(&mut self) {
        // Signal the reader thread to stop so it releases its Arc<Terminal>.
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

/// Panel with tabs
pub struct Panel {
    pub tabs: Vec<Tab>,
    pub active_index: usize,
}

impl Panel {
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            active_index: 0,
        }
    }

    pub fn add_tab(&mut self, tab: Tab) {
        self.tabs.push(tab);
        self.active_index = self.tabs.len() - 1;
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active_index)
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active_index)
    }
}
