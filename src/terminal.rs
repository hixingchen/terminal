//! Core terminal entity - Simplified but complete implementation

use alacritty_terminal::{
    event::{Event as AlacEvent, EventListener},
    grid::{Dimensions, Scroll},
    sync::FairMutex,
    term::{Config, Term, TermMode},
    vte::ansi::{Color, Processor, StdSyncHandler},
};
use regex::Regex;
use std::sync::Arc;

/// Terminal bounds
#[derive(Debug, Clone, Copy)]
pub struct TerminalBounds {
    pub cell_width: f32,
    pub cell_height: f32,
    pub width: f32,
    pub height: f32,
}

impl TerminalBounds {
    pub fn num_lines(&self) -> usize {
        (self.height / self.cell_height) as usize
    }
    pub fn num_columns(&self) -> usize {
        (self.width / self.cell_width) as usize
    }
}

impl Default for TerminalBounds {
    fn default() -> Self {
        Self { cell_width: 8.0, cell_height: 16.0, width: 800.0, height: 400.0 }
    }
}

struct TermDimensions { columns: usize, screen_lines: usize }

impl Dimensions for TermDimensions {
    fn total_lines(&self) -> usize { self.screen_lines }
    fn screen_lines(&self) -> usize { self.screen_lines }
    fn columns(&self) -> usize { self.columns }
}

/// ANSI color
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnsiColor {
    Black, Red, Green, Yellow, Blue, Magenta, Cyan, White,
    BrightBlack, BrightRed, BrightGreen, BrightYellow, BrightBlue, BrightMagenta, BrightCyan, BrightWhite,
    Rgb(u8, u8, u8),
    Default,
}

impl Default for AnsiColor {
    fn default() -> Self { Self::Default }
}

impl AnsiColor {
    pub fn to_rgb(&self) -> (u8, u8, u8) {
        match self {
            Self::Black => (0, 0, 0),
            Self::Red => (205, 49, 49),
            Self::Green => (13, 188, 84),
            Self::Yellow => (229, 229, 16),
            Self::Blue => (36, 114, 200),
            Self::Magenta => (188, 63, 188),
            Self::Cyan => (17, 168, 205),
            Self::White => (229, 229, 229),
            Self::BrightBlack => (102, 102, 102),
            Self::BrightRed => (241, 76, 76),
            Self::BrightGreen => (35, 209, 139),
            Self::BrightYellow => (245, 245, 67),
            Self::BrightBlue => (64, 156, 255),
            Self::BrightMagenta => (214, 112, 214),
            Self::BrightCyan => (41, 184, 219),
            Self::BrightWhite => (255, 255, 255),
            Self::Rgb(r, g, b) => (*r, *g, *b),
            Self::Default => (204, 204, 204),
        }
    }
}

/// Convert alacritty Color to AnsiColor
fn convert_color(color: Color) -> AnsiColor {
    match color {
        Color::Named(named) => {
            use alacritty_terminal::vte::ansi::NamedColor;
            match named {
                NamedColor::Black => AnsiColor::Black,
                NamedColor::Red => AnsiColor::Red,
                NamedColor::Green => AnsiColor::Green,
                NamedColor::Yellow => AnsiColor::Yellow,
                NamedColor::Blue => AnsiColor::Blue,
                NamedColor::Magenta => AnsiColor::Magenta,
                NamedColor::Cyan => AnsiColor::Cyan,
                NamedColor::White => AnsiColor::White,
                NamedColor::BrightBlack => AnsiColor::BrightBlack,
                NamedColor::BrightRed => AnsiColor::BrightRed,
                NamedColor::BrightGreen => AnsiColor::BrightGreen,
                NamedColor::BrightYellow => AnsiColor::BrightYellow,
                NamedColor::BrightBlue => AnsiColor::BrightBlue,
                NamedColor::BrightMagenta => AnsiColor::BrightMagenta,
                NamedColor::BrightCyan => AnsiColor::BrightCyan,
                NamedColor::BrightWhite => AnsiColor::BrightWhite,
                _ => AnsiColor::Default,
            }
        }
        Color::Indexed(idx) => indexed_to_rgb(idx),
        Color::Spec(rgb) => AnsiColor::Rgb(rgb.r, rgb.g, rgb.b),
    }
}

