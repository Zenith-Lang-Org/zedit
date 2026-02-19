mod buffer_state;
mod editing;
mod filetree_integration;
mod helpers;
mod palette;
mod prompt;
mod search;
mod selection;
mod view;
mod wrap;

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

    // File tree sidebar
    filetree: Option<crate::filetree::FileTree>,
    filetree_focused: bool,

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
            filetree: None,
            filetree_focused: false,
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
            filetree: None,
            filetree_focused: false,
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

    /// Width of the file tree sidebar (0 if not visible).
    fn sidebar_width(&self) -> u16 {
        self.filetree.as_ref().map_or(0, |ft| ft.width)
    }

    /// Resolve the layout tree for the current terminal size.
    fn resolve_layout(&mut self) {
        let (w, h) = self.terminal.size();
        let sidebar_w = self.sidebar_width();
        let pane_area_height = (h as usize).saturating_sub(self.status_height) as u16;
        let pane_area_width = w.saturating_sub(sidebar_w);
        self.layout.resolve(Rect {
            x: sidebar_w,
            y: 0,
            width: pane_area_width,
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
                self.recompute_all_wrap_maps();
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

        // When file tree is focused, route keys there first
        if self.filetree_focused
            && self.filetree.is_some()
            && let Event::Key(ref ke) = event
        {
            let ke_copy = ke.clone();
            if self.handle_filetree_key(ke_copy) {
                return;
            }
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
                if self.buf().wrap_map.is_some() {
                    self.move_cursor_up_visual();
                } else {
                    self.move_all_cursors(|c, buf| c.move_up(buf));
                }
            }
            (Key::Down, false, false) => {
                if self.buf().wrap_map.is_some() {
                    self.move_cursor_down_visual();
                } else {
                    self.move_all_cursors(|c, buf| c.move_down(buf));
                }
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
                if self.buf().wrap_map.is_some() {
                    self.move_cursor_home_visual();
                } else {
                    self.move_all_cursors(|c, buf| c.move_home(buf));
                }
            }
            (Key::End, false, false) => {
                if self.buf().wrap_map.is_some() {
                    self.move_cursor_end_visual();
                } else {
                    self.move_all_cursors(|c, buf| c.move_end(buf));
                }
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
                if self.buf().wrap_map.is_some() {
                    let h = self.text_area_height();
                    for _ in 0..h {
                        self.move_cursor_up_visual();
                    }
                } else {
                    let h = self.text_area_height();
                    let b = self.buf_mut();
                    b.scroll_row = b.scroll_row.saturating_sub(h);
                    b.cursors[b.primary].cursor.move_page_up(&b.buffer, h);
                }
            }
            (Key::PageDown, false, false) => {
                if self.buf().is_multi() {
                    self.buf_mut().collapse_to_primary();
                }
                if self.buf().wrap_map.is_some() {
                    let h = self.text_area_height();
                    for _ in 0..h {
                        self.move_cursor_down_visual();
                    }
                } else {
                    let h = self.text_area_height();
                    let b = self.buf_mut();
                    let max_line = b.buffer.line_count().saturating_sub(1);
                    b.scroll_row = (b.scroll_row + h).min(max_line);
                    b.cursors[b.primary].cursor.move_page_down(&b.buffer, h);
                }
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

            // -- Command Palette (Ctrl+P / Ctrl+Shift+P) --
            (Key::Char('p'), true, false) if !ke.shift => {
                self.palette = Some(palette::Palette::new());
            }

            // -- File tree toggle (Ctrl+B) --
            (Key::Char('b'), true, false) => {
                self.toggle_filetree();
            }

            // -- Word Wrap toggle (Alt+Z) --
            (Key::Char('z'), false, true) => {
                self.toggle_word_wrap();
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
        let sidebar_w = self.sidebar_width();
        let pane_area_height = (h as usize).saturating_sub(self.status_height) as u16;
        let total = Rect {
            x: sidebar_w,
            y: 0,
            width: w.saturating_sub(sidebar_w),
            height: pane_area_height,
        };
        self.layout.resize_split(self.active_pane, delta, total);
        self.resolve_layout();
    }

    fn resize_active_pane_vertical(&mut self, delta: i16) {
        // Vertical resize uses the same mechanism
        self.resize_active_pane(delta);
    }

    // -----------------------------------------------------------------------
    // Word wrap
    // -----------------------------------------------------------------------

    fn toggle_word_wrap(&mut self) {
        let buf_idx = self.active_buffer_index();
        if self.buffers[buf_idx].wrap_map.is_some() {
            // Disable wrap
            self.buffers[buf_idx].wrap_map = None;
            self.buffers[buf_idx].scroll_visual_offset = 0;
            self.set_message("Word wrap off", MessageType::Info);
        } else {
            // Enable wrap
            let wrap_col = self.wrap_col_for_buffer(buf_idx);
            let wm = wrap::WrapMap::new(&self.buffers[buf_idx].buffer, wrap_col);
            self.buffers[buf_idx].wrap_map = Some(wm);
            self.buffers[buf_idx].scroll_col = 0;
            self.buffers[buf_idx].scroll_visual_offset = 0;
            self.set_message("Word wrap on", MessageType::Info);
        }
    }

    /// Calculate the wrap column for a given buffer (pane width minus gutter).
    fn wrap_col_for_buffer(&self, buf_idx: usize) -> usize {
        let pane_width = if let Some(rect) = self.layout.pane_rect(self.active_pane) {
            rect.width as usize
        } else {
            self.screen.width()
        };
        pane_width.saturating_sub(self.buffers[buf_idx].gutter_width)
    }

    /// Move cursor up one visual row (accounting for wrapped segments).
    fn move_cursor_up_visual(&mut self) {
        let b = self.buf();
        let cursor_line = b.cursor().line;
        let cursor_col = b.cursor().col;
        let line_text = b.buffer.get_line(cursor_line).unwrap_or_default();

        let (visual_row, _visual_col) = if let Some(ref wm) = b.wrap_map {
            wm.buffer_to_visual(cursor_line, cursor_col, &line_text)
        } else {
            return;
        };

        if visual_row == 0 {
            return; // Already at top
        }

        let target_visual_row = visual_row - 1;
        let desired_display_col = b.cursor().desired_col;

        let (target_line, target_segment) = b
            .wrap_map
            .as_ref()
            .unwrap()
            .visual_to_buffer(target_visual_row);
        let target_line_text = b.buffer.get_line(target_line).unwrap_or_default();
        let (seg_start, seg_end) = b
            .wrap_map
            .as_ref()
            .unwrap()
            .segment_byte_range(target_line, target_segment);
        let seg_end_clamped = seg_end.min(target_line_text.len());
        let seg_text = &target_line_text[seg_start..seg_end_clamped];
        let byte_in_seg = display_col_to_byte_col(seg_text, desired_display_col);
        let new_col = seg_start + byte_in_seg;

        let b = self.buf_mut();
        b.cursors[b.primary].cursor.line = target_line;
        b.cursors[b.primary].cursor.col = new_col;
        // Preserve desired_col for vertical movement
        b.cursors[b.primary].cursor.clamp(&b.buffer);
    }

    /// Move cursor down one visual row (accounting for wrapped segments).
    fn move_cursor_down_visual(&mut self) {
        let b = self.buf();
        let cursor_line = b.cursor().line;
        let cursor_col = b.cursor().col;
        let line_text = b.buffer.get_line(cursor_line).unwrap_or_default();

        let (visual_row, _visual_col) = if let Some(ref wm) = b.wrap_map {
            wm.buffer_to_visual(cursor_line, cursor_col, &line_text)
        } else {
            return;
        };

        let total = b.wrap_map.as_ref().unwrap().total_visual_rows();
        if visual_row + 1 >= total {
            return; // Already at bottom
        }

        let target_visual_row = visual_row + 1;
        let desired_display_col = b.cursor().desired_col;

        let (target_line, target_segment) = b
            .wrap_map
            .as_ref()
            .unwrap()
            .visual_to_buffer(target_visual_row);
        let target_line_text = b.buffer.get_line(target_line).unwrap_or_default();
        let (seg_start, seg_end) = b
            .wrap_map
            .as_ref()
            .unwrap()
            .segment_byte_range(target_line, target_segment);
        let seg_end_clamped = seg_end.min(target_line_text.len());
        let seg_text = &target_line_text[seg_start..seg_end_clamped];
        let byte_in_seg = display_col_to_byte_col(seg_text, desired_display_col);
        let new_col = seg_start + byte_in_seg;

        let b = self.buf_mut();
        b.cursors[b.primary].cursor.line = target_line;
        b.cursors[b.primary].cursor.col = new_col;
        b.cursors[b.primary].cursor.clamp(&b.buffer);
    }

    /// Move cursor to start of current visual segment (Home in wrap mode).
    fn move_cursor_home_visual(&mut self) {
        let b = self.buf();
        let cursor_line = b.cursor().line;
        let cursor_col = b.cursor().col;
        let line_text = b.buffer.get_line(cursor_line).unwrap_or_default();

        let (visual_row, _) = if let Some(ref wm) = b.wrap_map {
            wm.buffer_to_visual(cursor_line, cursor_col, &line_text)
        } else {
            return;
        };

        let (target_line, target_segment) =
            b.wrap_map.as_ref().unwrap().visual_to_buffer(visual_row);
        let (seg_start, _) = b
            .wrap_map
            .as_ref()
            .unwrap()
            .segment_byte_range(target_line, target_segment);

        let b = self.buf_mut();
        b.cursors[b.primary].cursor.col = seg_start;
        b.cursors[b.primary].cursor.desired_col = seg_start;
    }

    /// Move cursor to end of current visual segment (End in wrap mode).
    fn move_cursor_end_visual(&mut self) {
        let b = self.buf();
        let cursor_line = b.cursor().line;
        let cursor_col = b.cursor().col;
        let line_text = b.buffer.get_line(cursor_line).unwrap_or_default();

        let (visual_row, _) = if let Some(ref wm) = b.wrap_map {
            wm.buffer_to_visual(cursor_line, cursor_col, &line_text)
        } else {
            return;
        };

        let (target_line, target_segment) =
            b.wrap_map.as_ref().unwrap().visual_to_buffer(visual_row);
        let target_line_text = b.buffer.get_line(target_line).unwrap_or_default();
        let (_, seg_end) = b
            .wrap_map
            .as_ref()
            .unwrap()
            .segment_byte_range(target_line, target_segment);
        let seg_end_clamped = seg_end.min(target_line_text.len());

        let b = self.buf_mut();
        b.cursors[b.primary].cursor.col = seg_end_clamped;
        b.cursors[b.primary].cursor.desired_col = seg_end_clamped;
        b.cursors[b.primary].cursor.clamp(&b.buffer);
    }

    /// Recompute wrap maps for all buffers that have wrapping enabled (e.g., after resize).
    fn recompute_all_wrap_maps(&mut self) {
        let panes: Vec<_> = self.layout.panes().to_vec();
        for pane_info in &panes {
            let bi = pane_info.buffer_index;
            if bi < self.buffers.len() {
                let bs = &mut self.buffers[bi];
                if let Some(ref mut wm) = bs.wrap_map {
                    let pane_w = pane_info.rect.width as usize;
                    let wrap_col = pane_w.saturating_sub(bs.gutter_width);
                    wm.rebuild_with_col(&bs.buffer, wrap_col);
                }
            }
        }
    }
}
