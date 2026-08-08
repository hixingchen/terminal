//! GUI Terminal Emulator
//!
//! Fixes implemented:
//! 1. PTY resize sync
//! 2. Real event listener (replaces DummyListener)
//! 3. Color mapping fix (Default fg/bg)
//! 4. Hyperlink click-to-open
//! 5. Cursor blinking
//! 6. Process exit handling
//! 7. IME support (Preedit/Commit handling)
//! 8. Persistence working directory restore
//! 9. IME enablement + candidate window positioning at cursor
//!    (egui only enables the platform IME when PlatformOutput::ime is set,
//!    which normally only TextEdit does — a custom-painted terminal never
//!    gets it, so winit never delivers Ime events and CJK input is impossible)
//! 10. CJK font fallback + wide-character rendering (two-cell glyphs)

mod terminal;
mod pty;
mod hyperlinks;
mod theme;
mod persistence;
mod panel;

use anyhow::Result;
use eframe::egui;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use terminal::{Cell, CursorShape, Modes, Point, SelectionType, TerminalBounds, TerminalEvent};
use theme::Theme;
use persistence::SessionManager;
use panel::{Panel, Tab};

fn main() -> Result<()> {
    // Set log level to error to suppress wgpu/vulkan warnings
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("error"))
        .filter_module("terminal", log::LevelFilter::Info)
        .init();
    log::info!("Starting GUI Terminal Emulator with wgpu backend");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([400.0, 300.0])
            .with_title("Terminal"),
        // Enable wgpu renderer for GPU acceleration
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native("Terminal", options, Box::new(|cc| Ok(Box::new(TerminalApp::new(cc)))))
        .map_err(|e| anyhow::anyhow!("Failed to run application: {}", e))?;
    Ok(())
}

/// Input events
enum InputEvent {
    Text(String),
    Key { key: egui::Key, modifiers: egui::Modifiers },
    Scroll(f32),
    Ime(String),
}


/// Cursor blink state
struct CursorBlink {
    visible: bool,
    last_toggle: Instant,
    interval: Duration,
}

impl CursorBlink {
    fn new() -> Self {
        Self {
            visible: true,
            last_toggle: Instant::now(),
            interval: Duration::from_millis(500),
        }
    }

    fn tick(&mut self) {
        if self.last_toggle.elapsed() >= self.interval {
            self.visible = !self.visible;
            self.last_toggle = Instant::now();
        }
    }

    fn reset(&mut self) {
        self.visible = true;
        self.last_toggle = Instant::now();
    }
}

/// IME state for handling Chinese/Japanese/Korean input
struct ImeState {
    /// Pre-edit text (composition in progress)
    preedit: String,
    /// Whether IME is active
    active: bool,
}

/// Install a CJK-capable fallback font so Chinese/Japanese/Korean text renders
/// instead of tofu boxes. epaint panics on unparseable font data, so every
/// candidate is validated before insertion.
fn install_cjk_fonts(ctx: &egui::Context) {
    const CANDIDATES: &[(&str, &str)] = &[
        ("Microsoft YaHei", "C:\\Windows\\Fonts\\msyh.ttc"),
        ("SimHei", "C:\\Windows\\Fonts\\simhei.ttf"),
        ("DengXian", "C:\\Windows\\Fonts\\Deng.ttf"),
        ("SimSun", "C:\\Windows\\Fonts\\simsun.ttc"),
        ("KaiTi", "C:\\Windows\\Fonts\\simkai.ttf"),
    ];

    for (name, path) in CANDIDATES {
        let Ok(bytes) = std::fs::read(path) else { continue };
        if !is_font_file(&bytes) {
            log::warn!("Skipping invalid font file: {path}");
            continue;
        }
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert((*name).to_owned(), egui::FontData::from_owned(bytes));
        for family in [egui::FontFamily::Monospace, egui::FontFamily::Proportional] {
            fonts.families.entry(family).or_default().push((*name).to_owned());
        }
        ctx.set_fonts(fonts);
        log::info!("Loaded CJK fallback font: {name} ({path})");
        return;
    }
    log::warn!("No CJK font found - Chinese/Japanese/Korean text will render as boxes");
}

/// Minimal sfnt/TTC magic-byte check (epaint panics on invalid font data).
fn is_font_file(data: &[u8]) -> bool {
    let magic = &data[..4.min(data.len())];
    matches!(magic, b"\x00\x01\x00\x00" | b"OTTO" | b"ttcf" | b"true" | b"typ1")
}

/// Main application
struct TerminalApp {
    panel: Panel,
    theme: Theme,
    session_manager: SessionManager,
    clipboard: Option<String>,
    font_size: f32,
    cell_width: f32,
    cell_height: f32,
    cursor_blink: CursorBlink,
    last_cols: u16,
    last_rows: u16,
    ime_state: ImeState,
    scrollbar_dragging: bool,
}

