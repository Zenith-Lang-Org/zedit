mod buffer_state;
mod editing;
mod helpers;
mod palette;
mod prompt;
mod search;
mod selection;
mod view;

#[cfg(test)]
mod tests;

use std::path::Path;

use crate::config::Config;
use crate::input::{self, Event, Key, KeyEvent};
use crate::layout::{Direction, LayoutState, PaneId, Rect, SplitDir};
use crate::render::Screen;
use crate::terminal::{self, ColorMode, Terminal};
use crate::undo::CursorState;

use buffer_state::*;
use helpers::*;
use prompt::*;
use search::*;

// ---------------------------------------------------------------------------
// Message types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum MessageType {
    Info,
    Error,
    Warning,
}

// ---------------------------------------------------------------------------
// Clipboard — multi-line aware clipboard with line-mode support
// ---------------------------------------------------------------------------

pub(super) struct Clipboard {
    /// Each entry is one piece of copied text.
    /// Single copy → vec!["the text"].
    /// Multi-cursor copy → vec!["sel1", "sel2", ...].
    pub entries: Vec<String>,
    /// Whether this was a line-mode copy (e.g. Ctrl+C with no selection copies the whole line).
    /// When true, paste inserts above/below as a full line rather than inline.
    pub line_mode: bool,
}

impl Clipboard {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            line_mode: false,
        }
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty() || self.entries.iter().all(|e| e.is_empty())
    }

    /// Set clipboard to a single text entry (inline mode).
    fn set_text(&mut self, text: String) {
        self.entries = vec![text];
        self.line_mode = false;
    }

    /// Set clipboard to a single text entry in line mode.
    fn set_line(&mut self, text: String) {
        self.entries = vec![text];
        self.line_mode = true;
    }

    /// Get the combined text for pasting (joins all entries with newlines for multi-cursor).
    fn text(&self) -> String {
        self.entries.join("")
    }
}

// ---------------------------------------------------------------------------
// Editor
// ---------------------------------------------------------------------------

pub struct Editor {
    buffers: Vec<BufferState>,
    active_buffer: usize,

    terminal: Terminal,
    screen: Screen,
    color_mode: ColorMode,

    // User configuration
    config: Config,

    // UI layout
    status_height: usize,
    layout: LayoutState,
    active_pane: PaneId,

    // Transient message
    message: Option<String>,
    message_type: MessageType,

    // Quit state
    quit_confirm: bool,

    // Clipboard (shared across buffers)
    clipboard: Clipboard,

    // Active prompt (mini-prompt for Open, Save As, etc.)
    prompt: Option<Prompt>,

    // Mouse drag state
    mouse_dragging: bool,

    // Help overlay
    help_visible: bool,

    // Command palette
    palette: Option<palette::Palette>,

    running: bool,
}

impl Editor {
    /// Create a new editor with an empty buffer.
    pub fn new(config: Config) -> Result<Self, String> {
        let color_mode = terminal::detect_color_mode();
        let mut terminal = Terminal::new()?;
        let (w, h) = terminal.size();

        let line_numbers = config.line_numbers;
        let layout = LayoutState::new(0);
        let active_pane = layout.first_pane();
        Ok(Editor {
            buffers: vec![BufferState::new_empty(line_numbers)],
            active_buffer: 0,
            screen: Screen::new(w as usize, h as usize),
            terminal,
            color_mode,
            config,
            status_height: 2,
            layout,
            active_pane,
            message: None,
            message_type: MessageType::Info,
            quit_confirm: false,
            clipboard: Clipboard::new(),
            prompt: None,
            mouse_dragging: false,
            help_visible: false,
            palette: None,
            running: true,
        })
    }