fn indexed_to_rgb(idx: u8) -> AnsiColor {
    match idx {
        0 => AnsiColor::Black,
        1 => AnsiColor::Red,
        2 => AnsiColor::Green,
        3 => AnsiColor::Yellow,
        4 => AnsiColor::Blue,
        5 => AnsiColor::Magenta,
        6 => AnsiColor::Cyan,
        7 => AnsiColor::White,
        8 => AnsiColor::BrightBlack,
        9 => AnsiColor::BrightRed,
        10 => AnsiColor::BrightGreen,
        11 => AnsiColor::BrightYellow,
        12 => AnsiColor::BrightBlue,
        13 => AnsiColor::BrightMagenta,
        14 => AnsiColor::BrightCyan,
        15 => AnsiColor::BrightWhite,
        16..=231 => {
            let i = idx - 16;
            let r = i / 36;
            let g = (i % 36) / 6;
            let b = i % 6;
            AnsiColor::Rgb(
                if r == 0 { 0 } else { r * 40 + 55 },
                if g == 0 { 0 } else { g * 40 + 55 },
                if b == 0 { 0 } else { b * 40 + 55 },
            )
        }
        232..=255 => {
            let v = (idx - 232) * 10 + 8;
            AnsiColor::Rgb(v, v, v)
        }
    }
}

/// Cell flags
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CellFlags {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub inverse: bool,
    pub dim: bool,
}

/// Terminal cell
#[derive(Debug, Clone)]
pub struct Cell {
    pub character: char,
    pub foreground: AnsiColor,
    pub background: AnsiColor,
    pub flags: CellFlags,
    pub hyperlink: Option<String>,
    pub width: u8,  // 1 for normal, 2 for wide characters (CJK)
    pub zero_width: Option<Vec<char>>,  // Zero-width characters (combining marks, etc.)
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            character: ' ',
            foreground: AnsiColor::Default,
            background: AnsiColor::Default,
            flags: CellFlags::default(),
            hyperlink: None,
            width: 1,
            zero_width: None,
        }
    }
}

impl Cell {
    /// Check if character is wide (CJK, fullwidth, etc.)
    pub fn is_wide_char(c: char) -> bool {
        use unicode_width::UnicodeWidthChar;
        c.width().unwrap_or(0) == 2
    }
}

/// Indexed cell
#[derive(Debug, Clone)]
pub struct IndexedCell {
    pub point: Point,
    pub cell: Cell,
}

/// Grid point
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Point {
    pub line: i32,
    pub column: usize,
}

impl Point {
    pub fn new(line: i32, column: usize) -> Self { Self { line, column } }
}

/// Cursor shape
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum CursorShape { Block, Underline, Bar, Hidden }

impl Default for CursorShape {
    fn default() -> Self { Self::Block }
}

/// Terminal cursor
#[derive(Debug, Clone, Copy)]
pub struct Cursor {
    pub point: Point,
    pub visible: bool,
    pub shape: CursorShape,
}

/// Selection type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionType { Simple, Semantic, Lines }

/// Selection state
#[derive(Debug, Clone)]
pub struct Selection {
    pub start: Point,
    pub end: Point,
}

/// Selection range
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct SelectionRange {
    pub start: Point,
    pub end: Point,
}

/// Terminal modes
#[derive(Debug, Clone, Copy, Default)]
pub struct Modes(pub u32);

impl Modes {
    pub const NONE: Self = Self(0);
    pub const APP_CURSOR: Self = Self(1 << 0);
    pub const SHOW_CURSOR: Self = Self(1 << 1);
    pub const ALT_SCREEN: Self = Self(1 << 2);
    pub const VI: Self = Self(1 << 3);
    pub const BRACKETED_PASTE: Self = Self(1 << 4);
    pub const INSERT: Self = Self(1 << 5);

