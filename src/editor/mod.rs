mod buffer_state;
mod completion;
mod diff_integration;
mod editing;
mod filetree_integration;
mod helpers;
mod hover;
mod minimap;
mod palette;
mod prompt;
mod search;
mod selection;
pub mod tasks;
mod view;
mod wrap;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::input::{self, Event, Key, KeyEvent};
use crate::keybindings::EditorAction;
use crate::layout::{Direction, LayoutState, PaneContent, PaneId, Rect, SplitDir};
use crate::pty::{self, POLLIN, PollFd, Pty};
use crate::render::Screen;
use crate::session;
use crate::swap;
use crate::terminal::{self, ColorMode, Terminal};
use crate::undo::CursorState;
use crate::vterm::VTerm;

use buffer_state::*;
use helpers::*;
use prompt::*;
use search::*;

use crate::diff_view;
use crate::lsp;
use crate::plugin;

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
// File change detection
// ---------------------------------------------------------------------------

struct FileChangeNotice {
    buf_idx: usize,
    buffer_modified: bool, // true → user has unsaved edits
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
// Bottom panel tab
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BottomTab {
    Terminal,
    /// LSP diagnostics for the active buffer (rust-analyzer, real-time).
    Diagnostics,
    /// cargo check results for the whole workspace (background run).
    Problems,
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

    // Persistent arboard instance — must stay alive for X11 clipboard ownership.
    // On X11, dropping the Clipboard object releases selection ownership and clears
    // the clipboard; keeping it here ensures content survives until the next copy.
    sys_clipboard: Option<arboard::Clipboard>,

    // Active prompt (mini-prompt for Open, Save As, etc.)
    prompt: Option<Prompt>,

    // Mouse drag state
    mouse_dragging: bool,

    // When true, render() skips adjust_viewport() so scroll-only actions
    // (Ctrl+Up/Down) don't get immediately cancelled. Cleared on any non-scroll key.
    scroll_only_mode: bool,

    // Help overlay
    help_visible: bool,

    // Command palette
    palette: Option<palette::Palette>,

    // File tree sidebar
    filetree: Option<crate::filetree::FileTree>,
    filetree_focused: bool,

    // Integrated terminal
    pub(super) vterms: Vec<VTerm>,
    pub(super) ptys: Vec<Pty>,
    terminal_panel_pane: Option<PaneId>,
    terminal_idx: Option<usize>,
    bottom_tab: BottomTab, // controls which tab is active in the bottom panel

    // LSP support
    lsp_manager: Option<lsp::LspManager>,
    /// Generation counter: incremented when the LSP diagnostics change.
    /// Used to skip rebuilding problem_panel.lsp_items on frames with no change.
    lsp_diag_gen: u64,
    completion_menu: Option<completion::CompletionMenu>,
    hover_popup: Option<hover::HoverPopup>,

    // Plugin system
    plugin_manager: Option<plugin::PluginManager>,

    // Diff/Merge view overlay
    diff_view: Option<diff_view::DiffView>,

    // Minimap sidebar
    minimap: minimap::Minimap,

    // Swap file timer
    swap_timer: std::time::Instant,
    swap_interval_ms: u64,

    // Disk-change detection timer and pending notice
    disk_check_timer: std::time::Instant,
    file_change_notice: Option<FileChangeNotice>,

    // Stable counter for untitled buffer IDs (NewBuffer01, NewBuffer02, ...)
    next_untitled_id: usize,

    // Tab bar
    tab_bar_height: usize,
    tab_regions: Vec<(usize, usize, usize)>, // (start_col, end_col, buf_idx)
    tab_scroll_offset: usize,

    running: bool,

    // Task runner state
    last_task: Option<String>,     // last command sent (for re-run)
    task_language: Option<String>, // language of last task

    // Problem panel
    problem_panel: crate::problem_panel::ProblemPanel,
    /// Background cargo check — receiver delivers full output when done.
    bg_cargo_rx: Option<std::sync::mpsc::Receiver<String>>,

    // Diagnostics panel navigation (DIAGNOSTICS tab)
    diag_panel_selected: usize,
    diag_panel_scroll: usize,

    // Terminal window title — cached to avoid emitting OSC on every frame.
    last_title: String,
}

impl Editor {
    /// Build a PluginManager: discover and launch all installed plugins.
    fn build_plugin_manager() -> Option<plugin::PluginManager> {
        let mut mgr = plugin::PluginManager::new();
        mgr.discover();
        if mgr.discovered.is_empty() {
            return None;
        }
        mgr.launch_all();
        Some(mgr)
    }

    /// Build an LspManager from config (if any LSP servers are configured).
    fn build_lsp_manager(config: &Config) -> Option<lsp::LspManager> {
        if config.lsp_servers.is_empty() {
            return None;
        }
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let root_dir = cwd.to_string_lossy().to_string();
        let lsp_config: Vec<(String, lsp::LspServerConfig)> = config
            .lsp_servers
            .iter()
            .map(|(lang, cfg)| {
                (
                    lang.clone(),
                    lsp::LspServerConfig {
                        command: cfg.command.clone(),
                        args: cfg.args.clone(),
                    },
                )
            })
            .collect();
        Some(lsp::LspManager::new(lsp_config, &root_dir))
    }

    /// Create a new editor with an empty buffer.
    pub fn new(config: Config) -> Result<Self, String> {
        let color_mode = terminal::detect_color_mode();
        let mut terminal = Terminal::new()?;
        let (w, h) = terminal.size();

        let line_numbers = config.line_numbers;
        let lsp_manager = Self::build_lsp_manager(&config);
        let mut initial_buf = BufferState::new_empty(line_numbers);
        initial_buf.untitled_id = Some(1);
        let layout = LayoutState::new(0);
        let active_pane = layout.first_pane();
        Ok(Editor {
            buffers: vec![initial_buf],
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
            sys_clipboard: arboard::Clipboard::new().ok(),
            prompt: None,
            mouse_dragging: false,
            scroll_only_mode: false,
            help_visible: false,
            palette: None,
            filetree: None,
            filetree_focused: false,
            vterms: Vec::new(),
            ptys: Vec::new(),
            terminal_panel_pane: None,
            terminal_idx: None,
            bottom_tab: BottomTab::Terminal,
            lsp_manager,
            lsp_diag_gen: 0,
            completion_menu: None,
            hover_popup: None,
            plugin_manager: Self::build_plugin_manager(),
            diff_view: None,
            minimap: minimap::Minimap::new(),
            swap_timer: std::time::Instant::now(),
            swap_interval_ms: 2000,
            disk_check_timer: std::time::Instant::now(),
            file_change_notice: None,
            next_untitled_id: 2,
            tab_bar_height: 1,
            tab_regions: Vec::new(),
            tab_scroll_offset: 0,
            running: true,
            last_task: None,
            task_language: None,
            problem_panel: crate::problem_panel::ProblemPanel::new(),
            bg_cargo_rx: None,
            diag_panel_selected: 0,
            diag_panel_scroll: 0,
            last_title: String::new(),
        })
    }