    /// Create a new editor and load a file.
    pub fn open(path: &Path, config: Config) -> Result<Self, String> {
        let color_mode = terminal::detect_color_mode();
        let mut terminal = Terminal::new()?;
        let (w, h) = terminal.size();

        let bs =
            BufferState::from_file(path, config.line_numbers, &config.theme, &config.languages)?;

        let layout = LayoutState::new(0);
        let active_pane = layout.first_pane();
        Ok(Editor {
            buffers: vec![bs],
            active_buffer: 0,
            screen: Screen::new(w as usize, h as usize),
            terminal,
            color_mode,
            config,
            status_height: 2,
            layout,
            active_pane,
            message: None,
            message_type: MessageType::Info,
            quit_confirm: false,
            clipboard: Clipboard::new(),
            prompt: None,
            mouse_dragging: false,
            help_visible: false,
            palette: None,
            running: true,
        })
    }

    // -- Active buffer accessors --

    fn buf(&self) -> &BufferState {
        let idx = self
            .layout
            .pane_buffer(self.active_pane)
            .unwrap_or(self.active_buffer);
        &self.buffers[idx]
    }

    fn buf_mut(&mut self) -> &mut BufferState {
        let idx = self
            .layout
            .pane_buffer(self.active_pane)
            .unwrap_or(self.active_buffer);
        &mut self.buffers[idx]
    }

    /// Get the buffer index for the active pane.
    fn active_buffer_index(&self) -> usize {
        self.layout
            .pane_buffer(self.active_pane)
            .unwrap_or(self.active_buffer)
    }

    pub(super) fn config(&self) -> &Config {
        &self.config
    }

    /// Resolve the layout tree for the current terminal size.
    fn resolve_layout(&mut self) {
        let (w, h) = self.terminal.size();
        let pane_area_height = (h as usize).saturating_sub(self.status_height) as u16;
        self.layout.resolve(Rect {
            x: 0,
            y: 0,
            width: w,
            height: pane_area_height,
        });
    }