impl TerminalApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Install a CJK fallback font (the default fonts lack CJK glyphs, so
        // Chinese/Japanese/Korean would render as tofu boxes).
        install_cjk_fonts(&cc.egui_ctx);

        let theme = Theme::load_or_default();
        let session_manager = SessionManager::new();
        let mut panel = Panel::new();

        // Restore previous session
        if let Some(session) = session_manager.load_session() {
            for tab_state in session.tabs {
                let tab = Tab::from_state(tab_state);
                panel.add_tab(tab);
            }
        }

        if panel.tabs.is_empty() {
            panel.add_tab(Tab::new("Terminal 1".to_string()));
        }

        Self {
            panel,
            theme,
            session_manager,
            clipboard: None,
            font_size: 15.0,
            cell_width: 9.0,
            cell_height: 18.0,
            cursor_blink: CursorBlink::new(),
            last_cols: 0,
            last_rows: 0,
            ime_state: ImeState {
                preedit: String::new(),
                active: false,
            },
            scrollbar_dragging: false,
        }
    }

    fn pixel_to_point(&self, pos: egui::Pos2, origin: egui::Pos2, display_offset: usize) -> Point {
        let x = pos.x - origin.x;
        let y = pos.y - origin.y;
        // Rendering uses y = origin.y + (grid_line + display_offset) * cell_height,
        // so reverse: grid_line = (y / cell_height) - display_offset.
        let line = (y / self.cell_height) as i32 - display_offset as i32;
        Point::new(line.max(0), (x / self.cell_width).max(0.0) as usize)
    }

    fn cell_color(&self, cell: &Cell, is_selected: bool, is_search_match: bool, is_active_match: bool, is_hyperlink: bool) -> (egui::Color32, egui::Color32) {
        let mut fg = self.theme.color_to_egui(cell.foreground, true);
        let mut bg = self.theme.color_to_egui(cell.background, false);

        if cell.flags.inverse { std::mem::swap(&mut fg, &mut bg); }
        if is_selected { bg = self.theme.selection; fg = self.theme.selection_text; }
        if is_active_match { bg = self.theme.search_active; }
        else if is_search_match { bg = self.theme.search_match; }
        if is_hyperlink && !is_selected { fg = self.theme.hyperlink; }
        if cell.flags.dim {
            let [r, g, b, _] = fg.to_array();
            fg = egui::Color32::from_rgb(r / 2, g / 2, b / 2);
        }
        (fg, bg)
    }

    fn copy_selection(&mut self) {
        let term = self.panel.active_tab().map(|t| t.terminal.clone());
        if let Some(text) = term.and_then(|t| t.lock().unwrap().copy_selection()) {
            self.clipboard = Some(text.clone());
            if let Ok(mut clipboard) = arboard::Clipboard::new() {
                let _ = clipboard.set_text(text);
            }
        }
    }

    fn paste_clipboard(&self) {
        let text = if let Ok(mut clipboard) = arboard::Clipboard::new() {
            clipboard.get_text().ok()
        } else {
            self.clipboard.clone()
        };
        if let (Some(text), Some(tab)) = (text, self.panel.active_tab()) {
            if let Some(ref pty) = tab.pty {
                let content = tab.terminal.lock().unwrap();
                if content.get_content().mode.contains(Modes::BRACKETED_PASTE) {
                    let _ = pty.write(b"\x1b[200~");
                    let _ = pty.write(text.as_bytes());
                    let _ = pty.write(b"\x1b[201~");
                } else {
                    let _ = pty.write(text.as_bytes());
                }
            }
        }
    }

    fn process_input_events(&mut self, events: Vec<InputEvent>) {
        for event in events {
            match event {
                InputEvent::Text(text) | InputEvent::Ime(text) => {
                    if let Some(tab) = self.panel.active_tab_mut() {
                        if let Some(ref pty) = tab.pty {
                            let _ = pty.write(text.as_bytes());
                        }
                    }
                    self.cursor_blink.reset();
                }
                InputEvent::Key { key, modifiers } => {
                    self.cursor_blink.reset();
                    if modifiers.ctrl && !modifiers.shift {
                        // Ctrl+C - copy if selection exists, otherwise send interrupt
                        if key == egui::Key::C {
                            let has_selection = self.panel.active_tab()
                                .map(|tab| tab.terminal.lock().unwrap().get_content().selection.is_some())
                                .unwrap_or(false);
                            if has_selection {
                                self.copy_selection();
                            } else {
                                if let Some(tab) = self.panel.active_tab_mut() {
                                    if let Some(ref pty) = tab.pty {
                                        let _ = pty.write(&[0x03u8]);
                                    }
                                }
                            }
                            continue;
                        }
                        // Other Ctrl shortcuts
                        if let Some(tab) = self.panel.active_tab_mut() {
                            if let Some(ref pty) = tab.pty {
                                let bytes = match key {
                                    egui::Key::D => Some(vec![0x04]),
                                    egui::Key::Z => Some(vec![0x1a]),
                                    egui::Key::L => Some(vec![0x0c]),
                                    egui::Key::A => Some(vec![0x01]),
                                    egui::Key::E => Some(vec![0x05]),
                                    egui::Key::K => Some(vec![0x0b]),
                                    egui::Key::U => Some(vec![0x15]),
                                    egui::Key::W => Some(vec![0x17]),
                                    _ => None,
                                };
                                if let Some(bytes) = bytes {
                                    let _ = pty.write(&bytes);
                                    continue;
                                }
                            }
                        }
                    }
                    // Regular key handling
                    if let Some(tab) = self.panel.active_tab_mut() {
                        if let Some(ref pty) = tab.pty {
                            let bytes = match key {
                                egui::Key::Enter => vec![b'\r'],
                                egui::Key::Tab => vec![b'\t'],
                                egui::Key::Backspace => vec![0x7f],
                                egui::Key::Delete => vec![0x1b, b'[', b'3', b'~'],
                                egui::Key::Escape => vec![0x1b],
                                egui::Key::ArrowUp => vec![0x1b, b'[', b'A'],
                                egui::Key::ArrowDown => vec![0x1b, b'[', b'B'],
                                egui::Key::ArrowRight => vec![0x1b, b'[', b'C'],
                                egui::Key::ArrowLeft => vec![0x1b, b'[', b'D'],
                                egui::Key::Home => vec![0x1b, b'[', b'H'],
                                egui::Key::End => vec![0x1b, b'[', b'F'],
                                egui::Key::PageUp => vec![0x1b, b'[', b'5', b'~'],
                                egui::Key::PageDown => vec![0x1b, b'[', b'6', b'~'],
                                _ => continue,
                            };
                            let _ = pty.write(&bytes);
                        }
                    }
                }
                InputEvent::Scroll(delta) => {
                    if delta > 0.0 { self.font_size = (self.font_size - 1.0).max(8.0); }
                    else { self.font_size = (self.font_size + 1.0).min(32.0); }
                }
            }
        }
    }

    fn save_session(&self) {
        let tab_states = self.panel.tabs.iter().map(|t| t.to_state()).collect();
        self.session_manager.save_session(tab_states);
    }

    fn render_search_bar(&mut self, ui: &mut egui::Ui) {
        let (match_info, query) = if let Some(tab) = self.panel.active_tab() {
            let term = tab.terminal.lock().unwrap();
            let matches = term.get_search_matches();
            let active = term.get_active_match();
            let info = if matches.is_empty() {
                "No matches".to_string()
            } else {
                format!("{}/{}", active.map(|a| matches.iter().position(|m| m.start == a.start).unwrap_or(0) + 1).unwrap_or(1), matches.len())
            };
            (info, tab.search_query.clone())
        } else {
            return;
        };

        let mut query = query;
        ui.horizontal(|ui| {
            ui.label("🔍");
            let response = ui.text_edit_singleline(&mut query);
            if response.changed() {
                if let Some(tab) = self.panel.active_tab_mut() {
                    tab.search_query = query.clone();
                    tab.terminal.lock().unwrap().search(&query);
                }
            }
            ui.label(&match_info);
            if ui.button("⬆").clicked() {
                if let Some(tab) = self.panel.active_tab_mut() {
                    tab.terminal.lock().unwrap().prev_match();
                }
            }
            if ui.button("⬇").clicked() {
                if let Some(tab) = self.panel.active_tab_mut() {
                    tab.terminal.lock().unwrap().next_match();
                }
            }
            if ui.button("✕").clicked() {
                if let Some(tab) = self.panel.active_tab_mut() {
                    tab.search_mode = false;
                    tab.search_query.clear();
                    tab.terminal.lock().unwrap().clear_search();
                }
            }
        });
    }
}