    pub fn contains(self, other: Self) -> bool { self.0 & other.0 == other.0 }
    pub fn insert(&mut self, other: Self) { self.0 |= other.0; }
}

/// Terminal content snapshot
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Content {
    pub cells: Vec<IndexedCell>,
    pub mode: Modes,
    pub cursor: Cursor,
    pub display_offset: usize,
    pub selection: Option<SelectionRange>,
    pub selection_text: Option<String>,
}

impl Default for Content {
    fn default() -> Self {
        Self {
            cells: Vec::new(),
            mode: Modes::NONE,
            cursor: Cursor { point: Point::new(0, 0), visible: true, shape: CursorShape::Block },
            display_offset: 0,
            selection: None,
            selection_text: None,
        }
    }
}

/// Real event listener that forwards alacritty events
pub struct ChannelListener {
    tx: std::sync::mpsc::Sender<AlacEvent>,
}

impl ChannelListener {
    pub fn new() -> (Self, std::sync::mpsc::Receiver<AlacEvent>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (Self { tx }, rx)
    }
}

impl EventListener for ChannelListener {
    fn send_event(&self, event: AlacEvent) {
        let _ = self.tx.send(event);
    }
}

/// Terminal event for GUI
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum TerminalEvent {
    Wakeup,
    TitleChanged(String),
    ResetTitle,
    Bell,
    Exit,
    ChildExit(i32),
    ColorRequest(usize),
}

/// Main Terminal entity
pub struct Terminal {
    term: Arc<FairMutex<Term<ChannelListener>>>,
    event_rx: Option<std::sync::mpsc::Receiver<AlacEvent>>,
    output_processor: Processor<StdSyncHandler>,
    last_content: Content,
    bounds: TerminalBounds,
    selection: Option<Selection>,
    url_regex: Regex,
    path_regex: Regex,
    /// Set once the cursor has reached the bottom row at least once,
    /// meaning real content has scrolled into the history buffer.
    scrollback_ready: bool,
}