    /// Run the main editor loop.
    pub fn run(&mut self) -> Result<(), String> {
        self.resolve_layout();
        while self.running {
            // 1. Check for resize
            if self.terminal.check_resize() {
                let (w, h) = self.terminal.size();
                self.screen.resize(w as usize, h as usize);
                self.resolve_layout();
                self.adjust_viewport();
            }

            // 2. Render
            self.render();

            // 3. Read event (blocks until input or timeout)
            let event = input::read_event(&self.terminal);

            // 4. Handle event
            if event != Event::None {
                self.handle_event(event);
            }
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Event handling
    // -----------------------------------------------------------------------

    fn handle_event(&mut self, event: Event) {
        // When help overlay is visible, only process dismiss keys
        if self.help_visible {
            if let Event::Key(ke) = event {
                match &ke.key {
                    Key::F(1) | Key::Escape | Key::Char('q') => {
                        self.help_visible = false;
                    }
                    _ => {}
                }
            }
            return;
        }

        // When command palette is open, route all keys to it
        if self.palette.is_some() {
            if let Event::Key(ke) = event {
                self.handle_palette_key(ke);
            }
            return;
        }

        // Clear message on any event, but only when no prompt is active
        if self.prompt.is_none() && !matches!(&event, Event::None) {
            self.message = None;
        }

        match event {
            Event::Key(ke) => {
                if self.prompt.is_some() {
                    self.handle_prompt_key(ke);
                } else {
                    self.handle_key(ke);
                }
            }
            Event::Mouse(me) => {
                if self.prompt.is_none() {
                    self.handle_mouse(me);
                }
            }
            Event::Paste(text) => {
                if self.prompt.is_some() {
                    // Insert pasted text into prompt input
                    if let Some(ref mut prompt) = self.prompt {
                        prompt.input.insert_str(prompt.cursor_pos, &text);
                        prompt.cursor_pos += text.len();
                    }
                } else {
                    self.delete_selection();
                    self.handle_paste(&text);
                }
            }
            Event::None => {}
        }
    }

    fn handle_key(&mut self, ke: KeyEvent) {
        // Reset quit confirmation on any key that isn't Ctrl+Q
        if !(ke.ctrl && ke.key == Key::Char('q')) {
            self.quit_confirm = false;
        }

        let is_nav = matches!(
            &ke.key,
            Key::Up
                | Key::Down
                | Key::Left
                | Key::Right
                | Key::Home
                | Key::End
                | Key::PageUp
                | Key::PageDown
        );

        // Before navigation: start/continue selection if shift is held
        if is_nav && ke.shift {
            self.start_or_continue_selection();
        }

        match (&ke.key, ke.ctrl, ke.alt) {
            // -- Navigation (works with and without shift) --
            (Key::Up, false, false) => {
                self.move_all_cursors(|c, buf| c.move_up(buf));
            }
            (Key::Down, false, false) => {
                self.move_all_cursors(|c, buf| c.move_down(buf));
            }
            (Key::Left, false, false) => {
                self.move_all_cursors(|c, buf| c.move_left(buf));
            }
            (Key::Right, false, false) => {
                self.move_all_cursors(|c, buf| c.move_right(buf));
            }

            (Key::Left, true, false) => {
                self.move_all_cursors(|c, buf| c.move_word_left(buf));
            }
            (Key::Right, true, false) => {
                self.move_all_cursors(|c, buf| c.move_word_right(buf));
            }

            (Key::Home, false, false) => {
                self.move_all_cursors(|c, buf| c.move_home(buf));
            }
            (Key::End, false, false) => {
                self.move_all_cursors(|c, buf| c.move_end(buf));
            }

            (Key::Home, true, false) => {
                // Ctrl+Home: collapse to single cursor at start
                if self.buf().is_multi() {
                    self.buf_mut().collapse_to_primary();
                }
                self.buf_mut().cursor_mut().move_to_start();
            }
            (Key::End, true, false) => {
                if self.buf().is_multi() {
                    self.buf_mut().collapse_to_primary();
                }
                let b = self.buf_mut();
                b.cursors[b.primary].cursor.move_to_end(&b.buffer);
            }

            (Key::PageUp, false, false) => {
                if self.buf().is_multi() {
                    self.buf_mut().collapse_to_primary();
                }
                let h = self.text_area_height();
                let b = self.buf_mut();
                b.scroll_row = b.scroll_row.saturating_sub(h);
                b.cursors[b.primary].cursor.move_page_up(&b.buffer, h);
            }
            (Key::PageDown, false, false) => {
                if self.buf().is_multi() {
                    self.buf_mut().collapse_to_primary();
                }
                let h = self.text_area_height();
                let b = self.buf_mut();
                let max_line = b.buffer.line_count().saturating_sub(1);
                b.scroll_row = (b.scroll_row + h).min(max_line);
                b.cursors[b.primary].cursor.move_page_down(&b.buffer, h);
            }

            // -- Editing (delete selection first if active) --
            (Key::Char(ch), false, false) => {
                if self.buf().is_multi() {
                    self.insert_char_multi(*ch);
                } else {
                    self.delete_selection();
                    self.insert_char(*ch);
                }
            }
            (Key::Enter, false, false) => {
                if self.buf().is_multi() {
                    self.insert_newline_multi();
                } else {
                    self.delete_selection();
                    self.insert_newline();
                }
            }
            (Key::Tab, false, false) => {
                if self.buf().is_multi() {
                    self.insert_tab_multi();
                } else {
                    self.delete_selection();
                    self.insert_tab();
                }
            }
            (Key::BackTab, false, _) => {
                self.unindent();
            }
            (Key::Backspace, false, false) => {
                if self.buf().is_multi() {
                    self.backspace_multi();
                } else if self.delete_selection().is_none() {
                    self.backspace();
                }
            }
            (Key::Delete, false, false) => {
                if self.buf().is_multi() {
                    self.delete_at_multi();
                } else if self.delete_selection().is_none() {
                    self.delete_at_cursor();
                }
            }

            // -- Clipboard --
            (Key::Char('c'), true, false) => self.copy_selection(),
            (Key::Char('x'), true, false) => self.cut_selection(),
            (Key::Char('v'), true, false) => self.paste_clipboard(),
            (Key::Char('a'), true, false) => self.select_all(),

            // -- Commands --
            (Key::Char('s'), true, false) if !ke.shift => self.save(),
            (Key::Char('S'), true, false) => {
                // Ctrl+Shift+S → Save As
                self.start_prompt("Save as: ", PromptAction::SaveAs);
            }
            (Key::Char('q'), true, false) => self.quit(),

            // -- Select next occurrence (Ctrl+D) --
            (Key::Char('d'), true, false) if !ke.shift => self.select_next_occurrence(),
            // -- Duplicate line (Ctrl+Shift+D) --
            (Key::Char('D'), true, false) => self.duplicate_line(),

            // -- Select all occurrences (Ctrl+Shift+L) --
            (Key::Char('L'), true, false) => self.select_all_occurrences(),

            // -- Delete line (Ctrl+Shift+K) --
            (Key::Char('K'), true, false) => self.delete_line(),

            // -- Select line (Ctrl+L) --
            (Key::Char('l'), true, false) => self.select_line(),

            // -- Go to line (Ctrl+G) --
            (Key::Char('g'), true, false) => {
                self.start_prompt("Go to line: ", PromptAction::GoToLine);
            }

            // -- Toggle comment (Ctrl+/) --
            (Key::Char('/'), true, false) => self.toggle_comment(),

            // -- Multi-buffer --
            (Key::Char('n'), true, false) => self.new_buffer(),
            (Key::Char('w'), true, false) => self.close_buffer(),
            (Key::PageDown, true, false) => self.next_buffer(),
            (Key::PageUp, true, false) => self.prev_buffer(),

            // -- Undo/Redo --
            (Key::Char('z'), true, false) => self.do_undo(),
            (Key::Char('y'), true, false) => self.do_redo(),

            // -- Search --
            (Key::Char('f'), true, false) => {
                self.open_find_prompt(PromptAction::Find);
            }
            (Key::Char('h'), true, false) => {
                self.open_find_prompt(PromptAction::Replace);
            }
            (Key::F(3), false, false) if !ke.shift => {
                self.search_next();
            }
            (Key::F(3), false, false) if ke.shift => {
                self.search_prev();
            }

            // -- File --
            (Key::Char('o'), true, false) => {
                self.start_prompt("Open: ", PromptAction::OpenFile);
            }

            // -- Escape: collapse multi-cursor or cancel --
            (Key::Escape, false, false) => {
                if self.buf().is_multi() {
                    self.buf_mut().collapse_to_primary();
                    self.set_message("Single cursor", MessageType::Info);
                }
            }

            // -- Pane operations --
            // Ctrl+\ — split horizontally (left|right)
            (Key::Char('\\'), true, false) if !ke.shift => {
                self.split_pane_horizontal();
            }
            // Ctrl+Shift+\ — split vertically (top|bottom)
            (Key::Char('\\'), true, false) if ke.shift => {
                self.split_pane_vertical();
            }
            // Ctrl+Shift+W — close active pane
            (Key::Char('W'), true, false) => {
                self.close_active_pane();
            }

            // Alt+Arrow — move focus to adjacent pane
            (Key::Left, false, true) if !ke.shift => {
                self.focus_pane(Direction::Left);
            }
            (Key::Right, false, true) if !ke.shift => {
                self.focus_pane(Direction::Right);
            }
            (Key::Up, false, true) if !ke.shift => {
                self.focus_pane(Direction::Up);
            }
            (Key::Down, false, true) if !ke.shift => {
                self.focus_pane(Direction::Down);
            }

            // Alt+Shift+Arrow — resize active pane
            (Key::Left, false, true) if ke.shift => {
                self.resize_active_pane(-2);
            }
            (Key::Right, false, true) if ke.shift => {
                self.resize_active_pane(2);
            }
            (Key::Up, false, true) if ke.shift => {
                self.resize_active_pane_vertical(-2);
            }
            (Key::Down, false, true) if ke.shift => {
                self.resize_active_pane_vertical(2);
            }

            // -- Command Palette --
            (Key::Char('P'), true, false) => {
                self.palette = Some(palette::Palette::new());
            }

            // -- Help --
            (Key::F(1), false, false) => {
                self.help_visible = !self.help_visible;
            }

            _ => {}
        }

        // After navigation: extend or clear selection
        if is_nav {
            if ke.shift {
                self.extend_selection();
            } else if !self.buf().is_multi() {
                // Clear selection on nav without shift (single cursor only)
                self.buf_mut().set_selection(None);
            } else {
                // Multi-cursor: clear all selections on nav without shift
                for cs in &mut self.buf_mut().cursors {
                    cs.selection = None;
                }
                self.buf_mut().sort_and_merge();
            }
        }
    }

    // -----------------------------------------------------------------------
    // Multi-cursor helpers
    // -----------------------------------------------------------------------

    /// Apply a movement function to all cursors.
    fn move_all_cursors<F>(&mut self, f: F)
    where
        F: Fn(&mut crate::cursor::Cursor, &crate::buffer::Buffer),
    {
        let b = self.buf_mut();
        for cs in &mut b.cursors {
            f(&mut cs.cursor, &b.buffer);
        }
    }

    // -----------------------------------------------------------------------
    // Messages
    // -----------------------------------------------------------------------

    fn set_message(&mut self, msg: &str, msg_type: MessageType) {
        self.message = Some(msg.to_string());
        self.message_type = msg_type;
    }

    // -----------------------------------------------------------------------
    // Pane operations
    // -----------------------------------------------------------------------

    fn split_pane_horizontal(&mut self) {
        let buf_idx = self.active_buffer_index();
        if let Some(new_id) =
            self.layout
                .split_pane(self.active_pane, SplitDir::Horizontal, buf_idx)
        {
            self.resolve_layout();
            self.active_pane = new_id;
            self.active_buffer = self.active_buffer_index();
            self.set_message("Split horizontal", MessageType::Info);
        }
    }

    fn split_pane_vertical(&mut self) {
        let buf_idx = self.active_buffer_index();
        if let Some(new_id) = self
            .layout
            .split_pane(self.active_pane, SplitDir::Vertical, buf_idx)
        {
            self.resolve_layout();
            self.active_pane = new_id;
            self.active_buffer = self.active_buffer_index();
            self.set_message("Split vertical", MessageType::Info);
        }
    }

    fn close_active_pane(&mut self) {
        if self.layout.pane_count() <= 1 {
            self.set_message("Only one pane", MessageType::Warning);
            return;
        }
        // Find a neighbor to move focus to before closing
        let next = self
            .layout
            .adjacent_pane(self.active_pane, Direction::Left)
            .or_else(|| {
                self.layout
                    .adjacent_pane(self.active_pane, Direction::Right)
            })
            .or_else(|| self.layout.adjacent_pane(self.active_pane, Direction::Up))
            .or_else(|| self.layout.adjacent_pane(self.active_pane, Direction::Down))
            .unwrap_or(self.layout.first_pane());
        self.layout.close_pane(self.active_pane);
        self.active_pane = next;
        self.active_buffer = self.active_buffer_index();
        self.resolve_layout();
        self.set_message("Pane closed", MessageType::Info);
    }

    fn focus_pane(&mut self, dir: Direction) {
        if let Some(target) = self.layout.adjacent_pane(self.active_pane, dir) {
            self.active_pane = target;
            self.active_buffer = self.active_buffer_index();
        }
    }

    fn resize_active_pane(&mut self, delta: i16) {
        let (w, h) = self.terminal.size();
        let pane_area_height = (h as usize).saturating_sub(self.status_height) as u16;
        let total = Rect {
            x: 0,
            y: 0,
            width: w,
            height: pane_area_height,
        };
        self.layout.resize_split(self.active_pane, delta, total);
        self.resolve_layout();
    }

    fn resize_active_pane_vertical(&mut self, delta: i16) {
        // Vertical resize uses the same mechanism
        self.resize_active_pane(delta);
    }
}