impl eframe::App for TerminalApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Update cursor blink
        self.cursor_blink.tick();

        // Drain terminal events
        let mut needs_repaint = false;
        for tab in &mut self.panel.tabs {
            let events = tab.terminal.lock().unwrap().drain_events();
            for event in events {
                match event {
                    TerminalEvent::Wakeup => needs_repaint = true,
                    TerminalEvent::TitleChanged(title) => {
                        tab.title = title;
                        needs_repaint = true;
                    }
                    TerminalEvent::Bell => needs_repaint = true,
                    TerminalEvent::Exit => {
                        tab.process_exited = true;
                        needs_repaint = true;
                    }
                    TerminalEvent::ChildExit(_) => {
                        tab.process_exited = true;
                        needs_repaint = true;
                    }
                    _ => {}
                }
            }
        }

        if needs_repaint {
            ctx.request_repaint();
        }

        ctx.request_repaint_after(Duration::from_millis(50));

        // Collect input events
        let mut events = Vec::new();
        let mut copy = false;
        let mut paste = false;
        let mut search = false;
        let mut new_tab = false;
        let mut close_tab = false;
        let mut split_h = false;
        let mut split_v = false;
        let mut close_split = false;
        let mut font_increase = false;
        let mut font_decrease = false;

        ctx.input(|i| {
            if i.modifiers.ctrl && i.modifiers.shift {
                if i.key_pressed(egui::Key::C) { copy = true; }
                if i.key_pressed(egui::Key::V) { paste = true; }
                if i.key_pressed(egui::Key::F) { search = true; }
                if i.key_pressed(egui::Key::T) { new_tab = true; }
                if i.key_pressed(egui::Key::W) { close_tab = true; }
                if i.key_pressed(egui::Key::Backslash) { split_h = true; }
                if i.key_pressed(egui::Key::Slash) { split_v = true; }
                if i.key_pressed(egui::Key::X) { close_split = true; }
            }
            if i.modifiers.ctrl && i.key_pressed(egui::Key::Equals) { font_increase = true; }
            if i.modifiers.ctrl && i.key_pressed(egui::Key::Minus) { font_decrease = true; }
            // F1 key - no action (removed help)

            // Handle IME and text events
            for event in &i.events {
                match event {
                    egui::Event::Ime(ime_event) => {
                        match ime_event {
                            egui::ImeEvent::Commit(text) => {
                                // IME committed text (final input)
                                // Clear preedit state
                                self.ime_state.preedit.clear();
                                self.ime_state.active = false;
                                // Send committed text to terminal
                                events.push(InputEvent::Ime(text.clone()));
                            }
                            egui::ImeEvent::Preedit(text) => {
                                // IME preedit text (composition in progress)
                                if text.is_empty() {
                                    // Composition cancelled
                                    self.ime_state.preedit.clear();
                                    self.ime_state.active = false;
                                } else {
                                    // Composition in progress
                                    self.ime_state.preedit = text.clone();
                                    self.ime_state.active = true;
                                }
                            }
                            egui::ImeEvent::Enabled => {
                                self.ime_state.active = true;
                            }
                            egui::ImeEvent::Disabled => {
                                self.ime_state.active = false;
                                self.ime_state.preedit.clear();
                            }
                        }
                    }
                    egui::Event::Text(text) => {
                        // Regular text input (only if not in IME composition)
                        if !self.ime_state.active {
                            events.push(InputEvent::Text(text.clone()));
                        }
                    }
                    egui::Event::Key { key, pressed, modifiers, .. } => {
                        if *pressed {
                            // If IME is active, only pass through certain keys
                            if self.ime_state.active {
                                // Let IME handle the input, only pass through Escape
                                if *key == egui::Key::Escape {
                                    self.ime_state.active = false;
                                    self.ime_state.preedit.clear();
                                    events.push(InputEvent::Key { key: *key, modifiers: *modifiers });
                                }
                            } else {
                                events.push(InputEvent::Key { key: *key, modifiers: *modifiers });
                            }
                        }
                    }
                    _ => {}
                }
            }

            if i.raw_scroll_delta.y != 0.0 {
                if i.modifiers.ctrl {
                    events.push(InputEvent::Scroll(i.raw_scroll_delta.y));
                } else {
                    // Scroll viewport through scrollback (cmd behavior).
                    // positive delta = wheel up = scroll up through history.
                    let scroll_lines = (i.raw_scroll_delta.y / self.cell_height) as i32;
                    if let Some(tab) = self.panel.active_tab() {
                        tab.terminal.lock().unwrap().scroll_up(scroll_lines);
                    }
                }
            }
        });

        if copy { self.copy_selection(); }
        if paste { self.paste_clipboard(); }
        if search {
            if let Some(tab) = self.panel.active_tab_mut() {
                tab.search_mode = true;
                tab.search_query.clear();
            }
        }
        if new_tab {
            let new_idx = self.panel.tabs.len() + 1;
            self.panel.add_tab(Tab::new(format!("Terminal {}", new_idx)));
        }
        if close_tab {
            self.panel.remove_tab(self.panel.active_index);
        }
        if split_h { self.panel.split_horizontal(); }
        if split_v { self.panel.split_vertical(); }
        if close_split { self.panel.close_split(); }
        if font_increase { self.font_size = (self.font_size + 1.0).min(32.0); }
        if font_decrease { self.font_size = (self.font_size - 1.0).max(8.0); }

        // Process search mode input
        let search_mode = self.panel.active_tab().map_or(false, |t| t.search_mode);
        if search_mode {
            for event in events {
                match event {
                    InputEvent::Key { key, modifiers } => {
                        match key {
                            egui::Key::Escape => {
                                if let Some(tab) = self.panel.active_tab_mut() {
                                    tab.search_mode = false;
                                    tab.search_query.clear();
                                    tab.terminal.lock().unwrap().clear_search();
                                }
                            }
                            egui::Key::Enter => {
                                if let Some(tab) = self.panel.active_tab_mut() {
                                    if modifiers.shift { tab.terminal.lock().unwrap().prev_match(); }
                                    else { tab.terminal.lock().unwrap().next_match(); }
                                }
                            }
                            egui::Key::Backspace => {
                                if let Some(tab) = self.panel.active_tab_mut() {
                                    tab.search_query.pop();
                                    if tab.search_query.is_empty() { tab.terminal.lock().unwrap().clear_search(); }
                                    else { tab.terminal.lock().unwrap().search(&tab.search_query); }
                                }
                            }
                            _ => {}
                        }
                    }
                    InputEvent::Text(text) | InputEvent::Ime(text) => {
                        if let Some(tab) = self.panel.active_tab_mut() {
                            tab.search_query.push_str(&text);
                            tab.terminal.lock().unwrap().search(&tab.search_query);
                        }
                    }
                    _ => {}
                }
            }
        } else {
            self.process_input_events(events);
        }

        // Search bar (only show when searching)
        let search_mode = self.panel.active_tab().map_or(false, |t| t.search_mode);
        if search_mode {
            egui::TopBottomPanel::top("search_bar").show(ctx, |ui| {
                self.render_search_bar(ui);
            });
        }

        // Minimal status bar - only show when needed
        let (show_status, search_mode, search_query, process_exited, title) =
            if let Some(tab) = self.panel.active_tab() {
                (tab.search_mode || tab.process_exited || self.panel.has_split(),
                 tab.search_mode, tab.search_query.clone(), tab.process_exited, tab.title.clone())
            } else {
                (false, false, String::new(), false, String::new())
            };

        if show_status {
            egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if search_mode {
                        ui.colored_label(egui::Color32::from_rgb(100, 200, 100), format!("Search: {}", search_query));
                    }
                    if process_exited {
                        ui.colored_label(egui::Color32::from_rgb(255, 100, 100), "Process Exited");
                        if ui.button("Restart").clicked() {
                            if let Some(tab) = self.panel.active_tab_mut() {
                                let new_tab = Tab::new(title);
                                *tab = new_tab;
                            }
                        }
                    }
                });
            });
        }

        // Main terminal area - full screen terminal
        egui::CentralPanel::default().show(ctx, |ui| {
            let Some(tab) = self.panel.active_tab() else { return; };
            let content = tab.terminal.lock().unwrap().get_content().clone();

            let available_width = ui.available_width();
            let available_height = ui.available_height();
            let display_cols = (available_width / self.cell_width) as u16;
            let display_rows = (available_height / self.cell_height) as u16;

            // FIX 1: PTY resize sync
            if display_cols != self.last_cols || display_rows != self.last_rows {
                self.last_cols = display_cols;
                self.last_rows = display_rows;

                let new_bounds = TerminalBounds {
                    cell_width: self.cell_width,
                    cell_height: self.cell_height,
                    width: display_cols as f32 * self.cell_width,
                    height: display_rows as f32 * self.cell_height,
                };

                if let Some(tab) = self.panel.active_tab_mut() {
                    tab.terminal.lock().unwrap().resize(new_bounds);
                    if let Some(ref pty) = tab.pty {
                        let _ = pty.resize(display_cols, display_rows);
                    }
                }
            }

            let (response, painter) = ui.allocate_painter(
                egui::Vec2::new(display_cols as f32 * self.cell_width, display_rows as f32 * self.cell_height),
                egui::Sense::click_and_drag(),
            );

            let rect = response.rect;
            let origin = rect.min;
            painter.rect_filled(rect, 0.0, self.theme.bg);

            // Batch rendering: group cells by line and render with LayoutJob
            use egui::text::{LayoutJob, LayoutSection};
            use egui::{TextFormat, FontId, Color32};

            let (search_matches, active_match) = if let Some(tab) = self.panel.active_tab() {
                let term = tab.terminal.lock().unwrap();
                (term.get_search_matches().to_vec(), term.get_active_match().cloned())
            } else {
                (Vec::new(), None)
            };

            // Group cells by line (sorted Vec instead of HashMap — cells are already
            // ordered by line, so we can partition in one pass).
            // Note: when display_offset > 0, line numbers can be negative
            // (scrollback cells above the viewport). All cells from display_iter
            // are valid and must be rendered.
            let offset = content.display_offset as i32;
            let mut line_groups: Vec<(i32, Vec<&terminal::IndexedCell>)> = Vec::new();
            for cell in &content.cells {
                let row = cell.point.line;
                let viewport_line = row + offset;
                if viewport_line >= display_rows as i32 || viewport_line < 0 { continue; }
                match line_groups.last_mut() {
                    Some((line, cells)) if *line == row => cells.push(cell),
                    _ => line_groups.push((row, vec![cell])),
                }
            }

            // Render each line as a batch.
            // When scrolled back (display_offset > 0), display_iter returns
            // cells with negative line numbers (scrollback). We must shift
            // them down so they appear at the top of the visible area.
            for (line_idx, cells) in &line_groups {
                let y = origin.y + (*line_idx + offset) as f32 * self.cell_height;
                let mut line_text = String::new();
                let mut sections = Vec::new();
                let mut last_col = 0;
                // Wide (CJK) glyph overlays: (column, text, color) painted after
                // the line galley at their exact 2-cell slots.
                let mut wide_glyphs: Vec<(usize, String, Color32)> = Vec::new();

                // Sort cells by column
                let mut sorted_cells: Vec<_> = cells.iter().collect();
                sorted_cells.sort_by_key(|c| c.point.column);

                for cell in sorted_cells {
                    let col = cell.point.column as usize;
                    if col >= display_cols as usize { continue; }

                    // Add spaces for gaps (batched — one section per contiguous gap)
                    if last_col < col {
                        let gap_start = line_text.len();
                        for _ in last_col..col {
                            line_text.push(' ');
                        }
                        sections.push(LayoutSection {
                            leading_space: 0.0,
                            byte_range: gap_start..line_text.len(),
                            format: TextFormat {
                                font_id: FontId::monospace(self.font_size),
                                color: self.theme.fg,
                                ..Default::default()
                            },
                        });
                    }

                    let is_selected = content.selection.as_ref().map_or(false, |sel| cell.point >= sel.start && cell.point <= sel.end);
                    let is_search_match = search_matches.iter().any(|m| cell.point >= m.start && cell.point <= m.end);
                    let is_active_match = active_match.as_ref().map_or(false, |m| cell.point >= m.start && cell.point <= m.end);
                    let is_hyperlink = cell.cell.hyperlink.is_some();

                    let (fg, bg) = self.cell_color(&cell.cell, is_selected, is_search_match, is_active_match, is_hyperlink);

                    // Draw background if needed
                    let x = origin.x + col as f32 * self.cell_width;
                    let char_width = self.cell_width * cell.cell.width as f32;
                    let char_rect = egui::Rect::from_min_size(egui::Pos2::new(x, y), egui::Vec2::new(char_width, self.cell_height));

                    if bg != self.theme.bg {
                        painter.rect_filled(char_rect, 0.0, bg);
                    }

                    // Add character to line.
                    // Wide (CJK) glyphs are 1em wide; at the monospace size they
                    // would advance only one cell and misalign the whole line, so
                    // reserve two space cells in the job and paint the glyph as an
                    // overlay sized to exactly two cells (wide_glyphs, below).
                    let start_idx = line_text.len();
                    let mut overlay_text: Option<String> = None;
                    if cell.cell.width >= 2 {
                        line_text.push(' ');
                        line_text.push(' ');
                        if cell.cell.character != '\0' {
                            let mut text = cell.cell.character.to_string();
                            if let Some(ref zw) = cell.cell.zero_width {
                                for c in zw {
                                    text.push(*c);
                                }
                            }
                            overlay_text = Some(text);
                        }
                    } else {
                        if cell.cell.character != '\0' {
                            line_text.push(cell.cell.character);
                        } else {
                            line_text.push(' ');
                        }
                        // Add zero-width characters
                        if let Some(ref zw) = cell.cell.zero_width {
                            for c in zw {
                                line_text.push(*c);
                            }
                        }
                    }

                    let end_idx = line_text.len();

                    // Calculate text color with bold
                    let mut text_color = fg;
                    if cell.cell.flags.bold {
                        let [r, g, b, a] = text_color.to_array();
                        text_color = Color32::from_rgba_premultiplied(
                            (r as u16 + 30).min(255) as u8,
                            (g as u16 + 30).min(255) as u8,
                            (b as u16 + 30).min(255) as u8,
                            a
                        );
                    }

                    sections.push(LayoutSection {
                        leading_space: 0.0,
                        byte_range: start_idx..end_idx,
                        format: TextFormat {
                            font_id: FontId::monospace(self.font_size),
                            color: text_color,
                            ..Default::default()
                        },
                    });

                    // Queue wide glyph overlay (painted after the line galley).
                    if let Some(text) = overlay_text {
                        wide_glyphs.push((col, text, text_color));
                    }

                    // Draw underline
                    if cell.cell.flags.underline {
                        painter.line_segment(
                            [egui::Pos2::new(char_rect.left(), char_rect.bottom() - 1.0),
                             egui::Pos2::new(char_rect.right(), char_rect.bottom() - 1.0)],
                            egui::Stroke::new(1.0, fg)
                        );
                    }

                    // Draw strikethrough
                    if cell.cell.flags.strikethrough {
                        painter.line_segment(
                            [egui::Pos2::new(char_rect.left(), char_rect.center().y),
                             egui::Pos2::new(char_rect.right(), char_rect.center().y)],
                            egui::Stroke::new(1.0, fg)
                        );
                    }

                    last_col = col + cell.cell.width as usize;
                }

                // Create LayoutJob and render the line
                if !line_text.is_empty() {
                    let layout_job = LayoutJob {
                        text: line_text,
                        sections,
                        ..Default::default()
                    };

                    let galley = ui.fonts(|f| f.layout_job(layout_job));
                    let line_rect = egui::Rect::from_min_size(
                        egui::Pos2::new(origin.x, y),
                        egui::Vec2::new(display_cols as f32 * self.cell_width, self.cell_height)
                    );
                    painter.galley(line_rect.min, galley, Color32::WHITE);

                    // Paint wide (CJK) glyph overlays at their exact 2-cell slots.
                    // Same size as Latin text — the two reserved space cells in
                    // the job already provide the 2-cell advance.
                    if !wide_glyphs.is_empty() {
                        let font_id = FontId::new(self.font_size, egui::FontFamily::Monospace);
                        for (col, text, color) in wide_glyphs {
                            let x = origin.x + col as f32 * self.cell_width;
                            let galley = ui.fonts(|f| f.layout_no_wrap(text, font_id.clone(), color));
                            painter.galley(egui::Pos2::new(x, y), galley, color);
                        }
                    }
                }
            }

            // FIX 5: Cursor with blinking
            let cursor = &content.cursor;
            let cursor_row = cursor.point.line as usize;
            let cursor_col = cursor.point.column as usize;
            if cursor_row < display_rows as usize && cursor_col < display_cols as usize {
                let cursor_x = origin.x + cursor_col as f32 * self.cell_width;
                // Account for display_offset: when scrolled back, the cursor's
                // visual position shifts down just like content cells do.
                let cursor_y = origin.y + (cursor_row as i32 + content.display_offset as i32) as f32 * self.cell_height;

                // FIX 9: Enable the platform IME and anchor its candidate window to the
                // terminal cursor while the terminal has focus. Only TextEdit sets
                // PlatformOutput::ime, so a custom-painted terminal must do it itself —
                // otherwise egui-winit never calls set_ime_allowed(true) and winit on
                // Windows delivers no Ime events (no composition, no CJK input).
                // Equivalent to Window::invalidate_character_coordinates ->
                // PlatformWindow::update_ime_position.
                //
                // Deliberately NOT gated on cursor.visible: TUIs like the claude
                // CLI hide the native cursor and draw their own, but IME must
                // still be enabled and follow the logical cursor position.
                if response.has_focus() {
                    let ime_rect = egui::Rect::from_min_size(
                        egui::Pos2::new(cursor_x, cursor_y),
                        egui::Vec2::new(self.cell_width, self.cell_height),
                    );
                    ui.ctx().output_mut(|o| {
                        o.ime = Some(egui::output::IMEOutput {
                            rect: ime_rect,
                            cursor_rect: ime_rect,
                        });
                    });
                } else if self.ime_state.active {
                    // Terminal lost focus (tab switch, search bar, split). The
                    // platform IME is being disabled, but winit sends no Disabled
                    // event for an aborted composition — clear the stale state or
                    // Event::Text would be filtered forever (no more typing).
                    self.ime_state.active = false;
                    self.ime_state.preedit.clear();
                }

                if cursor.visible && self.cursor_blink.visible {
                    let cursor_rect = egui::Rect::from_min_size(egui::Pos2::new(cursor_x, cursor_y), egui::Vec2::new(self.cell_width, self.cell_height));
                    match cursor.shape {
                        CursorShape::Block => {
                            painter.rect_filled(cursor_rect, 0.0, self.theme.cursor);
                            if let Some(cell) = content.cells.iter().find(|c| c.point == cursor.point) {
                                if cell.cell.character != ' ' && cell.cell.character != '\0' {
                                    painter.text(cursor_rect.center(), egui::Align2::CENTER_CENTER, cell.cell.character.to_string(), egui::FontId::monospace(self.font_size), self.theme.bg);
                                }
                            }
                        }
                        CursorShape::Underline => {
                            painter.rect_filled(egui::Rect::from_min_size(egui::Pos2::new(cursor_x, cursor_y + self.cell_height - 2.0), egui::Vec2::new(self.cell_width, 2.0)), 0.0, self.theme.cursor);
                        }
                        CursorShape::Bar => {
                            painter.rect_filled(egui::Rect::from_min_size(egui::Pos2::new(cursor_x, cursor_y), egui::Vec2::new(2.0, self.cell_height)), 0.0, self.theme.cursor);
                        }
                        CursorShape::Hidden => {}
                    }
                }

                // Draw IME preedit text at cursor position.
                // Outside the blink and visibility gates: the composition
                // window must stay visible while the block cursor is in its
                // off-phase, and even when the app inside hides the cursor.
                if self.ime_state.active && !self.ime_state.preedit.is_empty() {
                    let preedit_x = cursor_x + self.cell_width;
                    let preedit_y = cursor_y;
                    // String::len() is bytes; use display width so CJK preedit
                    // text does not get an over-wide highlight box.
                    let preedit_width =
                        unicode_width::UnicodeWidthStr::width(self.ime_state.preedit.as_str()) as f32;
                    let preedit_rect = egui::Rect::from_min_size(
                        egui::Pos2::new(preedit_x, preedit_y),
                        egui::Vec2::new(preedit_width * self.cell_width, self.cell_height)
                    );
                    // Draw preedit background
                    painter.rect_filled(preedit_rect, 0.0, egui::Color32::from_rgba_premultiplied(100, 100, 255, 80));
                    // Draw preedit text
                    painter.text(
                        egui::Pos2::new(preedit_x, preedit_y + self.cell_height * 0.1),
                        egui::Align2::LEFT_TOP,
                        &self.ime_state.preedit,
                        egui::FontId::monospace(self.font_size),
                        self.theme.fg
                    );
                }
            }

            // Scrollbar (right edge) — visible only when display_offset > 0
            // (user has scrolled back into history). total_lines() includes the
            // scrollback *capacity* (10k), so checking total > display_rows
            // would always be true and show the scrollbar at startup.
            {
                let offset = content.display_offset;

                if offset > 0 {
                    let total = self.panel.active_tab().unwrap().terminal.lock().unwrap().total_lines();
                    let track_x = rect.right() - 8.0;
                    let track_rect = egui::Rect::from_min_size(
                        egui::Pos2::new(track_x, rect.top()),
                        egui::Vec2::new(8.0, rect.height()),
                    );
                    // Track background
                    painter.rect_filled(track_rect, 0.0, egui::Color32::from_rgba_premultiplied(60, 60, 60, 180));

                    let max_offset = (total - display_rows as usize) as f32;
                    let thumb_h = (rect.height() * display_rows as f32 / total as f32).max(20.0);
                    let thumb_y = rect.top() + (rect.height() - thumb_h) * (1.0 - offset as f32 / max_offset);
                    let thumb_rect = egui::Rect::from_min_size(
                        egui::Pos2::new(track_x, thumb_y),
                        egui::Vec2::new(8.0, thumb_h),
                    );
                    painter.rect_filled(thumb_rect, 2.0, egui::Color32::from_rgba_premultiplied(160, 160, 160, 200));

                    // Scrollbar interaction
                    let pointer = ctx.input(|i| i.pointer.clone());
                    if self.scrollbar_dragging {
                        // Dragging thumb
                        if pointer.primary_down() {
                            if let Some(pos) = pointer.latest_pos() {
                                let rel = (pos.y - rect.top()) / rect.height();
                                let target_offset = ((1.0 - rel.clamp(0.0, 1.0)) * max_offset) as usize;
                                let delta = target_offset as i32 - offset as i32;
                                if delta != 0 {
                                    if let Some(tab) = self.panel.active_tab_mut() {
                                        tab.terminal.lock().unwrap().scroll_up(delta);
                                    }
                                }
                            }
                        } else {
                            self.scrollbar_dragging = false;
                        }
                    } else if pointer.primary_pressed() {
                        if let Some(pos) = pointer.press_origin() {
                            if track_rect.contains(pos) {
                                if thumb_rect.contains(pos) {
                                    self.scrollbar_dragging = true;
                                } else {
                                    // Click track = page scroll
                                    let page = display_rows as i32;
                                    if let Some(tab) = self.panel.active_tab_mut() {
                                        if pos.y < thumb_rect.top() {
                                            tab.terminal.lock().unwrap().scroll_up(page);
                                        } else {
                                            tab.terminal.lock().unwrap().scroll_up(-page);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Mouse interaction
            if response.hovered() {
                ctx.set_cursor_icon(egui::CursorIcon::Text);
                // Single click - clear selection and position cursor
                if response.clicked() {
                    response.request_focus();
                    self.cursor_blink.reset();

                    // Clear any existing selection when clicking
                    if let Some(tab) = self.panel.active_tab_mut() {
                        tab.terminal.lock().unwrap().clear_selection();
                    }

                    // Check for hyperlink click-to-open
                    if let Some(pos) = response.interact_pointer_pos() {
                        let point = self.pixel_to_point(pos, origin, content.display_offset);
                        let term = self.panel.active_tab().unwrap().terminal.lock().unwrap();
                        if let Some((url, is_url)) = term.find_hyperlink_at(point) {
                            if is_url {
                                hyperlinks::open_url(&url);
                            } else {
                                hyperlinks::open_path(&url, None, None);
                            }
                        }
                    }
                }
                // Double click - select word
                if response.double_clicked() {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let point = self.pixel_to_point(pos, origin, content.display_offset);
                        if let Some(tab) = self.panel.active_tab_mut() {
                            tab.terminal.lock().unwrap().start_selection(point, SelectionType::Semantic);
                            // Copy selected word to clipboard
                            let term = tab.terminal.lock().unwrap();
                            if let Some(text) = term.copy_selection() {
                                if !text.is_empty() {
                                    self.clipboard = Some(text.clone());
                                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                        let _ = clipboard.set_text(text);
                                    }
                                }
                            }
                        }
                    }
                }
                // Triple click - select line
                if response.triple_clicked() {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let point = self.pixel_to_point(pos, origin, content.display_offset);
                        if let Some(tab) = self.panel.active_tab_mut() {
                            tab.terminal.lock().unwrap().start_selection(point, SelectionType::Lines);
                            // Copy selected line to clipboard
                            let term = tab.terminal.lock().unwrap();
                            if let Some(text) = term.copy_selection() {
                                if !text.is_empty() {
                                    self.clipboard = Some(text.clone());
                                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                        let _ = clipboard.set_text(text);
                                    }
                                }
                            }
                        }
                    }
                }
                // Drag - select text
                if response.drag_started() {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let point = self.pixel_to_point(pos, origin, content.display_offset);
                        if let Some(tab) = self.panel.active_tab_mut() {
                            tab.selecting = true;
                            tab.terminal.lock().unwrap().start_selection(point, SelectionType::Simple);
                        }
                    }
                }
                if response.dragged() {
                    let selecting = self.panel.active_tab().map_or(false, |t| t.selecting);
                    if selecting {
                        if let Some(pos) = response.interact_pointer_pos() {
                            let point = self.pixel_to_point(pos, origin, content.display_offset);
                            if let Some(tab) = self.panel.active_tab_mut() {
                                tab.terminal.lock().unwrap().update_selection(point);
                            }
                        }
                    }
                }
                if response.drag_stopped() {
                    if let Some(tab) = self.panel.active_tab_mut() {
                        tab.selecting = false;
                        // Copy-on-select: automatically copy selected text
                        let term = tab.terminal.lock().unwrap();
                        if let Some(text) = term.copy_selection() {
                            if !text.is_empty() {
                                self.clipboard = Some(text.clone());
                                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                    let _ = clipboard.set_text(text);
                                }
                            }
                        }
                    }
                }
                // Right click = paste (like cmd.exe)
                if response.secondary_clicked() {
                    self.paste_clipboard();
                }
            }
        });

        // Save session periodically (thread-safe, no unsafe)
        static LAST_SAVE: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
        let last_save = LAST_SAVE.get_or_init(|| Mutex::new(None));
        if let Ok(mut guard) = last_save.lock() {
            if guard.map_or(true, |t| t.elapsed() > Duration::from_secs(30)) {
                self.save_session();
                *guard = Some(Instant::now());
            }
        }
    }
}