    /// Create a new editor and load a file.
    pub fn open(path: &Path, config: Config) -> Result<Self, String> {
        let color_mode = terminal::detect_color_mode();
        let mut terminal = Terminal::new()?;
        let (w, h) = terminal.size();

        let bs =
            BufferState::from_file(path, config.line_numbers, &config.theme, &config.languages)?;

        let lsp_manager = Self::build_lsp_manager(&config);
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
            sys_clipboard: arboard::Clipboard::new().ok(),
            prompt: None,
            mouse_dragging: false,
            scroll_only_mode: false,
            help_visible: false,
            palette: None,
            filetree: None,
            filetree_focused: false,
            vterms: Vec::new(),
            ptys: Vec::new(),
            terminal_panel_pane: None,
            terminal_idx: None,
            bottom_tab: BottomTab::Terminal,
            lsp_manager,
            lsp_diag_gen: 0,
            completion_menu: None,
            hover_popup: None,
            plugin_manager: Self::build_plugin_manager(),
            diff_view: None,
            minimap: minimap::Minimap::new(),
            swap_timer: std::time::Instant::now(),
            swap_interval_ms: 2000,
            disk_check_timer: std::time::Instant::now(),
            file_change_notice: None,
            next_untitled_id: 1,
            tab_bar_height: 1,
            tab_regions: Vec::new(),
            tab_scroll_offset: 0,
            running: true,
            last_task: None,
            task_language: None,
            problem_panel: crate::problem_panel::ProblemPanel::new(),
            bg_cargo_rx: None,
            diag_panel_selected: 0,
            diag_panel_scroll: 0,
            last_title: String::new(),
        })
    }

    /// Restore editor from a saved session.
    pub fn restore_session(sess: session::Session, config: Config) -> Result<Self, String> {
        let color_mode = terminal::detect_color_mode();
        let mut terminal = Terminal::new()?;
        let (w, h) = terminal.size();

        let line_numbers = config.line_numbers;
        let mut buffers = Vec::new();
        let mut recovery_msgs = Vec::new();

        for bs in &sess.buffers {
            if let Some(ref file_path) = bs.file_path {
                let path = Path::new(file_path);
                if path.exists() {
                    // Check for orphaned swap
                    let swap_status = swap::check_swap(path);
                    if swap_status == swap::SwapStatus::Orphaned {
                        // Recover from swap
                        let swp = swap::swap_path(path);
                        if let Ok((_header, content)) = swap::read_swap(&swp) {
                            let mut buf_state = BufferState::from_file(
                                path,
                                line_numbers,
                                &config.theme,
                                &config.languages,
                            )
                            .unwrap_or_else(|_| BufferState::new_empty(line_numbers));
                            // Replace buffer content with recovered content
                            let current_len = buf_state.buffer.len();
                            if current_len > 0 {
                                buf_state.buffer.delete(0, current_len);
                            }
                            buf_state.buffer.insert(0, &content);
                            // Restore cursor position from session
                            buf_state.cursors[buf_state.primary].cursor.set_position(
                                bs.cursor_line,
                                bs.cursor_col,
                                &buf_state.buffer,
                            );
                            buf_state.scroll_row = bs.scroll_row;
                            recovery_msgs.push(format!("Recovered: {}", file_path));
                            swap::remove_swap(path);
                            buffers.push(buf_state);
                            continue;
                        }
                    }
                    // Normal file open
                    match BufferState::from_file(
                        path,
                        line_numbers,
                        &config.theme,
                        &config.languages,
                    ) {
                        Ok(mut buf_state) => {
                            buf_state.cursors[buf_state.primary].cursor.set_position(
                                bs.cursor_line,
                                bs.cursor_col,
                                &buf_state.buffer,
                            );
                            buf_state.scroll_row = bs.scroll_row;
                            // Clean up any swap that belongs to us
                            if swap_status == swap::SwapStatus::OwnedByUs {
                                swap::remove_swap(path);
                            }
                            buffers.push(buf_state);
                        }
                        Err(_) => {
                            // File no longer exists or unreadable, skip
                        }
                    }
                }
            } else {
                // Untitled buffer — only restore if swap file exists
                if let Some(idx) = bs.untitled_index {
                    let swp = swap::swap_path_untitled(idx);
                    if swp.exists()
                        && let Ok((_header, content)) = swap::read_swap(&swp)
                    {
                        let mut buf_state = BufferState::new_empty(line_numbers);
                        buf_state.buffer.insert(0, &content);
                        buf_state.untitled_id = Some(idx);
                        buf_state.cursors[buf_state.primary].cursor.set_position(
                            bs.cursor_line,
                            bs.cursor_col,
                            &buf_state.buffer,
                        );
                        buf_state.scroll_row = bs.scroll_row;
                        recovery_msgs.push(format!("Recovered NewBuffer{:02}", idx));
                        swap::remove_swap_untitled(idx);
                        buffers.push(buf_state);
                    }
                    // No swap → buffer was closed properly, don't recreate
                }
            }
        }

        // Also scan for orphaned untitled swap files not tracked by the session
        let known_ids: Vec<usize> = buffers.iter().filter_map(|bs| bs.untitled_id).collect();
        for (id, swp_path) in swap::scan_orphaned_untitled() {
            if known_ids.contains(&id) {
                continue; // Already recovered from session data
            }
            if let Ok((_header, content)) = swap::read_swap(&swp_path) {
                let mut buf_state = BufferState::new_empty(line_numbers);
                buf_state.buffer.insert(0, &content);
                buf_state.untitled_id = Some(id);
                recovery_msgs.push(format!("Recovered NewBuffer{:02}", id));
                swap::remove_swap_untitled(id);
                buffers.push(buf_state);
            }
        }

        if buffers.is_empty() {
            let mut bs = BufferState::new_empty(line_numbers);
            bs.untitled_id = Some(1);
            buffers.push(bs);
        }

        // Compute next untitled ID from restored buffers
        let max_untitled = buffers
            .iter()
            .filter_map(|bs| bs.untitled_id)
            .max()
            .unwrap_or(0);

        let active = sess.active_buffer.min(buffers.len().saturating_sub(1));
        let layout = LayoutState::new(active);
        let active_pane = layout.first_pane();

        let startup_message = if recovery_msgs.is_empty() {
            "Session restored".to_string()
        } else {
            recovery_msgs.join(", ")
        };

        let lsp_manager = Self::build_lsp_manager(&config);

        Ok(Editor {
            buffers,
            active_buffer: active,
            screen: Screen::new(w as usize, h as usize),
            terminal,
            color_mode,
            config,
            status_height: 2,
            layout,
            active_pane,
            message: Some(startup_message),
            message_type: MessageType::Info,
            quit_confirm: false,
            clipboard: Clipboard::new(),
            sys_clipboard: arboard::Clipboard::new().ok(),
            prompt: None,
            mouse_dragging: false,
            scroll_only_mode: false,
            help_visible: false,
            palette: None,
            filetree: None,
            filetree_focused: false,
            vterms: Vec::new(),
            ptys: Vec::new(),
            terminal_panel_pane: None,
            terminal_idx: None,
            bottom_tab: BottomTab::Terminal,
            lsp_manager,
            lsp_diag_gen: 0,
            completion_menu: None,
            hover_popup: None,
            plugin_manager: Self::build_plugin_manager(),
            diff_view: None,
            minimap: minimap::Minimap::new(),
            swap_timer: std::time::Instant::now(),
            swap_interval_ms: 2000,
            disk_check_timer: std::time::Instant::now(),
            file_change_notice: None,
            next_untitled_id: max_untitled + 1,
            tab_bar_height: 1,
            tab_regions: Vec::new(),
            tab_scroll_offset: 0,
            running: true,
            last_task: None,
            task_language: None,
            problem_panel: crate::problem_panel::ProblemPanel::new(),
            bg_cargo_rx: None,
            diag_panel_selected: 0,
            diag_panel_scroll: 0,
            last_title: String::new(),
        })
    }

    /// Check for orphaned swap when opening a single file from CLI.
    pub fn check_swap_on_open(&mut self, path: &Path) {
        let status = swap::check_swap(path);
        if status == swap::SwapStatus::Orphaned {
            let swp = swap::swap_path(path);
            if let Ok((_header, content)) = swap::read_swap(&swp) {
                // Replace buffer content with recovered content
                let b = self.buf_mut();
                let current_len = b.buffer.len();
                if current_len > 0 {
                    b.buffer.delete(0, current_len);
                }
                b.buffer.insert(0, &content);
                b.cursors[b.primary].cursor.set_position(0, 0, &b.buffer);
                swap::remove_swap(path);
                self.set_message("Recovered from swap file!", MessageType::Warning);
            }
        } else if status == swap::SwapStatus::Corrupt {
            // Remove corrupt swap
            swap::remove_swap(path);
        }
    }

    // -----------------------------------------------------------------------
    // Disk-change detection
    // -----------------------------------------------------------------------

    /// Check all open file-backed buffers for on-disk mtime changes.
    /// Called once per main loop iteration; throttled to every 2 seconds.
    fn check_disk_changes(&mut self) {
        if self.disk_check_timer.elapsed().as_millis() < 2000 {
            return;
        }
        self.disk_check_timer = std::time::Instant::now();

        // Don't raise a new notice while one is already being shown.
        if self.file_change_notice.is_some() {
            return;
        }

        for buf_idx in 0..self.buffers.len() {
            let path = match self.buffers[buf_idx].buffer.file_path() {
                Some(p) => p.to_path_buf(),
                None => continue,
            };
            let current_mtime = match std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok())
            {
                Some(t) => t,
                None => continue,
            };
            if let Some(stored) = self.buffers[buf_idx].disk_mtime {
                if current_mtime != stored {
                    let modified = self.buffers[buf_idx].buffer.is_modified();
                    self.file_change_notice = Some(FileChangeNotice {
                        buf_idx,
                        buffer_modified: modified,
                    });
                    break; // one notice at a time
                }
            }
        }
    }

    /// Replace the buffer contents with the current on-disk version.
    fn reload_buffer_from_disk(&mut self, buf_idx: usize) {
        let path = match self.buffers[buf_idx].buffer.file_path() {
            Some(p) => p.to_path_buf(),
            None => return,
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                self.set_message(&format!("Reload failed: {}", e), MessageType::Error);
                return;
            }
        };
        let b = &mut self.buffers[buf_idx];
        let total = b.buffer.len();
        b.buffer.delete(0, total);
        b.buffer.insert(0, &text);
        b.buffer.mark_saved();
        // Clamp cursor to valid range
        let primary = b.primary;
        let old_line = b.cursors[primary].cursor.line;
        let lines = b.buffer.line_count();
        b.cursors[primary].cursor.line = old_line.min(lines.saturating_sub(1));
        b.cursors[primary].cursor.col = 0;
        b.scroll_row = b.scroll_row.min(b.cursors[primary].cursor.line);
        // Update stored mtime so we don't re-trigger immediately
        b.update_disk_mtime(&path);
        self.set_message("Buffer reloaded from disk", MessageType::Info);
    }

    /// Handle a key when the file-change notice overlay is active.
    fn handle_file_change_key(&mut self, key: KeyEvent) {
        let notice = match self.file_change_notice.take() {
            Some(n) => n,
            None => return,
        };
        match key.key {
            Key::Char('r') | Key::Char('R') => {
                self.reload_buffer_from_disk(notice.buf_idx);
            }
            Key::Char('d') | Key::Char('D') if notice.buffer_modified => {
                self.open_diff_vs_disk(notice.buf_idx);
            }
            Key::Char('i') | Key::Char('I') | Key::Escape => {
                // Ignore: advance stored mtime so we don't re-trigger immediately.
                let path = self.buffers[notice.buf_idx]
                    .buffer
                    .file_path()
                    .map(|p| p.to_path_buf());
                if let Some(p) = path {
                    self.buffers[notice.buf_idx].update_disk_mtime(&p);
                }
                self.set_message("File change ignored", MessageType::Info);
            }
            _ => {
                // Any other key: restore the notice so it remains visible.
                self.file_change_notice = Some(notice);
            }
        }
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

    /// Get the display name for a buffer (file path or NewBufferNN).
    fn buffer_display_name(&self, buf_idx: usize) -> String {
        let bs = &self.buffers[buf_idx];
        if let Some(path) = bs.buffer.file_path() {
            shorten_path(path)
        } else if let Some(id) = bs.untitled_id {
            format!("[NewBuffer{:02}]", id)
        } else {
            "[No Name]".to_string()
        }
    }

    /// Width of the file tree sidebar (0 if not visible).
    fn sidebar_width(&self) -> u16 {
        self.filetree.as_ref().map_or(0, |ft| ft.width)
    }

    /// Resolve the layout tree for the current terminal size.
    fn resolve_layout(&mut self) {
        let (w, h) = self.terminal.size();
        let sidebar_w = self.sidebar_width();
        let pane_area_height =
            (h as usize).saturating_sub(self.status_height + self.tab_bar_height) as u16;
        let pane_area_width = w.saturating_sub(sidebar_w);
        self.layout.resolve(Rect {
            x: sidebar_w,
            y: self.tab_bar_height as u16,
            width: pane_area_width,
            height: pane_area_height,
        });
    }

    /// Check if the active pane is a terminal pane.
    fn active_pane_is_terminal(&self) -> bool {
        // If bottom panel is active but showing Problems or Diagnostics tab → NOT a terminal for input purposes
        if self.terminal_panel_pane == Some(self.active_pane)
            && (self.bottom_tab == BottomTab::Problems || self.bottom_tab == BottomTab::Diagnostics)
        {
            return false;
        }
        matches!(
            self.layout.pane_content(self.active_pane),
            Some(PaneContent::Terminal(_))
        )
    }

    /// Ensure the active pane is an editor (buffer) pane, not a terminal.
    /// If the active pane is a terminal, switch focus to the nearest editor pane.
    /// Returns the pane ID suitable for opening files.
    fn ensure_editor_pane(&mut self) -> PaneId {
        // Also treat the bottom panel (Problems OR Terminal tab) as needing a switch,
        // because set_pane_buffer on it would replace the terminal with a buffer pane.
        let is_bottom_panel = self.terminal_panel_pane == Some(self.active_pane);
        if !is_bottom_panel && !self.active_pane_is_terminal() {
            return self.active_pane;
        }
        // Find a buffer pane to switch to
        let target = self
            .layout
            .adjacent_pane(self.active_pane, Direction::Up)
            .or_else(|| self.layout.adjacent_pane(self.active_pane, Direction::Left))
            .or_else(|| {
                self.layout
                    .adjacent_pane(self.active_pane, Direction::Right)
            })
            .or_else(|| self.layout.adjacent_pane(self.active_pane, Direction::Down))
            .unwrap_or(self.layout.first_pane());
        self.active_pane = target;
        self.active_buffer = self.active_buffer_index();
        target
    }

    /// Get the terminal index for the active pane, if it's a terminal.
    fn active_terminal_index(&self) -> Option<usize> {
        match self.layout.pane_content(self.active_pane) {
            Some(PaneContent::Terminal(idx)) => Some(idx),
            _ => None,
        }
    }

    /// Run the main editor loop.
    pub fn run(&mut self) -> Result<(), String> {
        self.resolve_layout();
        // Notify LSP for all initially opened buffers
        if self.lsp_manager.is_some() {
            for i in 0..self.buffers.len() {
                self.lsp_notify_open(i);
            }
        }
        while self.running {
            // 1. Check for resize
            if self.terminal.check_resize() {
                let (w, h) = self.terminal.size();
                self.screen.resize(w as usize, h as usize);
                self.resolve_layout();
                self.recompute_all_wrap_maps();
                self.sync_pty_sizes();
                if !self.active_pane_is_terminal() {
                    self.adjust_viewport();
                }
            }

            // 2. Drain PTY output
            self.drain_all_ptys();

            // 2b. Drain LSP messages and sync diagnostics
            self.drain_lsp_messages();

            // 2c. Drain plugin messages and handle requests
            self.drain_plugin_messages();

            // 2d. Collect background cargo check output when finished
            self.drain_bg_cargo_check();

            // 2e. Check for on-disk file changes
            self.check_disk_changes();

            // 3. Reap dead children
            for pty in &mut self.ptys {
                pty.reap();
            }

            // 4. Render
            self.render();

            // 5. Poll stdin + PTY fds
            let (stdin_ready, pty_ready) = self.poll_fds(50);

            // 6. Drain ready PTYs again
            for idx in &pty_ready {
                self.drain_pty(*idx);
            }

            // 7. Handle stdin if ready
            if stdin_ready {
                let event = input::read_event(&self.terminal);
                if event != Event::None {
                    self.handle_event(event);
                }
            }

            // 8. Periodic swap file writes for modified buffers
            if self.swap_timer.elapsed().as_millis() >= self.swap_interval_ms as u128 {
                self.save_all_swaps();
                self.swap_timer = std::time::Instant::now();
            }

            // 9. Send LSP didChange for dirty buffers
            self.flush_lsp_changes();
        }

        // On exit: shutdown plugins, LSP servers, save session and final swap state
        if let Some(ref mut mgr) = self.plugin_manager {
            mgr.shutdown_all();
        }
        if let Some(ref mut mgr) = self.lsp_manager {
            mgr.shutdown_all();
        }
        self.save_session();

        Ok(())
    }

    /// Poll stdin and all PTY master fds.
    /// Returns (stdin_ready, vec_of_pty_indices_with_data).
    fn poll_fds(&self, timeout_ms: i32) -> (bool, Vec<usize>) {
        let lsp_fds = self
            .lsp_manager
            .as_ref()
            .map(|m| m.stdout_fds())
            .unwrap_or_default();
        let plugin_fds = self
            .plugin_manager
            .as_ref()
            .map(|m| m.stdout_fds())
            .unwrap_or_default();
        let mut fds = Vec::with_capacity(1 + self.ptys.len() + lsp_fds.len() + plugin_fds.len());

        // stdin
        fds.push(PollFd {
            fd: 0, // STDIN_FILENO
            events: POLLIN,
            revents: 0,
        });

        // PTY master fds
        for pty in &self.ptys {
            if !pty.is_dead() {
                fds.push(PollFd {
                    fd: pty.master_fd(),
                    events: POLLIN,
                    revents: 0,
                });
            }
        }

        // LSP stdout fds
        for fd in &lsp_fds {
            fds.push(PollFd {
                fd: *fd,
                events: POLLIN,
                revents: 0,
            });
        }

        // Plugin stdout fds
        for fd in &plugin_fds {
            fds.push(PollFd {
                fd: *fd,
                events: POLLIN,
                revents: 0,
            });
        }

        let _result = pty::poll_fds(&mut fds, timeout_ms);

        let stdin_ready = fds[0].revents & POLLIN != 0;
        let mut pty_ready = Vec::new();

        let mut fd_idx = 1;
        for (i, pty) in self.ptys.iter().enumerate() {
            if !pty.is_dead() {
                if fd_idx < fds.len() && fds[fd_idx].revents & POLLIN != 0 {
                    pty_ready.push(i);
                }
                fd_idx += 1;
            }
        }

        (stdin_ready, pty_ready)
    }

    /// Drain output from all PTYs into their VTerms.
    fn drain_all_ptys(&mut self) {
        for i in 0..self.ptys.len() {
            self.drain_pty(i);
        }
    }

    /// Drain output from a single PTY into its VTerm.
    fn drain_pty(&mut self, idx: usize) {
        if idx >= self.ptys.len() || idx >= self.vterms.len() {
            return;
        }
        if self.ptys[idx].is_dead() {
            return;
        }

        let mut buf = [0u8; 4096];
        loop {
            let n = self.ptys[idx].read_nonblocking(&mut buf);
            if n == 0 {
                break;
            }

            // Feed to problem panel if this is the task terminal and a task is tracked.
            if self.terminal_idx == Some(idx) && self.problem_panel.source_cmd.is_some() {
                if let Ok(s) = std::str::from_utf8(&buf[..n]) {
                    self.problem_panel.feed_raw(s);
                }
            }

            self.vterms[idx].feed(&buf[..n]);

            // Send any responses back to PTY (e.g., DSR 6n)
            let responses = self.vterms[idx].take_responses();
            for resp in responses {
                self.ptys[idx].write_bytes(&resp);
            }
        }
    }

    // -----------------------------------------------------------------------
    // LSP integration
    // -----------------------------------------------------------------------

    /// Drain LSP messages from all clients and sync diagnostics to buffers.
    fn drain_lsp_messages(&mut self) {
        if self.lsp_manager.is_none() {
            return;
        }

        let mgr = self.lsp_manager.as_mut().unwrap();
        mgr.reap_dead_clients();
        mgr.drain_all();

        // Detect clients that just completed initialization (race-condition fix).
        // lsp_notify_open calls did_open + request_semantic_tokens immediately when a
        // file is opened, but the server may not be initialized yet at that point, so
        // both calls silently no-op. Now that drain_all() has processed the initialize
        // response, we re-send for every open buffer whose language just became ready.
        let newly_init_langs = mgr.take_newly_initialized_langs();

        // Sync diagnostics from LSP clients to matching buffer states
        for bs in &mut self.buffers {
            if let Some(ref lang) = bs.lsp_language {
                if let Some(path) = bs.buffer.file_path() {
                    let path_str = path.to_string_lossy();
                    let uri = lsp::protocol::path_to_uri(&path_str);
                    if let Some(client) = mgr.client_mut(lang) {
                        let diags = client.diagnostics_for(&uri);
                        bs.diagnostics = diags
                            .iter()
                            .map(|d| (d.range.clone(), d.severity, d.message.clone()))
                            .collect();
                    }
                }
            }
        }

        // Feed LSP diagnostics from ALL workspace files into the problem panel.
        // Only rebuild when diagnostics actually changed (generation counter).
        {
            let current_gen = mgr.diagnostics_generation();
            if current_gen != self.lsp_diag_gen {
                self.lsp_diag_gen = current_gen;

                let workspace_diags = mgr.all_diagnostics_owned();
                crate::dlog!(
                    "[lsp_diag] gen={} files={}",
                    current_gen,
                    workspace_diags.len()
                );
                for (u, d) in &workspace_diags {
                    crate::dlog!("[lsp_diag]   file={} count={}", u, d.len());
                }

                let cwd = std::env::current_dir()
                    .ok()
                    .map(|p| {
                        let mut s = p.to_string_lossy().into_owned();
                        if !s.ends_with('/') {
                            s.push('/');
                        }
                        s
                    })
                    .unwrap_or_default();

                let mut lsp_problems = Vec::new();
                for (uri, diags) in workspace_diags {
                    let abs = lsp::protocol::uri_to_path(&uri).unwrap_or(uri);
                    // Display relative path when possible (strips CWD prefix).
                    let file_str = if !cwd.is_empty() && abs.starts_with(&cwd) {
                        abs[cwd.len()..].to_string()
                    } else {
                        abs
                    };
                    for d in &diags {
                        use crate::lsp::protocol::DiagnosticSeverity;
                        use crate::problem_panel::{Problem, Severity};
                        let severity = match d.severity {
                            DiagnosticSeverity::Error => Severity::Error,
                            DiagnosticSeverity::Warning => Severity::Warning,
                            DiagnosticSeverity::Info => Severity::Info,
                            DiagnosticSeverity::Hint => Severity::Note,
                        };
                        lsp_problems.push(Problem {
                            severity,
                            file: file_str.clone(),
                            line: d.range.start.line + 1,
                            col: d.range.start.character + 1,
                            message: d.message.clone(),
                            code: None,
                        });
                    }
                }
                // Sort by file then by line for consistent, readable ordering.
                lsp_problems.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
                self.problem_panel.set_lsp_items(lsp_problems);
            }
        }

        // Collect interactive LSP results (NLL ends mgr borrow after last use)
        let new_completion = mgr.take_completion_result();
        let new_hover = mgr.take_hover_result();
        let new_definition = mgr.take_definition_result();
        // Drain all pending semantic token results — multiple buffers may have responses ready.
        let mut sem_token_results = Vec::new();
        while let Some(r) = mgr.take_semantic_tokens_result() {
            sem_token_results.push(r);
        }

        // Apply results — mgr borrow is fully released at this point
        if let Some(items) = new_completion {
            crate::dlog!("[lsp] completion result arrived: {} items", items.len());
            let (row, col) = self.cursor_screen_pos_for_lsp();
            self.completion_menu = Some(completion::CompletionMenu::new(items, row, col));
        }
        if let Some(text) = new_hover {
            let (row, col) = self.cursor_screen_pos_for_lsp();
            self.hover_popup = Some(hover::HoverPopup::new(&text, row, col, 60));
        }
        if let Some(locs) = new_definition
            && let Some(loc) = locs.into_iter().next()
        {
            self.lsp_navigate_to(loc);
        }
        // Apply all accumulated semantic token results — one per buffer URI.
        use crate::syntax::highlight;
        for (uri, tokens, legend) in sem_token_results {
            for bs in &mut self.buffers {
                if let Some(path) = bs.buffer.file_path() {
                    if lsp::protocol::path_to_uri(&path.to_string_lossy()) == uri {
                        let theme_opt = bs.highlighter.as_ref().map(|hl| &hl.theme);
                        bs.semantic_spans = tokens
                            .iter()
                            .map(|t| {
                                let type_name = legend
                                    .get(t.token_type_idx as usize)
                                    .map(|s| s.as_str())
                                    .unwrap_or("");
                                let scope = highlight::semantic_token_scope(type_name);
                                let (fg, bold) = if let Some(th) = theme_opt {
                                    let style = th.resolve(&[scope.to_string()]);
                                    (style.fg, style.bold)
                                } else {
                                    (crate::render::Color::Default, false)
                                };
                                highlight::SemanticSpan {
                                    line: t.line,
                                    start_char: t.start_char,
                                    end_char: t.start_char + t.length,
                                    fg,
                                    bold,
                                }
                            })
                            .collect();
                        break;
                    }
                }
            }
        }

        // Re-notify all open buffers for languages that just became initialized.
        // This fixes the startup race condition where did_open + request_semantic_tokens
        // were silently dropped because the server hadn't responded to initialize yet.
        for lang in newly_init_langs {
            self.lsp_renotify_after_init(&lang);
        }
    }

    /// Re-send did_open + request_semantic_tokens for every open buffer whose
    /// LSP language matches `lang`. Called once per language, right after the
    /// initialize handshake completes.
    fn lsp_renotify_after_init(&mut self, lang: &str) {
        // Collect buffer info without borrowing self.lsp_manager yet
        let buf_infos: Vec<(String, String)> = self
            .buffers
            .iter()
            .filter(|bs| bs.lsp_language.as_deref() == Some(lang))
            .filter_map(|bs| {
                let path = bs.buffer.file_path()?.to_string_lossy().to_string();
                let uri = lsp::protocol::path_to_uri(&path);
                let text = bs.buffer.text();
                Some((uri, text))
            })
            .collect();

        for (uri, text) in buf_infos {
            if let Some(ref mut mgr) = self.lsp_manager {
                if let Some(client) = mgr.client_mut(lang) {
                    // Push CARGO_INCREMENTAL=0 config before did_save so flycheck
                    // re-checks all workspace files (bypasses artifact cache).
                    client.notify_configuration();
                    client.did_open(&uri, &text);
                    client.request_semantic_tokens(&uri);
                    // Trigger flycheck so rust-analyzer analyses the whole workspace
                    // and sends publishDiagnostics for ALL files, not just open ones.
                    client.did_save(&uri);
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Plugin integration
    // -----------------------------------------------------------------------

    /// Drain plugin messages and handle each incoming request.
    fn drain_plugin_messages(&mut self) {
        if self.plugin_manager.is_none() {
            return;
        }

        // Collect requests (releases borrow on plugin_manager)
        let requests = self.plugin_manager.as_mut().unwrap().drain_and_collect();

        for req in requests {
            match req {
                plugin::PluginRequest::RegisterCommand { .. } => {
                    // Already stored on the plugin's commands vec.
                    // If palette is open, rebuild it to include new command.
                    if self.palette.is_some() {
                        self.refresh_palette_plugin_commands();
                    }
                }
                plugin::PluginRequest::SubscribeEvent { .. } => {
                    // Already stored on the plugin's subscriptions vec.
                }
                plugin::PluginRequest::ShowMessage { text, kind, .. } => {
                    let msg_type = if kind == "error" {
                        MessageType::Error
                    } else if kind == "warning" {
                        MessageType::Warning
                    } else {
                        MessageType::Info
                    };
                    self.set_message(&text, msg_type);
                }
                plugin::PluginRequest::InsertText { text, .. } => {
                    self.delete_selection();
                    self.handle_paste(&text);
                }
                plugin::PluginRequest::GetBufferText {
                    plugin_name,
                    request_id,
                } => {
                    let text = self.buf().buffer.text();
                    let response = plugin::build_response(
                        &request_id,
                        crate::syntax::json_parser::JsonValue::String(text),
                    );
                    if let Some(ref mut mgr) = self.plugin_manager {
                        mgr.send_to(&plugin_name, &response);
                    }
                }
                plugin::PluginRequest::GetFilePath {
                    plugin_name,
                    request_id,
                } => {
                    let path_val = self
                        .buf()
                        .buffer
                        .file_path()
                        .map(|p| {
                            crate::syntax::json_parser::JsonValue::String(
                                p.to_string_lossy().to_string(),
                            )
                        })
                        .unwrap_or(crate::syntax::json_parser::JsonValue::Null);
                    let response = plugin::build_response(&request_id, path_val);
                    if let Some(ref mut mgr) = self.plugin_manager {
                        mgr.send_to(&plugin_name, &response);
                    }
                }
            }
        }

        // Reap any dead plugin processes
        if let Some(ref mut mgr) = self.plugin_manager {
            mgr.reap_dead();
        }
    }

    /// Dispatch a plugin event to all subscribed plugins.
    fn plugin_dispatch(
        &mut self,
        kind: plugin::EventKind,
        data: crate::syntax::json_parser::JsonValue,
    ) {
        if let Some(ref mut mgr) = self.plugin_manager {
            mgr.dispatch_event(&kind, &data);
        }
    }

    /// Rebuild the palette's plugin-command section when the palette is open.
    fn refresh_palette_plugin_commands(&mut self) {
        let cmds: Vec<(String, String, String)> = if let Some(ref mgr) = self.plugin_manager {
            mgr.all_commands()
                .iter()
                .map(|(pname, cmd)| (pname.clone(), cmd.id.clone(), cmd.label.clone()))
                .collect()
        } else {
            Vec::new()
        };
        if let Some(ref mut palette) = self.palette {
            palette.replace_plugin_commands(&cmds);
        }
    }

    /// Notify LSP that a buffer was opened.
    fn lsp_notify_open(&mut self, buf_idx: usize) {
        let path = match self.buffers[buf_idx].buffer.file_path() {
            Some(p) => p.to_string_lossy().to_string(),
            None => return,
        };

        // Dispatch plugin event
        let event_data = crate::syntax::json_parser::JsonValue::Object(vec![(
            "path".to_string(),
            crate::syntax::json_parser::JsonValue::String(path.clone()),
        )]);
        self.plugin_dispatch(plugin::EventKind::BufferOpen, event_data);

        // Detect language from file extension
        let lang = match self.detect_lsp_language(&path) {
            Some(l) => l,
            None => return,
        };

        self.buffers[buf_idx].lsp_language = Some(lang.clone());
        self.buffers[buf_idx].lsp_version = 1;

        let uri = lsp::protocol::path_to_uri(&path);
        let text = self.buffers[buf_idx].buffer.text();

        if let Some(ref mut mgr) = self.lsp_manager {
            if let Some(client) = mgr.ensure_client(&lang) {
                client.did_open(&uri, &text);
                client.request_semantic_tokens(&uri);
            }
        }
    }

    /// Notify LSP that a buffer changed (full document sync).
    fn lsp_notify_change(&mut self, buf_idx: usize) {
        let lang = match self.buffers[buf_idx].lsp_language.clone() {
            Some(l) => l,
            None => return,
        };
        let path = match self.buffers[buf_idx].buffer.file_path() {
            Some(p) => p.to_string_lossy().to_string(),
            None => return,
        };
        let uri = lsp::protocol::path_to_uri(&path);
        let text = self.buffers[buf_idx].buffer.text();

        if let Some(ref mut mgr) = self.lsp_manager {
            if let Some(client) = mgr.client_mut(&lang) {
                client.did_change(&uri, &text);
                // Re-request semantic tokens so directive-based language sections
                // (.:ES:. / .:EN:.) stay highlighted after edits, not just after save.
                client.request_semantic_tokens(&uri);
            }
        }
    }

    /// Close any open LSP overlays (completion menu, hover popup).
    fn dismiss_lsp_overlays(&mut self) {
        self.completion_menu = None;
        self.hover_popup = None;
    }

    /// Notify LSP that a buffer was saved.
    fn lsp_notify_save(&mut self, buf_idx: usize) {
        // Dispatch plugin event
        if let Some(path) = self.buffers[buf_idx].buffer.file_path() {
            let path_str = path.to_string_lossy().to_string();
            let event_data = crate::syntax::json_parser::JsonValue::Object(vec![(
                "path".to_string(),
                crate::syntax::json_parser::JsonValue::String(path_str),
            )]);
            self.plugin_dispatch(plugin::EventKind::BufferSave, event_data);
        }

        let lang = match self.buffers[buf_idx].lsp_language.clone() {
            Some(l) => l,
            None => return,
        };
        let path = match self.buffers[buf_idx].buffer.file_path() {
            Some(p) => p.to_string_lossy().to_string(),
            None => return,
        };
        let uri = lsp::protocol::path_to_uri(&path);

        if let Some(ref mut mgr) = self.lsp_manager {
            if let Some(client) = mgr.client_mut(&lang) {
                client.did_save(&uri);
                client.request_semantic_tokens(&uri);
            }
        }
    }

    /// Notify LSP that a buffer was closed.
    fn lsp_notify_close(&mut self, buf_idx: usize) {
        // Dispatch plugin event
        if let Some(path) = self.buffers[buf_idx].buffer.file_path() {
            let path_str = path.to_string_lossy().to_string();
            let event_data = crate::syntax::json_parser::JsonValue::Object(vec![(
                "path".to_string(),
                crate::syntax::json_parser::JsonValue::String(path_str),
            )]);
            self.plugin_dispatch(plugin::EventKind::BufferClose, event_data);
        }

        let lang = match self.buffers[buf_idx].lsp_language.clone() {
            Some(l) => l,
            None => return,
        };
        let path = match self.buffers[buf_idx].buffer.file_path() {
            Some(p) => p.to_string_lossy().to_string(),
            None => return,
        };
        let uri = lsp::protocol::path_to_uri(&path);

        if let Some(ref mut mgr) = self.lsp_manager {
            if let Some(client) = mgr.client_mut(&lang) {
                client.did_close(&uri);
            }
        }
    }

    /// Send didChange for all buffers marked dirty.
    fn flush_lsp_changes(&mut self) {
        if self.lsp_manager.is_none() {
            return;
        }
        for i in 0..self.buffers.len() {
            if self.buffers[i].lsp_dirty && self.buffers[i].lsp_language.is_some() {
                self.buffers[i].lsp_dirty = false;
                self.lsp_notify_change(i);
            }
        }
    }

    /// Detect LSP language ID from file path using config's language definitions.
    fn detect_lsp_language(&self, path: &str) -> Option<String> {
        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())?;
        crate::dlog!("[lsp] detect_lsp_language: path={} ext={}", path, ext);
        for lang_def in &self.config.languages {
            if lang_def.extensions.iter().any(|e| e == ext) {
                crate::dlog!("[lsp] detected language: {}", lang_def.name);
                // Only return a language if we have a server configured for it
                let has_server = self.lsp_manager.is_some()
                    && self
                        .config
                        .lsp_servers
                        .iter()
                        .any(|(l, _)| l == &lang_def.name);
                crate::dlog!("[lsp] has_server={}", has_server);
                return Some(lang_def.name.clone());
            }
        }
        crate::dlog!("[lsp] no language match for ext={}", ext);
        None
    }

    /// Return the (screen_row, screen_col) of the cursor in the active buffer.
    fn cursor_screen_pos_for_lsp(&self) -> (usize, usize) {
        if let Some(rect) = self.layout.pane_rect(self.active_pane) {
            let b = self.buf();
            let cursor_line = b.cursor().line;
            let scroll_row = b.scroll_row;
            let row = rect.y as usize + cursor_line.saturating_sub(scroll_row);
            let display_col = self.cursor_display_col();
            let scroll_col = b.scroll_col;
            let col = rect.x as usize + b.gutter_width + display_col.saturating_sub(scroll_col);
            (row, col)
        } else {
            (0, 0)
        }
    }

    /// Trigger LSP completion at the current cursor position.
    fn lsp_trigger_completion(&mut self) {
        let buf_idx = self.active_buffer_index();
        crate::dlog!("[lsp] Ctrl+Space triggered — buf_idx={}", buf_idx);
        let lang = match self.buffers[buf_idx].lsp_language.clone() {
            Some(l) => l,
            None => {
                crate::dlog!("[lsp] no lsp_language for buf_idx={} — aborting", buf_idx);
                return;
            }
        };
        let path = match self.buffers[buf_idx].buffer.file_path() {
            Some(p) => p.to_string_lossy().to_string(),
            None => {
                crate::dlog!("[lsp] buffer has no file path — aborting");
                return;
            }
        };
        let uri = lsp::protocol::path_to_uri(&path);
        let line = self.buffers[buf_idx].cursor().line as u32;
        let character = self.buffers[buf_idx].cursor().col as u32;
        crate::dlog!(
            "[lsp] requesting completion: lang={} uri={} line={} char={}",
            lang,
            uri,
            line,
            character
        );
        if let Some(ref mut mgr) = self.lsp_manager {
            mgr.request_completion(&lang, &uri, line, character);
            crate::dlog!("[lsp] request_completion sent");
        } else {
            crate::dlog!("[lsp] lsp_manager is None — no LSP configured");
        }
    }

    /// Trigger LSP hover at the current cursor position.
    fn lsp_trigger_hover(&mut self) {
        let buf_idx = self.active_buffer_index();
        let lang = match self.buffers[buf_idx].lsp_language.clone() {
            Some(l) => l,
            None => return,
        };
        let path = match self.buffers[buf_idx].buffer.file_path() {
            Some(p) => p.to_string_lossy().to_string(),
            None => return,
        };
        let uri = lsp::protocol::path_to_uri(&path);
        let line = self.buffers[buf_idx].cursor().line as u32;
        let character = self.buffers[buf_idx].cursor().col as u32;
        if let Some(ref mut mgr) = self.lsp_manager {
            mgr.request_hover(&lang, &uri, line, character);
        }
    }

    /// Trigger LSP go-to-definition at the current cursor position.
    fn lsp_trigger_goto_def(&mut self) {
        let buf_idx = self.active_buffer_index();
        let lang = match self.buffers[buf_idx].lsp_language.clone() {
            Some(l) => l,
            None => return,
        };
        let path = match self.buffers[buf_idx].buffer.file_path() {
            Some(p) => p.to_string_lossy().to_string(),
            None => return,
        };
        let uri = lsp::protocol::path_to_uri(&path);
        let line = self.buffers[buf_idx].cursor().line as u32;
        let character = self.buffers[buf_idx].cursor().col as u32;
        if let Some(ref mut mgr) = self.lsp_manager {
            mgr.request_definition(&lang, &uri, line, character);
        }
    }

    /// Handle a key event while the completion menu is open.
    /// Returns true if the key was consumed (menu acted on it).
    fn handle_completion_key(&mut self, ke: KeyEvent) -> bool {
        match &ke.key {
            Key::Tab | Key::Enter => {
                let text = self
                    .completion_menu
                    .as_ref()
                    .map(|m| m.selected_insert_text().to_string())
                    .unwrap_or_default();
                self.completion_menu = None;
                if !text.is_empty() {
                    self.lsp_apply_completion(&text);
                }
                true
            }
            Key::Up => {
                if let Some(ref mut menu) = self.completion_menu {
                    menu.select_prev();
                }
                true
            }
            Key::Down => {
                if let Some(ref mut menu) = self.completion_menu {
                    menu.select_next();
                }
                true
            }
            Key::Escape => {
                self.completion_menu = None;
                true
            }
            _ => {
                // Any other key: close the menu but let the key pass through
                self.completion_menu = None;
                false
            }
        }
    }

    /// Insert completion text at the current cursor position.
    fn lsp_apply_completion(&mut self, text: &str) {
        self.delete_selection();
        self.handle_paste(text);
    }

    /// Navigate to an LSP Location (same file or open new buffer).
    fn lsp_navigate_to(&mut self, loc: lsp::Location) {
        let path = match lsp::protocol::uri_to_path(&loc.uri) {
            Some(p) => p,
            None => return,
        };

        let target_line = loc.range.start.line as usize;
        let target_col = loc.range.start.character as usize;

        // Check if any existing buffer has this path
        let existing = self.buffers.iter().position(|bs| {
            bs.buffer
                .file_path()
                .map(|p| p.to_string_lossy() == path.as_str())
                .unwrap_or(false)
        });

        if let Some(buf_idx) = existing {
            // Switch to that buffer
            self.layout.set_pane_buffer(self.active_pane, buf_idx);
            self.active_buffer = buf_idx;
            let b = &mut self.buffers[buf_idx];
            b.cursors[b.primary]
                .cursor
                .set_position(target_line, target_col, &b.buffer);
            b.scroll_row = target_line.saturating_sub(5);
        } else {
            // Open new buffer
            let file_path = std::path::Path::new(&path);
            if !file_path.exists() {
                return;
            }
            if let Ok(mut new_buf) = BufferState::from_file(
                file_path,
                self.config.line_numbers,
                &self.config.theme,
                &self.config.languages,
            ) {
                new_buf.cursors[new_buf.primary].cursor.set_position(
                    target_line,
                    target_col,
                    &new_buf.buffer,
                );
                new_buf.scroll_row = target_line.saturating_sub(5);
                self.buffers.push(new_buf);
                let new_idx = self.buffers.len() - 1;
                self.layout.set_pane_buffer(self.active_pane, new_idx);
                self.active_buffer = new_idx;
                self.lsp_notify_open(new_idx);
            }
        }

        self.dismiss_lsp_overlays();
        self.adjust_viewport();
    }

    /// Sync PTY/VTerm sizes to match their pane rects.
    fn sync_pty_sizes(&mut self) {
        let panes: Vec<_> = self.layout.panes().to_vec();
        for pane_info in &panes {
            if let PaneContent::Terminal(ti) = pane_info.content
                && ti < self.ptys.len()
                && ti < self.vterms.len()
            {
                let cols = pane_info.rect.width;
                // If this pane is the bottom panel, subtract 1 row for the tab bar
                let rows = if self.terminal_panel_pane == Some(pane_info.id) {
                    pane_info.rect.height.saturating_sub(1)
                } else {
                    pane_info.rect.height
                };
                if cols > 0 && rows > 0 {
                    self.ptys[ti].resize(cols, rows);
                    self.vterms[ti].resize(cols, rows);
                }
            }
        }
    }

    /// Detect the user's shell.
    fn detect_shell() -> String {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }

    /// Toggle the integrated terminal panel (bottom split).
    // -----------------------------------------------------------------------
    // Task runner
    // -----------------------------------------------------------------------

    /// Detect the language name for a file path using the config language table.
    fn detect_language_by_ext(&self, file_path: &str) -> Option<String> {
        let ext = std::path::Path::new(file_path)
            .extension()
            .and_then(|e| e.to_str())?;
        self.config
            .languages
            .iter()
            .find(|l| l.extensions.iter().any(|e| e == ext))
            .map(|l| l.name.clone())
    }

    /// Ensure the terminal panel is open; opens one if needed.
    /// Always switches to the Terminal tab so task output is visible.
    fn ensure_terminal_panel(&mut self) {
        if self.terminal_panel_pane.is_none() {
            self.bottom_tab = BottomTab::Terminal;
            self.restore_or_new_terminal_panel();
        } else {
            // Panel already open — switch to Terminal tab if needed
            self.bottom_tab = BottomTab::Terminal;
            self.problem_panel.focused = false;
        }
    }

    /// Write text directly to the main terminal panel PTY.
    fn send_to_terminal_panel(&mut self, text: &str) {
        let idx = match self.terminal_idx {
            Some(i) => i,
            None => return,
        };
        if idx < self.ptys.len() && !self.ptys[idx].is_dead() {
            self.ptys[idx].write_bytes(text.as_bytes());
        }
    }

    /// Run a task (run/build/test) for the active buffer's language.
    fn run_task(&mut self, kind: tasks::TaskKind) {
        let buf_idx = self.active_buffer_index();
        let file_path = match self.buffers[buf_idx].buffer.file_path() {
            Some(p) => p.to_string_lossy().into_owned(),
            None => {
                self.set_message("Save the file first before running", MessageType::Warning);
                return;
            }
        };

        let lang = match self.detect_language_by_ext(&file_path) {
            Some(l) => l,
            None => {
                self.set_message(
                    "Unknown file type — no task available",
                    MessageType::Warning,
                );
                return;
            }
        };

        let workspace = std::env::current_dir()
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();

        let cmd_template =
            match tasks::TaskRunner::resolve(&lang, kind, &self.config.extensions, &self.config) {
                Some(c) => c,
                None => {
                    self.set_message(
                        &format!("No {} task configured for '{}'", kind.as_str(), lang),
                        MessageType::Warning,
                    );
                    return;
                }
            };

        let cmd = tasks::TaskRunner::expand(&cmd_template, &file_path, &workspace);
        self.last_task = Some(cmd.clone());
        self.task_language = Some(lang);

        // Reset problem panel for the new task.
        self.problem_panel.clear();
        self.problem_panel.source_cmd = Some(cmd.clone());

        self.ensure_terminal_panel();
        self.send_to_terminal_panel(&format!("{}\n", cmd));
        self.set_message(&format!("▶ {}", cmd), MessageType::Info);
    }

    /// Send Ctrl+C to the terminal panel to stop a running task.
    fn stop_task(&mut self) {
        match self.terminal_idx {
            Some(idx) if idx < self.ptys.len() && !self.ptys[idx].is_dead() => {
                self.ptys[idx].write_bytes(&[0x03]); // Ctrl+C
                self.set_message("Sent Ctrl+C to terminal", MessageType::Info);
            }
            _ => {
                self.set_message("No terminal open", MessageType::Warning);
            }
        }
    }

    /// Format the active buffer using an external formatter (stdin → stdout).
    fn format_document(&mut self) {
        let buf_idx = self.active_buffer_index();
        let file_path = match self.buffers[buf_idx].buffer.file_path() {
            Some(p) => p.to_string_lossy().into_owned(),
            None => {
                self.set_message(
                    "Save the file first before formatting",
                    MessageType::Warning,
                );
                return;
            }
        };

        let lang = match self.detect_language_by_ext(&file_path) {
            Some(l) => l,
            None => {
                self.set_message(
                    "Unknown file type — no formatter available",
                    MessageType::Warning,
                );
                return;
            }
        };

        // Zenith and Zymbol use an in-place external formatter:
        //   save → `zenith --format {file}` / `zymbol --format {file}` → reload.
        if let Some(fmt_cmd) = match lang.as_str() {
            "zenith" => Some("zenith"),
            "zymbol" => Some("zymbol"),
            _ => None,
        } {
            self.format_document_inplace(buf_idx, &file_path, fmt_cmd);
            return;
        }

        // Map language name → (command, args).
        // Formatters that read stdin and write stdout are preferred.
        let (cmd, args): (&str, Vec<&str>) = match lang.as_str() {
            "rust" => ("rustfmt", vec!["--emit", "stdout", &file_path]),
            "go" => ("gofmt", vec![&file_path]),
            "javascript" | "typescript" => ("prettier", vec!["--stdin-filepath", &file_path]),
            "python" => ("black", vec!["-", "--quiet"]),
            "c" | "cpp" => ("clang-format", vec![]),
            _ => {
                self.set_message(
                    &format!("No formatter configured for '{}'", lang),
                    MessageType::Warning,
                );
                return;
            }
        };

        // Collect current buffer content to feed via stdin.
        let line_count = self.buffers[buf_idx].buffer.line_count();
        let mut content = String::new();
        for i in 0..line_count {
            if let Some(line) = self.buffers[buf_idx].buffer.get_line(i) {
                content.push_str(&line);
                content.push('\n');
            }
        }

        use std::io::Write as _;
        use std::process::{Command, Stdio};

        let mut child = match Command::new(cmd)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => {
                self.set_message(
                    &format!("Formatter not available: {}", cmd),
                    MessageType::Warning,
                );
                return;
            }
        };

        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(content.as_bytes());
        }

        let output = match child.wait_with_output() {
            Ok(o) => o,
            Err(_) => {
                self.set_message(&format!("Formatter failed: {}", cmd), MessageType::Error);
                return;
            }
        };

        if !output.status.success() {
            self.set_message(
                &format!("Formatter exited with error: {}", cmd),
                MessageType::Warning,
            );
            return;
        }

        let formatted = match String::from_utf8(output.stdout) {
            Ok(s) => s,
            Err(_) => {
                self.set_message("Formatter produced non-UTF-8 output", MessageType::Error);
                return;
            }
        };

        // Replace buffer content with formatted output.
        {
            let b = &mut self.buffers[buf_idx];
            let total_chars = b.buffer.len();
            b.buffer.delete(0, total_chars);
            b.buffer.insert(0, &formatted);
            let primary = b.primary;
            b.cursors[primary].cursor.line = 0;
            b.cursors[primary].cursor.col = 0;
            b.scroll_row = 0;
            b.scroll_col = 0;
        }

        self.set_message(&format!("Formatted with {}", cmd), MessageType::Info);
    }

    /// Format a Zenith or Zymbol file using its external CLI formatter.
    ///
    /// Flow: save pending changes → `<cmd> --format <file>` (in-place) →
    /// read formatted content from disk → replace buffer via delete+insert →
    /// mark buffer as saved.
    fn format_document_inplace(&mut self, buf_idx: usize, file_path: &str, fmt_cmd: &str) {
        use std::process::Command;

        // Flush unsaved edits to disk so the formatter sees the latest content.
        if self.buffers[buf_idx].buffer.is_modified() {
            if let Err(e) = self.buffers[buf_idx].buffer.save() {
                self.set_message(
                    &format!("Save failed before format: {}", e),
                    MessageType::Error,
                );
                return;
            }
        }

        let status = match Command::new(fmt_cmd).args(["--format", file_path]).status() {
            Ok(s) => s,
            Err(_) => {
                self.set_message(
                    &format!("Formatter not found: {} (is it on $PATH?)", fmt_cmd),
                    MessageType::Warning,
                );
                return;
            }
        };

        if !status.success() {
            self.set_message(
                &format!("{} --format exited with error", fmt_cmd),
                MessageType::Error,
            );
            return;
        }

        // Read the formatted file from disk.
        let formatted = match std::fs::read_to_string(file_path) {
            Ok(s) => s,
            Err(e) => {
                self.set_message(
                    &format!("Reload failed after format: {}", e),
                    MessageType::Error,
                );
                return;
            }
        };

        // Replace buffer content using the same delete+insert pattern used by
        // the stdin/stdout formatters so syntax highlighting re-tokenizes correctly.
        {
            let b = &mut self.buffers[buf_idx];
            let total_len = b.buffer.len();
            b.buffer.delete(0, total_len);
            b.buffer.insert(0, &formatted);
            b.buffer.mark_saved();
            // Preserve cursor line (clamped), reset column to 0.
            let primary = b.primary;
            let old_line = b.cursors[primary].cursor.line;
            let line_count = b.buffer.line_count();
            let new_line = old_line.min(line_count.saturating_sub(1));
            b.cursors[primary].cursor.line = new_line;
            b.cursors[primary].cursor.col = 0;
            b.scroll_row = b.scroll_row.min(new_line);
        }

        self.set_message(
            &format!("Formatted with {} --format", fmt_cmd),
            MessageType::Info,
        );
    }

    fn toggle_problem_panel(&mut self) {
        if let Some(pane_id) = self.terminal_panel_pane {
            if self.layout.pane_exists(pane_id) {
                if self.bottom_tab == BottomTab::Problems {
                    // Already on Problems tab → close panel
                    self.close_bottom_panel();
                } else {
                    // Switch to Problems tab
                    self.bottom_tab = BottomTab::Problems;
                    self.problem_panel.focused = true;
                    self.problem_panel.visible = true;
                    self.active_pane = pane_id;
                    // Auto-run cargo check if no results yet (panel was open on
                    // a different tab so check was never triggered).
                    if self.problem_panel.items.is_empty() {
                        self.auto_cargo_check_if_rust();
                    }
                }
                return;
            }
        }
        // Panel not open → open it on Problems tab.
        self.bottom_tab = BottomTab::Problems;
        self.restore_or_new_terminal_panel();
        self.problem_panel.focused = true;
        self.problem_panel.visible = true;

        // Auto-run cargo check when opening the Problems panel for a Rust project.
        // LSP publishDiagnostics only provides native type-checker diagnostics for
        // open files. Compiler warnings (dead_code, unused, etc.) for ALL workspace
        // files require running `cargo check 2>&1` and parsing its output.
        // The output is captured via the terminal PTY → problem_panel.feed_raw().
        self.auto_cargo_check_if_rust();
    }

    /// Toggle the Diagnostics panel (LSP diagnostics for the active buffer).
    fn toggle_diagnostics_panel(&mut self) {
        if let Some(pane_id) = self.terminal_panel_pane {
            if self.layout.pane_exists(pane_id) {
                if self.bottom_tab == BottomTab::Diagnostics {
                    // Already on Diagnostics tab → close panel
                    self.close_bottom_panel();
                } else {
                    // Switch to Diagnostics tab
                    self.bottom_tab = BottomTab::Diagnostics;
                    self.problem_panel.focused = false;
                    self.active_pane = pane_id;
                }
                return;
            }
        }
        // Panel not open → open it on Diagnostics tab.
        self.bottom_tab = BottomTab::Diagnostics;
        self.restore_or_new_terminal_panel();
    }

    /// Return indices (into `problem_panel.lsp_items`) of diagnostics that belong
    /// to the active buffer's file, normalized to a relative path.
    fn active_buffer_lsp_items(&self) -> Vec<usize> {
        let buf_idx = self.active_buffer_index();
        let active_file = match self.buffers.get(buf_idx) {
            Some(bs) => bs.buffer.file_path().map(|p| {
                let abs = p.to_string_lossy().into_owned();
                // Normalize to relative path (same logic as drain_lsp_messages).
                let cwd = std::env::current_dir()
                    .ok()
                    .map(|cp| {
                        let mut s = cp.to_string_lossy().into_owned();
                        if !s.ends_with('/') {
                            s.push('/');
                        }
                        s
                    })
                    .unwrap_or_default();
                if !cwd.is_empty() && abs.starts_with(&cwd) {
                    abs[cwd.len()..].to_string()
                } else {
                    abs
                }
            }),
            None => None,
        };

        let file = match active_file {
            Some(f) => f,
            None => return Vec::new(),
        };

        self.problem_panel
            .lsp_items
            .iter()
            .enumerate()
            .filter(|(_, p)| p.file == file)
            .map(|(i, _)| i)
            .collect()
    }

    /// Returns `(error_count, warning_count)` for LSP diagnostics in the active buffer.
    fn active_buffer_diag_counts(&self) -> (usize, usize) {
        let indices = self.active_buffer_lsp_items();
        indices.iter().fold((0, 0), |(e, w), &i| {
            match self.problem_panel.lsp_items.get(i).map(|p| p.severity) {
                Some(crate::problem_panel::Severity::Error) => (e + 1, w),
                Some(crate::problem_panel::Severity::Warning) => (e, w + 1),
                _ => (e, w),
            }
        })
    }

    /// Jump to the selected diagnostic item in the Diagnostics panel.
    fn jump_to_diagnostic_item(&mut self) {
        let indices = self.active_buffer_lsp_items();
        let lsp_idx = match indices.get(self.diag_panel_selected) {
            Some(&i) => i,
            None => return,
        };
        let (file, line, col) = match self.problem_panel.lsp_items.get(lsp_idx) {
            Some(p) => (
                p.file.clone(),
                p.line.saturating_sub(1) as usize,
                p.col.saturating_sub(1) as usize,
            ),
            None => return,
        };

        self.ensure_editor_pane();

        let existing = self.buffers.iter().position(|bs| {
            bs.buffer
                .file_path()
                .map(|p| p.to_string_lossy() == file.as_str())
                .unwrap_or(false)
        });

        if let Some(buf_idx) = existing {
            self.layout.set_pane_buffer(self.active_pane, buf_idx);
            self.active_buffer = buf_idx;
            let b = &mut self.buffers[buf_idx];
            b.cursors[b.primary]
                .cursor
                .set_position(line, col, &b.buffer);
            b.scroll_row = line.saturating_sub(5);
        } else {
            let path = std::path::Path::new(&file);
            if !path.exists() {
                self.set_message(&format!("File not found: {}", file), MessageType::Warning);
                return;
            }
            if let Ok(mut new_buf) = BufferState::from_file(
                path,
                self.config.line_numbers,
                &self.config.theme,
                &self.config.languages,
            ) {
                new_buf.cursors[new_buf.primary]
                    .cursor
                    .set_position(line, col, &new_buf.buffer);
                new_buf.scroll_row = line.saturating_sub(5);
                self.buffers.push(new_buf);
                let new_idx = self.buffers.len() - 1;
                self.layout.set_pane_buffer(self.active_pane, new_idx);
                self.active_buffer = new_idx;
                self.lsp_notify_open(new_idx);
            }
        }
        self.adjust_viewport();
    }

    /// If the active buffer is a Rust file with a Cargo.toml ancestor, spawn
    /// `cargo check` in a background thread and feed its output to the Problems
    /// panel when it finishes.  The user's terminal is left untouched.
    fn auto_cargo_check_if_rust(&mut self) {
        let buf_idx = self.active_buffer_index();
        let is_rust = self.buffers[buf_idx]
            .buffer
            .file_path()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            == Some("rs");
        if !is_rust {
            return;
        }

        // Find the Cargo.toml directory (walk up to 6 levels).
        // Resolve to absolute first so that relative paths like "src/main.rs"
        // don't produce an empty parent component that makes current_dir("") fail.
        let cargo_dir = self.buffers[buf_idx]
            .buffer
            .file_path()
            .map(|p| {
                if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    std::env::current_dir()
                        .map(|cwd| cwd.join(p))
                        .unwrap_or_else(|_| p.to_path_buf())
                }
            })
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .and_then(|dir| {
                let mut d = dir;
                for _ in 0..6 {
                    if d.join("Cargo.toml").exists() {
                        return Some(d);
                    }
                    if !d.pop() {
                        break;
                    }
                }
                None
            });

        let cargo_dir = match cargo_dir {
            Some(d) => d,
            None => return,
        };

        // Cancel any in-flight check and start fresh.
        self.bg_cargo_rx = None;
        self.problem_panel.clear();
        self.problem_panel.source_cmd = Some("cargo check".to_string());

        let (tx, rx) = std::sync::mpsc::channel::<String>();
        self.bg_cargo_rx = Some(rx);

        std::thread::spawn(move || {
            use std::process::{Command, Stdio};
            let result = Command::new("cargo")
                .arg("check")
                .arg("--message-format=human")
                .current_dir(&cargo_dir)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output();

            let output = match result {
                Ok(o) => {
                    // cargo check writes human output to stderr; merge both.
                    let mut merged = String::new();
                    merged.push_str(&String::from_utf8_lossy(&o.stdout));
                    merged.push_str(&String::from_utf8_lossy(&o.stderr));
                    merged
                }
                Err(e) => format!("error: failed to run cargo check: {}\n", e),
            };

            let _ = tx.send(output);
        });

        crate::dlog!("[problems] background cargo check spawned");
    }

    /// Poll the background cargo-check receiver.  When output arrives, feed it
    /// to the Problems panel and clear the receiver.
    fn drain_bg_cargo_check(&mut self) {
        let output = match &self.bg_cargo_rx {
            Some(rx) => match rx.try_recv() {
                Ok(s) => s,
                Err(_) => return,
            },
            None => return,
        };

        self.bg_cargo_rx = None;
        self.problem_panel.feed_raw(&output);
        crate::dlog!(
            "[problems] background cargo check done, {} bytes parsed",
            output.len()
        );
    }

    /// Send selected text (or current line) to the Z ecosystem REPL running in
    /// the terminal pane. Supported languages: zenith, zymbol.
    fn send_to_repl(&mut self) {
        let buf_idx = self.active_buffer_index();

        // Detect language from file extension
        let lang = {
            let file_path = self.buffers[buf_idx]
                .buffer
                .file_path()
                .map(|p| p.to_string_lossy().into_owned());
            file_path.and_then(|fp| self.detect_language_by_ext(&fp))
        };

        let repl_cmd = match lang.as_deref() {
            Some("zenith") => "zenith --repl",
            Some("zymbol") => "zymbol --repl",
            _ => {
                self.set_message("REPL not available for this language", MessageType::Warning);
                return;
            }
        };

        // Get selected text, or fall back to current line
        let text = if let Some((start, end)) = self.selection_range() {
            if start != end {
                self.buf().buffer.slice(start, end)
            } else {
                let line = self.buf().cursor().line;
                self.buf().buffer.get_line(line).unwrap_or_default()
            }
        } else {
            let line = self.buf().cursor().line;
            self.buf().buffer.get_line(line).unwrap_or_default()
        };

        if text.trim().is_empty() {
            return;
        }

        // If no live terminal exists, open one and start the REPL
        let is_fresh = self
            .terminal_idx
            .map(|i| i >= self.ptys.len() || self.ptys[i].is_dead())
            .unwrap_or(true);

        self.ensure_terminal_panel();

        if is_fresh {
            // Start the REPL in the newly-opened terminal
            self.send_to_terminal_panel(&format!("{}\n", repl_cmd));
        }

        // Send text (ensure trailing newline so the REPL evaluates it)
        let send_text = if text.ends_with('\n') {
            text
        } else {
            format!("{}\n", text)
        };
        self.send_to_terminal_panel(&send_text);

        let lang_name = lang.as_deref().unwrap_or("?");
        self.set_message(&format!("Sent to {} REPL", lang_name), MessageType::Info);
    }

    /// Open the file and jump to the selected problem's location.
    fn jump_to_problem(&mut self) {
        let (file, line, col) = match self.problem_panel.selected_problem() {
            Some(p) => (
                p.file.clone(),
                p.line.saturating_sub(1) as usize,
                p.col.saturating_sub(1) as usize,
            ),
            None => return,
        };

        // Unfocus panel but leave it visible
        self.problem_panel.focused = false;

        // Ensure an editor pane is active (not terminal)
        self.ensure_editor_pane();

        // Reuse an existing buffer or open new
        let existing = self.buffers.iter().position(|bs| {
            bs.buffer
                .file_path()
                .map(|p| p.to_string_lossy() == file.as_str())
                .unwrap_or(false)
        });

        if let Some(buf_idx) = existing {
            self.layout.set_pane_buffer(self.active_pane, buf_idx);
            self.active_buffer = buf_idx;
            let b = &mut self.buffers[buf_idx];
            b.cursors[b.primary]
                .cursor
                .set_position(line, col, &b.buffer);
            b.scroll_row = line.saturating_sub(5);
        } else {
            let path = std::path::Path::new(&file);
            if !path.exists() {
                self.set_message(&format!("File not found: {}", file), MessageType::Warning);
                return;
            }
            if let Ok(mut new_buf) = BufferState::from_file(
                path,
                self.config.line_numbers,
                &self.config.theme,
                &self.config.languages,
            ) {
                new_buf.cursors[new_buf.primary]
                    .cursor
                    .set_position(line, col, &new_buf.buffer);
                new_buf.scroll_row = line.saturating_sub(5);
                self.buffers.push(new_buf);
                let new_idx = self.buffers.len() - 1;
                self.layout.set_pane_buffer(self.active_pane, new_idx);
                self.active_buffer = new_idx;
                self.lsp_notify_open(new_idx);
            }
        }
        self.adjust_viewport();
    }

    fn toggle_terminal_panel(&mut self) {
        if let Some(pane_id) = self.terminal_panel_pane {
            if self.layout.pane_exists(pane_id) {
                if self.bottom_tab == BottomTab::Terminal {
                    // Already on Terminal tab → close panel
                    self.close_bottom_panel();
                } else {
                    // Switch to Terminal tab
                    self.bottom_tab = BottomTab::Terminal;
                    self.problem_panel.focused = false;
                    self.active_pane = pane_id;
                }
                return;
            }
        }
        // Panel not open → open on Terminal tab
        self.bottom_tab = BottomTab::Terminal;
        // Determine if we will restore an existing PTY or spawn a new one
        let will_restore = self
            .terminal_idx
            .map(|idx| idx < self.ptys.len() && !self.ptys[idx].is_dead())
            .unwrap_or(false);
        self.restore_or_new_terminal_panel();
        if will_restore {
            self.set_message("Terminal restored", MessageType::Info);
        } else {
            self.set_message("Terminal opened", MessageType::Info);
        }
    }

    /// Close the bottom panel pane (shared helper).
    fn close_bottom_panel(&mut self) {
        if let Some(pane_id) = self.terminal_panel_pane.take() {
            if self.layout.pane_exists(pane_id) {
                let next = self
                    .layout
                    .adjacent_pane(pane_id, Direction::Up)
                    .or_else(|| self.layout.adjacent_pane(pane_id, Direction::Left))
                    .unwrap_or(self.layout.first_pane());
                self.layout.close_pane(pane_id);
                self.active_pane = next;
                if let Some(PaneContent::Buffer(_)) = self.layout.pane_content(self.active_pane) {
                    self.active_buffer = self.active_buffer_index();
                }
            }
            self.resolve_layout();
            self.problem_panel.focused = false;
            self.problem_panel.visible = false;
        }
    }

    /// Restore the previous terminal session or spawn a new one.
    fn restore_or_new_terminal_panel(&mut self) {
        // Check if we have a living PTY to reuse
        if let Some(idx) = self.terminal_idx
            && idx < self.ptys.len()
            && !self.ptys[idx].is_dead()
        {
            // Reuse existing session — just re-split the pane
            let split_target = self.active_pane;
            let content = PaneContent::Terminal(idx);
            if let Some(new_id) =
                self.layout
                    .split_pane_with_content(split_target, SplitDir::Vertical, content)
            {
                self.resolve_layout();
                let (w, h) = self.terminal.size();
                let sidebar_w = self.sidebar_width();
                let pane_area_height =
                    (h as usize).saturating_sub(self.status_height + self.tab_bar_height) as u16;
                let total = Rect {
                    x: sidebar_w,
                    y: self.tab_bar_height as u16,
                    width: w.saturating_sub(sidebar_w),
                    height: pane_area_height,
                };
                let delta = (pane_area_height as i16) * 20 / 100;
                self.layout.resize_split(split_target, delta, total);
                self.terminal_panel_pane = Some(new_id);
                self.active_pane = new_id;
                self.resolve_layout();
                self.sync_pty_sizes();
            }
            return;
        }
        // No living session — spawn new
        self.new_terminal_panel();
    }

    /// Spawn a new terminal in a bottom 30% split.
    fn new_terminal_panel(&mut self) {
        let shell = Self::detect_shell();

        // Get the pane to split (use the active pane or find an editor pane)
        let split_target = self.active_pane;

        // Create VTerm and PTY
        let vterm = VTerm::new(80, 12); // Will be resized after layout
        let pty = match Pty::spawn(80, 12, &shell) {
            Ok(p) => p,
            Err(e) => {
                self.set_message(
                    &format!("Failed to spawn terminal: {}", e),
                    MessageType::Error,
                );
                return;
            }
        };

        let term_idx = self.vterms.len();
        self.vterms.push(vterm);
        self.ptys.push(pty);
        self.terminal_idx = Some(term_idx);

        // Split the active pane vertically (top|bottom) with 70/30 ratio
        let content = PaneContent::Terminal(term_idx);
        if let Some(new_id) =
            self.layout
                .split_pane_with_content(split_target, SplitDir::Vertical, content)
        {
            // Adjust ratios: we want the original pane to be 70%, terminal 30%
            // The split creates 50/50 by default, so resize
            self.resolve_layout();

            // Resize to get ~70/30 split
            let (w, h) = self.terminal.size();
            let sidebar_w = self.sidebar_width();
            let pane_area_height =
                (h as usize).saturating_sub(self.status_height + self.tab_bar_height) as u16;
            let total = Rect {
                x: sidebar_w,
                y: self.tab_bar_height as u16,
                width: w.saturating_sub(sidebar_w),
                height: pane_area_height,
            };
            // Grow the original pane by 20% (from 50% to 70%)
            let delta = (pane_area_height as i16) * 20 / 100;
            self.layout.resize_split(split_target, delta, total);

            self.terminal_panel_pane = Some(new_id);
            self.active_pane = new_id;
            self.resolve_layout();
            self.sync_pty_sizes();
        }
    }

    /// Handle a click on the bottom panel tab bar.
    fn handle_bottom_panel_tab_click(&mut self, col: u16, rect: crate::layout::Rect) {
        let rel_col = col.saturating_sub(rect.x) as usize;

        // Compute tab widths the same way render_bottom_panel_tab_bar does.
        let term_w = "  TERMINAL  ".len(); // 12

        // DIAGNOSTICS tab width (base label + optional counts badge)
        let (de, dw) = self.active_buffer_diag_counts();
        let diag_w = if de > 0 || dw > 0 {
            format!("  DIAGNOSTICS E:{} W:{}  ", de, dw).len()
        } else {
            "  DIAGNOSTICS  ".len()
        };

        if rel_col < term_w {
            // Click on TERMINAL tab
            self.bottom_tab = BottomTab::Terminal;
            self.problem_panel.focused = false;
            if let Some(pane_id) = self.terminal_panel_pane {
                self.active_pane = pane_id;
            }
        } else if rel_col < term_w + diag_w {
            // Click on DIAGNOSTICS tab
            self.bottom_tab = BottomTab::Diagnostics;
            self.problem_panel.focused = false;
            if let Some(pane_id) = self.terminal_panel_pane {
                self.active_pane = pane_id;
            }
        } else {
            // Click on PROBLEMS tab
            self.bottom_tab = BottomTab::Problems;
            self.problem_panel.focused = true;
            self.problem_panel.visible = true;
            if let Some(pane_id) = self.terminal_panel_pane {
                self.active_pane = pane_id;
            }
            if self.problem_panel.items.is_empty() {
                self.auto_cargo_check_if_rust();
            }
        }
    }

    /// Spawn a new terminal tab (Ctrl+Shift+T).
    fn new_terminal(&mut self) {
        if self.terminal_panel_pane.is_none() {
            self.new_terminal_panel();
        } else {
            // TODO: support multiple terminal tabs
            self.set_message("Terminal already open", MessageType::Warning);
        }
    }

    /// Forward a key event to the active PTY.
    fn forward_key_to_pty(&mut self, ke: &KeyEvent) {
        let term_idx = match self.active_terminal_index() {
            Some(idx) => idx,
            None => return,
        };
        if term_idx >= self.ptys.len() || self.ptys[term_idx].is_dead() {
            return;
        }

        let bytes = key_event_to_bytes(ke);
        if !bytes.is_empty() {
            self.ptys[term_idx].write_bytes(&bytes);
        }
    }

    /// Forward pasted text to the active PTY.
    fn forward_paste_to_pty(&mut self, text: &str) {
        let term_idx = match self.active_terminal_index() {
            Some(idx) => idx,
            None => return,
        };
        if term_idx >= self.ptys.len() || self.ptys[term_idx].is_dead() {
            return;
        }
        self.ptys[term_idx].write_bytes(text.as_bytes());
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

        // When diagnostics panel is active, route navigation keys to it
        if self.terminal_panel_pane == Some(self.active_pane)
            && self.bottom_tab == BottomTab::Diagnostics
        {
            if let Event::Key(ref ke) = event {
                match &ke.key {
                    Key::Up => {
                        if self.diag_panel_selected > 0 {
                            self.diag_panel_selected -= 1;
                        }
                        return;
                    }
                    Key::Down => {
                        let count = self.active_buffer_lsp_items().len();
                        if self.diag_panel_selected + 1 < count {
                            self.diag_panel_selected += 1;
                        }
                        return;
                    }
                    Key::Enter => {
                        self.jump_to_diagnostic_item();
                        return;
                    }
                    Key::Escape => {
                        self.close_bottom_panel();
                        self.ensure_editor_pane();
                        return;
                    }
                    _ => {
                        // Allow F6/ToggleDiagnosticsPanel and Ctrl+T/ToggleTerminal to fall through
                        let action = self.config.keybindings.lookup(ke);
                        if action != Some(EditorAction::ToggleDiagnosticsPanel)
                            && action != Some(EditorAction::ToggleTerminal)
                            && action != Some(EditorAction::ToggleProblemPanel)
                        {
                            return; // Consume other keys silently
                        }
                    }
                }
            }
        }

        // When problem panel is focused (in bottom panel), route navigation keys to it
        if self.problem_panel.focused
            && self.terminal_panel_pane == Some(self.active_pane)
            && self.bottom_tab == BottomTab::Problems
        {
            if let Event::Key(ref ke) = event {
                match &ke.key {
                    Key::Up => {
                        self.problem_panel.move_up();
                        return;
                    }
                    Key::Down => {
                        self.problem_panel.move_down();
                        return;
                    }
                    Key::Enter => {
                        // On a header row: toggle collapse. On an item: jump.
                        use crate::problem_panel::{BUILD_GROUP_KEY, CARGO_FILE_PREFIX, RowKind};
                        let row = self
                            .problem_panel
                            .get_display_row(self.problem_panel.selected);
                        match row {
                            Some(RowKind::CargoSectionHeader) => {
                                self.problem_panel.toggle_group(BUILD_GROUP_KEY);
                            }
                            Some(RowKind::CargoFileHeader(f)) => {
                                let key = format!("{}{}", CARGO_FILE_PREFIX, f);
                                self.problem_panel.toggle_group(&key);
                            }
                            Some(RowKind::Item(_)) | None => {
                                self.jump_to_problem();
                            }
                        }
                        return;
                    }
                    Key::Escape => {
                        // Close the panel entirely and return focus to the editor.
                        self.close_bottom_panel();
                        self.ensure_editor_pane();
                        return;
                    }
                    _ => {
                        // Allow F7/F6/Ctrl+T (toggle) to fall through to execute_action
                        let action = self.config.keybindings.lookup(ke);
                        if action != Some(EditorAction::ToggleProblemPanel)
                            && action != Some(EditorAction::ToggleDiagnosticsPanel)
                            && action != Some(EditorAction::ToggleTerminal)
                        {
                            return; // Consume other keys silently
                        }
                    }
                }
            }
        }

        // When active pane is a terminal, forward most keys to PTY
        if self.active_pane_is_terminal() {
            match event {
                Event::Key(ref ke) => {
                    // Shift+PageUp/PageDown: terminal scrollback navigation
                    if ke.shift && matches!(ke.key, Key::PageUp | Key::PageDown) {
                        if let Some(idx) = self.active_terminal_index() {
                            let pane_h = self
                                .layout
                                .pane_rect(self.active_pane)
                                .map(|r| r.height as isize)
                                .unwrap_or(24);
                            let delta = if ke.key == Key::PageUp {
                                -pane_h
                            } else {
                                pane_h
                            };
                            self.vterms[idx].scroll_view(delta);
                        }
                        return;
                    }
                    // Shift+Arrow: extend keyboard selection (never forward to PTY —
                    // the bare escape sequences confuse bash and produce 'C'/'D').
                    if ke.shift
                        && !ke.ctrl
                        && !ke.alt
                        && matches!(ke.key, Key::Left | Key::Right | Key::Up | Key::Down)
                    {
                        let ke_clone = ke.clone();
                        self.extend_terminal_selection(&ke_clone);
                        return;
                    }
                    // Intercept editor-level keybindings
                    if self.is_terminal_intercepted_key(ke) {
                        self.handle_terminal_meta_key(ke.clone());
                        return;
                    }
                    // Any keypress resets scroll to bottom and clears selection
                    if let Some(idx) = self.active_terminal_index() {
                        let off = self.vterms[idx].scroll_offset();
                        if off > 0 {
                            self.vterms[idx].scroll_view(off as isize);
                        }
                        self.vterms[idx].clear_selection();
                    }
                    // Forward to PTY
                    self.forward_key_to_pty(ke);
                }
                Event::Paste(ref text) => {
                    // Reset scroll on paste
                    if let Some(idx) = self.active_terminal_index() {
                        let off = self.vterms[idx].scroll_offset();
                        if off > 0 {
                            self.vterms[idx].scroll_view(off as isize);
                        }
                    }
                    self.forward_paste_to_pty(text);
                }
                Event::Mouse(me) => {
                    // Left-click outside the terminal pane switches focus
                    if me.button == crate::input::MouseButton::Left && me.pressed && !me.motion {
                        // Top tab bar click
                        if (me.row as usize) < self.tab_bar_height {
                            self.handle_mouse(me);
                            return;
                        }
                        // Bottom panel tab bar click
                        if let Some(bp_id) = self.terminal_panel_pane {
                            if let Some(rect) = self.layout.pane_rect(bp_id) {
                                if me.row == rect.y
                                    && me.col >= rect.x
                                    && me.col < rect.x + rect.width
                                {
                                    self.handle_bottom_panel_tab_click(me.col, rect);
                                    return;
                                }
                            }
                        }
                        if let Some(clicked_pane) = self.pane_at_mouse(me.col, me.row)
                            && clicked_pane != self.active_pane
                        {
                            self.active_pane = clicked_pane;
                            self.active_buffer = self.active_buffer_index();
                            return;
                        }
                        // Also handle clicks in the sidebar area
                        let sidebar_w = self.sidebar_width();
                        if sidebar_w > 0 && me.col < sidebar_w {
                            self.handle_mouse(me);
                            return;
                        }
                    }
                    // Handle scroll wheel and left-drag selection in terminal pane
                    if let Some(idx) = self.active_terminal_index() {
                        match me.button {
                            crate::input::MouseButton::ScrollUp => {
                                self.vterms[idx].scroll_view(-3);
                            }
                            crate::input::MouseButton::ScrollDown => {
                                self.vterms[idx].scroll_view(3);
                            }
                            crate::input::MouseButton::Left => {
                                if let Some(rect) = self.layout.pane_rect(self.active_pane) {
                                    // Offset by 1 row for the tab bar in the bottom panel
                                    let tab_bar_offset =
                                        if self.terminal_panel_pane == Some(self.active_pane) {
                                            1u16
                                        } else {
                                            0u16
                                        };
                                    let local_row = me.row.saturating_sub(rect.y + tab_bar_offset);
                                    let local_col = me.col.saturating_sub(rect.x);
                                    if me.pressed && !me.motion {
                                        // Start new selection on click
                                        self.vterms[idx].set_sel_anchor(local_row, local_col);
                                    } else if me.motion {
                                        // Extend selection while dragging
                                        self.vterms[idx].set_sel_active(local_row, local_col);
                                    }
                                    // On release: keep selection as-is
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Event::None => {}
            }
            return;
        }

        // Completion menu: consumes navigation + accept/dismiss keys
        if self.completion_menu.is_some()
            && let Event::Key(ref ke) = event
        {
            let ke_copy = ke.clone();
            if self.handle_completion_key(ke_copy) {
                return;
            }
        }

        // Hover popup: any key or mouse event dismisses it (processing continues)
        if self.hover_popup.is_some() && matches!(&event, Event::Key(_) | Event::Mouse(_)) {
            self.hover_popup = None;
        }

        // File-change notice overlay: consume all key events
        if self.file_change_notice.is_some() {
            if let Event::Key(ke) = event {
                self.handle_file_change_key(ke);
            }
            return;
        }

        // Diff view overlay: consume all key events
        if self.diff_view.is_some() {
            if let Event::Key(ke) = event {
                self.handle_diff_key(ke);
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

    /// Check if a key event should be intercepted when in terminal mode.
    /// Any key bound to an editor action is intercepted so it works even
    /// when the terminal pane is focused.
    fn is_terminal_intercepted_key(&self, ke: &KeyEvent) -> bool {
        // Alt+Arrow: pane focus/resize — always intercepted (structural)
        if ke.alt && matches!(ke.key, Key::Up | Key::Down | Key::Left | Key::Right) {
            return true;
        }
        // Intercept ALL keymap-bound actions
        self.config.keybindings.lookup(ke).is_some()
    }

    /// Handle intercepted keys when in terminal mode.
    fn handle_terminal_meta_key(&mut self, ke: KeyEvent) {
        // Alt+Arrow pane operations (structural, not configurable)
        if ke.alt && matches!(ke.key, Key::Up | Key::Down | Key::Left | Key::Right) {
            match (&ke.key, ke.shift) {
                (Key::Left, false) => self.focus_pane(Direction::Left),
                (Key::Right, false) => self.focus_pane(Direction::Right),
                (Key::Up, false) => self.focus_pane(Direction::Up),
                (Key::Down, false) => self.focus_pane(Direction::Down),
                (Key::Left, true) => self.resize_active_pane(-2),
                (Key::Right, true) => self.resize_active_pane(2),
                (Key::Up, true) => self.resize_active_pane_vertical(-2),
                (Key::Down, true) => self.resize_active_pane_vertical(2),
                _ => {}
            }
            return;
        }
        // Copy / Paste / Cut need special terminal behaviour
        if let Some(action) = self.config.keybindings.lookup(&ke) {
            match action {
                EditorAction::Copy => {
                    // Copy terminal selection; if nothing selected, Ctrl+C = SIGINT.
                    let copied = self.terminal_copy();
                    if !copied {
                        self.forward_key_to_pty(&ke);
                    }
                }
                EditorAction::Cut => {
                    // Cut = copy in terminal (nothing to delete from rendered output).
                    // If no selection just inform the user — never send \x18 to PTY.
                    let copied = self.terminal_copy();
                    if !copied {
                        self.set_message("No text selected", MessageType::Warning);
                    }
                }
                EditorAction::Paste => {
                    // Prefer system clipboard, fall back to internal clipboard.
                    let text = self
                        .sys_clip_get()
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| self.clipboard.text());
                    if !text.is_empty() {
                        self.forward_paste_to_pty(&text);
                    } else {
                        self.forward_key_to_pty(&ke);
                    }
                }
                other => self.execute_action(other),
            }
        }
    }

    /// Copy selected text from the active terminal.
    /// Returns true if there was a selection that was copied.
    /// Extend the terminal keyboard selection by one step in the arrow direction.
    /// If no selection is active, anchors at the VTerm cursor position first.
    fn extend_terminal_selection(&mut self, ke: &KeyEvent) {
        let idx = match self.active_terminal_index() {
            Some(i) => i,
            None => return,
        };
        let pane_rect = match self.layout.pane_rect(self.active_pane) {
            Some(r) => r,
            None => return,
        };
        let pane_h = pane_rect.height as usize;
        let pane_w = pane_rect.width as usize;

        // VTerm cursor in pane-local display coordinates (accounting for scrollback).
        let scroll_off = self.vterms[idx].scroll_offset();
        let sb_lines = scroll_off
            .min(self.vterms[idx].scrollback().len())
            .min(pane_h);
        let (vt_row, vt_col) = self.vterms[idx].cursor_pos();
        let anchor_row = (sb_lines + vt_row as usize).min(pane_h.saturating_sub(1)) as u16;
        let anchor_col = vt_col;

        // If no selection exists yet, plant the anchor at the cursor.
        if self.vterms[idx].sel_anchor.is_none() {
            self.vterms[idx].set_sel_anchor(anchor_row, anchor_col);
            self.vterms[idx].set_sel_active(anchor_row, anchor_col);
        }

        // Advance the active (moving) end one cell in the arrow direction.
        let (row, col) = self.vterms[idx]
            .sel_active
            .unwrap_or((anchor_row, anchor_col));
        let (new_row, new_col) = match ke.key {
            Key::Right => (row, (col + 1).min(pane_w.saturating_sub(1) as u16)),
            Key::Left => (row, col.saturating_sub(1)),
            Key::Down => ((row + 1).min(pane_h.saturating_sub(1) as u16), col),
            Key::Up => (row.saturating_sub(1), col),
            _ => (row, col),
        };
        self.vterms[idx].set_sel_active(new_row, new_col);
    }

    fn terminal_copy(&mut self) -> bool {
        let idx = match self.active_terminal_index() {
            Some(i) => i,
            None => return false,
        };
        if !self.vterms[idx].has_selection() {
            return false;
        }
        let (pane_h, pane_w) = self
            .layout
            .pane_rect(self.active_pane)
            .map(|r| (r.height as usize, r.width as usize))
            .unwrap_or((24, 80));
        if let Some(text) = self.vterms[idx].selection_text(pane_h, pane_w) {
            crate::clipboard::set(&text);
            self.sys_clip_set(&text);
            self.clipboard.set_text(text);
            self.vterms[idx].clear_selection();
            self.set_message("Copied", MessageType::Info);
            true
        } else {
            false
        }
    }

    /// Write text to the persistent arboard clipboard (X11/Wayland native).
    pub(super) fn sys_clip_set(&mut self, text: &str) {
        if let Some(ref mut cb) = self.sys_clipboard {
            let _ = cb.set_text(text);
        }
    }

    /// Read text from the persistent arboard clipboard.
    pub(super) fn sys_clip_get(&mut self) -> Option<String> {
        self.sys_clipboard.as_mut()?.get_text().ok()
    }

    /// Execute a configurable editor action.
    fn execute_action(&mut self, action: EditorAction) {
        match action {
            EditorAction::Save => self.save(),
            EditorAction::SaveAs => self.start_prompt("Save as: ", PromptAction::SaveAs),
            EditorAction::OpenFile => self.start_open_file_prompt(""),
            EditorAction::Quit => self.quit(),
            EditorAction::NewBuffer => self.new_buffer(),
            EditorAction::CloseBuffer => self.close_buffer(),
            EditorAction::Undo => self.do_undo(),
            EditorAction::Redo => self.do_redo(),
            EditorAction::DuplicateLine => self.duplicate_line(),
            EditorAction::DeleteLine => self.delete_line(),
            EditorAction::ToggleComment => self.toggle_comment(),
            EditorAction::Unindent => self.unindent(),
            EditorAction::Copy => self.copy_selection(),
            EditorAction::Cut => self.cut_selection(),
            EditorAction::Paste => self.paste_clipboard(),
            EditorAction::SelectAll => self.select_all(),
            EditorAction::SelectLine => self.select_line(),
            EditorAction::SelectNextOccurrence => self.select_next_occurrence(),
            EditorAction::SelectAllOccurrences => self.select_all_occurrences(),
            EditorAction::Find => self.open_find_prompt(PromptAction::Find),
            EditorAction::Replace => self.open_find_prompt(PromptAction::Replace),
            EditorAction::FindNext => self.search_next(),
            EditorAction::FindPrev => self.search_prev(),
            EditorAction::GoToLine => self.start_prompt("Go to line: ", PromptAction::GoToLine),
            EditorAction::NextBuffer => self.next_buffer(),
            EditorAction::PrevBuffer => self.prev_buffer(),
            EditorAction::SplitHorizontal => self.split_pane_horizontal(),
            EditorAction::SplitVertical => self.split_pane_vertical(),
            EditorAction::ClosePane => self.close_active_pane(),
            EditorAction::FocusLeft => self.focus_pane(Direction::Left),
            EditorAction::FocusRight => self.focus_pane(Direction::Right),
            EditorAction::FocusUp => self.focus_pane(Direction::Up),
            EditorAction::FocusDown => self.focus_pane(Direction::Down),
            EditorAction::ResizePaneLeft => self.resize_active_pane(-2),
            EditorAction::ResizePaneRight => self.resize_active_pane(2),
            EditorAction::ResizePaneUp => self.resize_active_pane_vertical(-2),
            EditorAction::ResizePaneDown => self.resize_active_pane_vertical(2),
            EditorAction::ToggleHelp => {
                self.help_visible = !self.help_visible;
            }
            EditorAction::ToggleWrap => self.toggle_word_wrap(),
            EditorAction::ToggleFileTree => self.toggle_filetree(),
            EditorAction::FocusFileTree => {
                if self.filetree.is_none() {
                    self.toggle_filetree();
                } else {
                    self.filetree_focused = true;
                }
            }
            EditorAction::CommandPalette => {
                let mut p = palette::Palette::new(&self.config.keybindings);
                if let Some(ref mgr) = self.plugin_manager {
                    let cmds: Vec<(String, String, String)> = mgr
                        .all_commands()
                        .iter()
                        .map(|(pname, cmd)| (pname.clone(), cmd.id.clone(), cmd.label.clone()))
                        .collect();
                    p.add_plugin_commands(&cmds);
                }
                self.palette = Some(p);
            }
            EditorAction::ToggleTerminal => self.toggle_terminal_panel(),
            EditorAction::NewTerminal => self.new_terminal(),
            EditorAction::LspComplete => self.lsp_trigger_completion(),
            EditorAction::LspHover => self.lsp_trigger_hover(),
            EditorAction::LspGoToDef => self.lsp_trigger_goto_def(),
            EditorAction::DiffOpenVsHead => self.open_diff_vs_head(),
            EditorAction::DiffNextHunk => {
                if let Some(ref mut dv) = self.diff_view {
                    dv.next_hunk();
                }
            }
            EditorAction::DiffPrevHunk => {
                if let Some(ref mut dv) = self.diff_view {
                    dv.prev_hunk();
                }
            }
            EditorAction::ToggleMinimap => {
                self.minimap.visible = !self.minimap.visible;
            }
            EditorAction::TaskRun => self.run_task(tasks::TaskKind::Run),
            EditorAction::TaskBuild => self.run_task(tasks::TaskKind::Build),
            EditorAction::TaskTest => self.run_task(tasks::TaskKind::Test),
            EditorAction::TaskStop => self.stop_task(),
            EditorAction::ToggleProblemPanel => self.toggle_problem_panel(),
            EditorAction::ToggleDiagnosticsPanel => self.toggle_diagnostics_panel(),
            EditorAction::SendToRepl => self.send_to_repl(),
            EditorAction::ScrollLineUp => {
                let buf_idx = self.active_buffer_index();
                self.buffers[buf_idx].scroll_row =
                    self.buffers[buf_idx].scroll_row.saturating_sub(1);
            }
            EditorAction::ScrollLineDown => {
                let buf_idx = self.active_buffer_index();
                let max_scroll = self.buffers[buf_idx].buffer.line_count().saturating_sub(1);
                self.buffers[buf_idx].scroll_row =
                    (self.buffers[buf_idx].scroll_row + 1).min(max_scroll);
            }
            EditorAction::CopyFilePath => self.copy_file_path(),
            EditorAction::CopyFilePathRelative => self.copy_file_path_relative(),
            EditorAction::FormatDocument => self.format_document(),
        }
    }

    fn handle_key(&mut self, ke: KeyEvent) {
        // Reset quit confirmation on any key that isn't the Quit or CloseBuffer binding
        let action = self.config.keybindings.lookup(&ke);
        if action != Some(EditorAction::Quit) && action != Some(EditorAction::CloseBuffer) {
            self.quit_confirm = false;
        }

        // Scroll-only mode: active only while the user is pressing Ctrl+Up/Down.
        // Any other key re-enables viewport tracking (adjust_viewport will run again).
        let is_scroll_action = matches!(
            action,
            Some(EditorAction::ScrollLineUp) | Some(EditorAction::ScrollLineDown)
        );
        if is_scroll_action {
            self.scroll_only_mode = true;
        } else {
            // Any non-scroll key: let render() call adjust_viewport() again so the
            // viewport snaps back to show the cursor.
            self.scroll_only_mode = false;
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

        // Try configurable keybindings first
        if let Some(action) = self.config.keybindings.lookup(&ke) {
            self.execute_action(action);
        } else {
            // Structural keys: navigation, text input, escape
            match (&ke.key, ke.ctrl, ke.alt) {
                // -- Navigation --
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

                // -- Text input --
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

                // -- Escape: collapse multi-cursor --
                (Key::Escape, false, false) => {
                    if self.buf().is_multi() {
                        self.buf_mut().collapse_to_primary();
                        self.set_message("Single cursor", MessageType::Info);
                    }
                }

                _ => {}
            }
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
    // Swap & Session
    // -----------------------------------------------------------------------

    /// Write swap files for all modified buffers.
    fn save_all_swaps(&self) {
        for bs in &self.buffers {
            if bs.buffer.is_modified() {
                let content = bs.buffer.text_bytes();
                if let Some(path) = bs.buffer.file_path() {
                    let _ = swap::write_swap(path, &content, true);
                } else if let Some(id) = bs.untitled_id {
                    let _ = swap::write_swap_untitled(id, &content, true);
                }
            }
        }
    }

    /// Remove swap file for a specific buffer.
    fn cleanup_swap(&self, buf_idx: usize) {
        if buf_idx >= self.buffers.len() {
            return;
        }
        let bs = &self.buffers[buf_idx];
        if let Some(path) = bs.buffer.file_path() {
            swap::remove_swap(path);
        } else if let Some(id) = bs.untitled_id {
            swap::remove_swap_untitled(id);
        }
    }

    /// Save the current session to disk.
    fn save_session(&self) {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        let mut buf_sessions = Vec::new();
        for bs in &self.buffers {
            let file_path = bs
                .buffer
                .file_path()
                .map(|p| p.to_string_lossy().into_owned());
            let has_swap = bs.buffer.is_modified();

            // Skip empty untitled buffers that were never modified
            if file_path.is_none() && bs.buffer.is_empty() && !has_swap {
                continue;
            }

            let untitled_index = if file_path.is_none() {
                bs.untitled_id
            } else {
                None
            };

            buf_sessions.push(session::BufferSession {
                file_path,
                cursor_line: bs.cursor().line,
                cursor_col: bs.cursor().col,
                scroll_row: bs.scroll_row,
                has_swap,
                untitled_index,
            });
        }

        let sess = session::Session {
            version: 1,
            working_dir: cwd,
            buffers: buf_sessions,
            active_buffer: self.active_buffer_index(),
        };

        let _ = session::save_session(&sess);
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
        // Track if we're closing the terminal panel
        if self.terminal_panel_pane == Some(self.active_pane) {
            self.terminal_panel_pane = None;
            self.problem_panel.focused = false;
            self.problem_panel.visible = false;
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
        if !self.active_pane_is_terminal() {
            self.active_buffer = self.active_buffer_index();
        }
        self.resolve_layout();
        self.set_message("Pane closed", MessageType::Info);
    }

    fn focus_pane(&mut self, dir: Direction) {
        // Alt+Left with no adjacent pane: focus file tree if visible
        if dir == Direction::Left
            && self.layout.adjacent_pane(self.active_pane, dir).is_none()
            && self.filetree.is_some()
        {
            self.filetree_focused = true;
            return;
        }

        if let Some(target) = self.layout.adjacent_pane(self.active_pane, dir) {
            self.active_pane = target;
            if !self.active_pane_is_terminal() {
                self.active_buffer = self.active_buffer_index();
            }
        }
    }

    fn resize_active_pane(&mut self, delta: i16) {
        let (w, h) = self.terminal.size();
        let sidebar_w = self.sidebar_width();
        let pane_area_height =
            (h as usize).saturating_sub(self.status_height + self.tab_bar_height) as u16;
        let total = Rect {
            x: sidebar_w,
            y: self.tab_bar_height as u16,
            width: w.saturating_sub(sidebar_w),
            height: pane_area_height,
        };
        self.layout.resize_split(self.active_pane, delta, total);
        self.resolve_layout();
        self.sync_pty_sizes();
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
            if let crate::layout::PaneContent::Buffer(bi) = pane_info.content
                && bi < self.buffers.len()
            {
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

// ---------------------------------------------------------------------------
// Key-to-bytes encoding for PTY forwarding
// ---------------------------------------------------------------------------

fn key_event_to_bytes(ke: &KeyEvent) -> Vec<u8> {
    match &ke.key {
        Key::Char(ch) => {
            if ke.ctrl {
                // Ctrl+A..Z → 0x01..0x1A
                if ch.is_ascii_lowercase() {
                    return vec![*ch as u8 - b'a' + 1];
                }
                if ch.is_ascii_uppercase() {
                    return vec![ch.to_ascii_lowercase() as u8 - b'a' + 1];
                }
                // Ctrl+space
                if *ch == ' ' {
                    return vec![0x00];
                }
            }
            if ke.alt {
                let mut bytes = vec![0x1b];
                let mut buf = [0u8; 4];
                bytes.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                return bytes;
            }
            let mut buf = [0u8; 4];
            ch.encode_utf8(&mut buf).as_bytes().to_vec()
        }
        Key::Enter => vec![b'\r'],
        Key::Tab => vec![b'\t'],
        Key::BackTab => b"\x1b[Z".to_vec(),
        Key::Backspace => vec![0x7f],
        Key::Delete => b"\x1b[3~".to_vec(),
        Key::Escape => vec![0x1b],
        Key::Up => {
            if ke.ctrl {
                b"\x1b[1;5A".to_vec()
            } else if ke.shift {
                b"\x1b[1;2A".to_vec()
            } else {
                b"\x1b[A".to_vec()
            }
        }
        Key::Down => {
            if ke.ctrl {
                b"\x1b[1;5B".to_vec()
            } else if ke.shift {
                b"\x1b[1;2B".to_vec()
            } else {
                b"\x1b[B".to_vec()
            }
        }
        Key::Right => {
            if ke.ctrl {
                b"\x1b[1;5C".to_vec()
            } else if ke.shift {
                b"\x1b[1;2C".to_vec()
            } else {
                b"\x1b[C".to_vec()
            }
        }
        Key::Left => {
            if ke.ctrl {
                b"\x1b[1;5D".to_vec()
            } else if ke.shift {
                b"\x1b[1;2D".to_vec()
            } else {
                b"\x1b[D".to_vec()
            }
        }
        Key::Home => b"\x1b[H".to_vec(),
        Key::End => b"\x1b[F".to_vec(),
        Key::PageUp => b"\x1b[5~".to_vec(),
        Key::PageDown => b"\x1b[6~".to_vec(),
        Key::F(n) => match n {
            1 => b"\x1bOP".to_vec(),
            2 => b"\x1bOQ".to_vec(),
            3 => b"\x1bOR".to_vec(),
            4 => b"\x1bOS".to_vec(),
            5 => b"\x1b[15~".to_vec(),
            6 => b"\x1b[17~".to_vec(),
            7 => b"\x1b[18~".to_vec(),
            8 => b"\x1b[19~".to_vec(),
            9 => b"\x1b[20~".to_vec(),
            10 => b"\x1b[21~".to_vec(),
            11 => b"\x1b[23~".to_vec(),
            12 => b"\x1b[24~".to_vec(),
            _ => Vec::new(),
        },
    }
}