impl Terminal {
    pub fn new(bounds: TerminalBounds) -> Self {
        let config = Config { scrolling_history: 10_000, ..Config::default() };
        let dimensions = TermDimensions {
            columns: bounds.num_columns(),
            screen_lines: bounds.num_lines(),
        };
        let (listener, event_rx) = ChannelListener::new();
        let term = Term::new(config, &dimensions, listener);

        Self {
            term: Arc::new(FairMutex::new(term)),
            event_rx: Some(event_rx),
            output_processor: Processor::<StdSyncHandler>::new(),
            last_content: Content::default(),
            bounds,
            selection: None,
            url_regex: Regex::new(r#"https?://[^\s<>\"{}|\\^`\[\]]+"#).unwrap(),
            path_regex: Regex::new(r#"([a-zA-Z]:\\[^\s<>:"|?*]+|/[^\s<>:"|?*]+\.\w+)(?::(\d+))?(?::(\d+))?"#).unwrap(),
            scrollback_ready: false,
        }
    }

    /// Drain pending events from alacritty
    pub fn drain_events(&mut self) -> Vec<TerminalEvent> {
        let mut events = Vec::new();
        if let Some(rx) = &self.event_rx {
            while let Ok(event) = rx.try_recv() {
                match event {
                    AlacEvent::Wakeup => events.push(TerminalEvent::Wakeup),
                    AlacEvent::Title(title) => events.push(TerminalEvent::TitleChanged(title)),
                    AlacEvent::ResetTitle => events.push(TerminalEvent::ResetTitle),
                    AlacEvent::Bell => events.push(TerminalEvent::Bell),
                    AlacEvent::Exit => events.push(TerminalEvent::Exit),
                    AlacEvent::ChildExit(code) => {
                        events.push(TerminalEvent::ChildExit(code));
                    }
                    AlacEvent::ColorRequest(idx, _) => {
                        events.push(TerminalEvent::ColorRequest(idx));
                    }
                    _ => {}
                }
            }
        }
        events
    }

    pub fn write_output(&mut self, bytes: &[u8]) {
        {
            let mut term = self.term.lock();
            self.output_processor.advance(&mut *term, bytes);
        }
        self.update_content();
        // Once the cursor reaches the bottom row, real content will start
        // scrolling into the history buffer on subsequent output.
        if !self.scrollback_ready {
            let screen_lines = self.bounds.num_lines() as i32;
            if self.last_content.cursor.point.line >= screen_lines - 1 {
                self.scrollback_ready = true;
            }
        }
    }

    fn update_content(&mut self) {
        let term = self.term.lock();
        let content = term.renderable_content();

        let mut cells = Vec::new();
        for cell in content.display_iter {
            let alac_cell = cell.cell;
            // Skip the continuation cell of a wide (CJK) character. The wide
            // cell itself occupies both columns; rendering this spacer as a
            // normal space would shift every following glyph by one cell and
            // add spurious spaces to search/copy results.
            if alac_cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            let fg = convert_color(alac_cell.fg);
            let bg = convert_color(alac_cell.bg);
            let hyperlink = alac_cell.hyperlink().map(|h| h.uri().to_string());

            let mut flags = CellFlags::default();
            use alacritty_terminal::term::cell::Flags;
            if alac_cell.flags.contains(Flags::BOLD) { flags.bold = true; }
            if alac_cell.flags.contains(Flags::ITALIC) { flags.italic = true; }
            if alac_cell.flags.contains(Flags::UNDERLINE) { flags.underline = true; }
            if alac_cell.flags.contains(Flags::STRIKEOUT) { flags.strikethrough = true; }
            if alac_cell.flags.contains(Flags::INVERSE) { flags.inverse = true; }
            if alac_cell.flags.contains(Flags::DIM) { flags.dim = true; }

            // Calculate cell width for CJK characters
            let width = if Cell::is_wide_char(alac_cell.c) { 2 } else { 1 };

            // Get zero-width characters (combining marks)
            let zero_width = alac_cell.zerowidth().map(|zw| zw.to_vec());
            let zero_width = if zero_width.as_ref().map_or(false, |v| v.is_empty()) {
                None
            } else {
                zero_width
            };

            cells.push(IndexedCell {
                point: Point::new(cell.point.line.0, cell.point.column.0),
                cell: Cell {
                    character: alac_cell.c,
                    foreground: fg,
                    background: bg,
                    flags,
                    hyperlink,
                    width,
                    zero_width,
                },
            });
        }

        let cursor = Cursor {
            point: Point::new(content.cursor.point.line.0, content.cursor.point.column.0),
            visible: term.mode().contains(TermMode::SHOW_CURSOR),
            shape: CursorShape::Block,
        };

        let mut modes = Modes::NONE;
        let term_mode = term.mode();
        if term_mode.contains(TermMode::APP_CURSOR) { modes.insert(Modes::APP_CURSOR); }
        if term_mode.contains(TermMode::SHOW_CURSOR) { modes.insert(Modes::SHOW_CURSOR); }
        if term_mode.contains(TermMode::ALT_SCREEN) { modes.insert(Modes::ALT_SCREEN); }
        if term_mode.contains(TermMode::VI) { modes.insert(Modes::VI); }
        if term_mode.contains(TermMode::BRACKETED_PASTE) { modes.insert(Modes::BRACKETED_PASTE); }
        if term_mode.contains(TermMode::INSERT) { modes.insert(Modes::INSERT); }

        let selection_range = self.selection.as_ref().map(|sel| {
            let (start, end) = if sel.start <= sel.end { (sel.start, sel.end) } else { (sel.end, sel.start) };
            SelectionRange { start, end }
        });

        let selection_text = selection_range.as_ref().map(|range| {
            cells.iter()
                .filter(|c| c.point >= range.start && c.point <= range.end)
                .map(|c| c.cell.character)
                .collect()
        });

        self.last_content = Content {
            cells, mode: modes, cursor,
            display_offset: content.display_offset,
            selection: selection_range, selection_text,
        };
    }

    pub fn get_content(&self) -> &Content { &self.last_content }

    pub fn scroll_up(&mut self, lines: i32) {
        // Don't scroll up if no content has ever scrolled into history.
        if lines > 0 && !self.scrollback_ready {
            return;
        }
        { let mut term = self.term.lock(); term.scroll_display(Scroll::Delta(lines)); }
        self.update_content();
    }

    pub fn resize(&mut self, bounds: TerminalBounds) {
        self.bounds = bounds;
        let dimensions = TermDimensions { columns: bounds.num_columns(), screen_lines: bounds.num_lines() };
        { let mut term = self.term.lock(); term.resize(dimensions); }
        self.update_content();
    }

    pub fn start_selection(&mut self, point: Point, _ty: SelectionType) {
        match _ty {
            SelectionType::Semantic => {
                // Word boundary selection (double-click)
                let (start, end) = self.find_word_boundaries(point);
                self.selection = Some(Selection {
                    start,
                    end,
                });
            }
            SelectionType::Lines => {
                // Line selection (triple-click)
                let content = self.get_content();
                let line = point.line;
                let first_col = content.cells.iter()
                    .find(|c| c.point.line == line)
                    .map(|c| c.point.column)
                    .unwrap_or(0);
                let last_col = content.cells.iter()
                    .filter(|c| c.point.line == line)
                    .last()
                    .map(|c| c.point.column)
                    .unwrap_or(0);
                self.selection = Some(Selection {
                    start: Point::new(line, first_col),
                    end: Point::new(line, last_col),
                });
            }
            _ => {
                self.selection = Some(Selection { start: point, end: point });
            }
        }
        self.update_content();
    }

    /// Find word boundaries for semantic selection
    fn find_word_boundaries(&self, point: Point) -> (Point, Point) {
        let content = self.get_content();
        let line = point.line;
        let col = point.column;

        // Get characters on the same line
        let line_cells: Vec<&IndexedCell> = content.cells.iter()
            .filter(|c| c.point.line == line)
            .collect();

        if line_cells.is_empty() {
            return (point, point);
        }

        // Find word character at position
        let is_word_char = |c: char| c.is_alphanumeric() || c == '_';

        // Find start of word
        let mut start_col = col;
        while start_col > 0 {
            if let Some(cell) = line_cells.iter().find(|c| c.point.column == start_col - 1) {
                if is_word_char(cell.cell.character) {
                    start_col -= 1;
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        // Find end of word
        let mut end_col = col;
        loop {
            if let Some(cell) = line_cells.iter().find(|c| c.point.column == end_col) {
                if is_word_char(cell.cell.character) {
                    end_col += 1;
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        // If no word found, select non-space characters
        if start_col == end_col {
            start_col = col;
            end_col = col + 1;
        }

        (Point::new(line, start_col), Point::new(line, end_col.saturating_sub(1)))
    }

    pub fn update_selection(&mut self, point: Point) {
        if let Some(ref mut sel) = self.selection { sel.end = point; }
        self.update_content();
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
        self.update_content();
    }

    pub fn copy_selection(&self) -> Option<String> {
        self.last_content.selection_text.clone()
    }

    /// Total lines in the grid (visible + scrollback history).
    pub fn total_lines(&self) -> usize {
        self.term.lock().grid().total_lines()
    }

    pub fn find_hyperlink_at(&self, point: Point) -> Option<(String, bool)> {
        let content = self.get_content();
        let mut line_text = String::new();
        for cell in &content.cells {
            if cell.point.line == point.line {
                line_text.push(cell.cell.character);
            }
        }
        for mat in self.url_regex.find_iter(&line_text) {
            let col = point.column;
            if col >= mat.start() && col < mat.end() {
                return Some((mat.as_str().to_string(), true));
            }
        }
        for mat in self.path_regex.find_iter(&line_text) {
            let col = point.column;
            if col >= mat.start() && col < mat.end() {
                return Some((mat.as_str().to_string(), false));
            }
        }
        None
    }
}
