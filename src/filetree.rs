use std::path::{Path, PathBuf};

use crate::render::Color;
use crate::render::Screen;

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    File,
    Directory,
}

pub struct TreeNode {
    pub name: String,
    pub path: PathBuf,
    pub kind: NodeKind,
    pub depth: usize,
    pub expanded: bool,
    pub children: Vec<TreeNode>,
    /// Whether children have been loaded from disk yet (lazy scanning).
    children_loaded: bool,
}

pub struct FlatNode {
    pub depth: usize,
    pub kind: NodeKind,
    pub expanded: bool,
    pub name: String,
    pub path: PathBuf,
}

/// Operational mode for the file tree sidebar.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TreeMode {
    Normal,
    Filter,
    /// Prompt for creating a file.
    PromptNewFile,
    /// Prompt for creating a directory.
    PromptNewDir,
    /// Prompt for renaming.
    PromptRename,
    /// Confirm deletion.
    ConfirmDelete,
}

pub struct FileTree {
    root: TreeNode,
    root_path: PathBuf,
    visible: Vec<FlatNode>,
    cursor: usize,
    scroll_offset: usize,
    pub mode: TreeMode,
    pub prompt_input: String,
    prompt_cursor: usize,
    filter_text: String,
    filtered_visible: Vec<usize>, // indices into visible
    pub width: u16,
    ignored: Vec<String>,
}

// ---------------------------------------------------------------------------
// Default ignored paths
// ---------------------------------------------------------------------------

const DEFAULT_IGNORED: &[&str] = &[".git", "target", "node_modules", ".DS_Store", "__pycache__"];

// ---------------------------------------------------------------------------
// FileTree implementation
// ---------------------------------------------------------------------------

impl FileTree {
    pub fn new(root_path: PathBuf, width: u16, extra_ignored: &[String]) -> Self {
        let mut ignored: Vec<String> = DEFAULT_IGNORED.iter().map(|s| s.to_string()).collect();
        for ig in extra_ignored {
            if !ignored.contains(ig) {
                ignored.push(ig.clone());
            }
        }

        let name = root_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| root_path.to_string_lossy().to_string());

        let mut root = TreeNode {
            name,
            path: root_path.clone(),
            kind: NodeKind::Directory,
            depth: 0,
            expanded: true,
            children: Vec::new(),
            children_loaded: false,
        };

        // Eagerly load root children
        root.children = scan_dir(&root_path, 1, &ignored);
        root.children_loaded = true;

        let mut ft = FileTree {
            root,
            root_path,
            visible: Vec::new(),
            cursor: 0,
            scroll_offset: 0,
            mode: TreeMode::Normal,
            prompt_input: String::new(),
            prompt_cursor: 0,
            filter_text: String::new(),
            filtered_visible: Vec::new(),
            width,
            ignored,
        };
        ft.rebuild_visible();
        ft
    }

    // -----------------------------------------------------------------------
    // Visible list management
    // -----------------------------------------------------------------------

    fn rebuild_visible(&mut self) {
        self.visible.clear();
        Self::flatten_node(&self.root, &mut self.visible);
        // Clamp cursor
        if !self.visible.is_empty() {
            self.cursor = self.cursor.min(self.visible.len() - 1);
        } else {
            self.cursor = 0;
        }
    }

    fn flatten_node(node: &TreeNode, out: &mut Vec<FlatNode>) {
        out.push(FlatNode {
            depth: node.depth,
            kind: node.kind,
            expanded: node.expanded,
            name: node.name.clone(),
            path: node.path.clone(),
        });
        if node.kind == NodeKind::Directory && node.expanded {
            for child in &node.children {
                Self::flatten_node(child, out);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Navigation
    // -----------------------------------------------------------------------

    pub fn move_up(&mut self) {
        if self.mode == TreeMode::Filter {
            let list = &self.filtered_visible;
            if list.is_empty() {
                return;
            }
            if self.cursor > 0 {
                self.cursor -= 1;
            }
        } else if self.cursor > 0 {
            self.cursor -= 1;
        }
        self.adjust_scroll();
    }

    pub fn move_down(&mut self) {
        if self.mode == TreeMode::Filter {
            let len = self.filtered_visible.len();
            if len == 0 {
                return;
            }
            if self.cursor + 1 < len {
                self.cursor += 1;
            }
        } else if !self.visible.is_empty() && self.cursor + 1 < self.visible.len() {
            self.cursor += 1;
        }
        self.adjust_scroll();
    }

    fn adjust_scroll(&mut self) {
        // We'll use a fixed viewport; scroll_offset is adjusted in render based on height
    }

    /// Toggle expand on the currently selected directory, or open a file.
    /// Returns Some(path) if a file was selected.
    pub fn enter(&mut self) -> Option<PathBuf> {
        let node = self.current_flat_node()?;
        let path = node.path.clone();
        let kind = node.kind;

        if kind == NodeKind::File {
            return Some(path);
        }

        // Toggle expand
        self.toggle_expand();
        None
    }

    pub fn toggle_expand(&mut self) {
        let path = match self.current_flat_node() {
            Some(n) if n.kind == NodeKind::Directory => n.path.clone(),
            _ => return,
        };

        let ignored = self.ignored.clone();
        if let Some(tree_node) = self.find_node_mut(&path) {
            if tree_node.expanded {
                tree_node.expanded = false;
            } else {
                // Lazy load children
                if !tree_node.children_loaded {
                    tree_node.children = scan_dir(&tree_node.path, tree_node.depth + 1, &ignored);
                    tree_node.children_loaded = true;
                }
                tree_node.expanded = true;
            }
        }
        self.rebuild_visible();
    }

    /// Navigate to parent directory of current item.
    pub fn go_parent(&mut self) {
        let current_path = match self.current_flat_node() {
            Some(n) => n.path.clone(),
            None => return,
        };

        // If it's an expanded directory, collapse it instead
        if let Some(n) = self.current_flat_node()
            && n.kind == NodeKind::Directory
            && n.expanded
        {
            self.toggle_expand();
            return;
        }

        // Find parent path
        if let Some(parent) = current_path.parent() {
            let parent = parent.to_path_buf();
            // Find the parent in visible list
            for (i, flat) in self.visible.iter().enumerate() {
                if flat.path == parent {
                    self.cursor = i;
                    break;
                }
            }
        }
    }

    #[allow(dead_code)]
    pub fn selected_path(&self) -> Option<&Path> {
        self.current_flat_node().map(|n| n.path.as_path())
    }

    fn current_flat_node(&self) -> Option<&FlatNode> {
        if self.mode == TreeMode::Filter {
            let idx = *self.filtered_visible.get(self.cursor)?;
            self.visible.get(idx)
        } else {
            self.visible.get(self.cursor)
        }
    }

    // -----------------------------------------------------------------------
    // Filter mode
    // -----------------------------------------------------------------------

    pub fn start_filter(&mut self) {
        self.mode = TreeMode::Filter;
        self.filter_text.clear();
        self.filtered_visible = (0..self.visible.len()).collect();
        self.cursor = 0;
    }

    pub fn stop_filter(&mut self) {
        // If we have a filtered selection, map it back to the visible index
        if !self.filtered_visible.is_empty() {
            let real_idx = self.filtered_visible[self.cursor.min(self.filtered_visible.len() - 1)];
            self.cursor = real_idx;
        }
        self.mode = TreeMode::Normal;
        self.filter_text.clear();
        self.filtered_visible.clear();
    }

    pub fn filter_input(&mut self, ch: char) {
        self.filter_text.push(ch);
        self.update_filter();
    }

    pub fn filter_backspace(&mut self) {
        self.filter_text.pop();
        self.update_filter();
    }

    fn update_filter(&mut self) {
        if self.filter_text.is_empty() {
            self.filtered_visible = (0..self.visible.len()).collect();
        } else {
            let query: Vec<char> = self.filter_text.to_lowercase().chars().collect();
            self.filtered_visible = self
                .visible
                .iter()
                .enumerate()
                .filter(|(_, n)| {
                    let name_lower = n.name.to_lowercase();
                    fuzzy_contains(&name_lower, &query)
                })
                .map(|(i, _)| i)
                .collect();
        }
        self.cursor = 0;
    }

    // -----------------------------------------------------------------------
    // File operations
    // -----------------------------------------------------------------------

    pub fn start_new_file(&mut self) {
        self.mode = TreeMode::PromptNewFile;
        self.prompt_input.clear();
        self.prompt_cursor = 0;
    }

    pub fn start_new_dir(&mut self) {
        self.mode = TreeMode::PromptNewDir;
        self.prompt_input.clear();
        self.prompt_cursor = 0;
    }

    pub fn start_rename(&mut self) {
        if let Some(n) = self.current_flat_node() {
            if n.depth == 0 {
                return; // Don't rename root
            }
            self.prompt_input = n.name.clone();
            self.prompt_cursor = self.prompt_input.len();
            self.mode = TreeMode::PromptRename;
        }
    }

    pub fn start_delete(&mut self) {
        if let Some(n) = self.current_flat_node() {
            if n.depth == 0 {
                return; // Don't delete root
            }
            self.mode = TreeMode::ConfirmDelete;
        }
    }

    /// Get the parent directory for file operations (the selected dir, or parent of selected file).
    fn ops_parent_path(&self) -> Option<PathBuf> {
        let node = self.current_flat_node()?;
        if node.kind == NodeKind::Directory {
            Some(node.path.clone())
        } else {
            node.path.parent().map(|p| p.to_path_buf())
        }
    }

    pub fn create_file(&mut self) -> Result<Option<PathBuf>, String> {
        let name = self.prompt_input.trim().to_string();
        if name.is_empty() {
            self.mode = TreeMode::Normal;
            return Ok(None);
        }
        let parent = self.ops_parent_path().ok_or("No parent directory")?;
        let path = parent.join(&name);
        if path.exists() {
            return Err(format!("'{}' already exists", name));
        }
        // Create parent dirs if needed
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, "").map_err(|e| e.to_string())?;
        self.mode = TreeMode::Normal;
        self.refresh();
        Ok(Some(path))
    }

    pub fn create_dir(&mut self) -> Result<(), String> {
        let name = self.prompt_input.trim().to_string();
        if name.is_empty() {
            self.mode = TreeMode::Normal;
            return Ok(());
        }
        let parent = self.ops_parent_path().ok_or("No parent directory")?;
        let path = parent.join(&name);
        if path.exists() {
            return Err(format!("'{}' already exists", name));
        }
        std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
        self.mode = TreeMode::Normal;
        self.refresh();
        Ok(())
    }

    pub fn delete_node(&mut self) -> Result<(), String> {
        let node = self.current_flat_node().ok_or("Nothing selected")?;
        if node.depth == 0 {
            self.mode = TreeMode::Normal;
            return Err("Cannot delete root".to_string());
        }
        let path = node.path.clone();
        let kind = node.kind;
        if kind == NodeKind::Directory {
            std::fs::remove_dir_all(&path).map_err(|e| e.to_string())?;
        } else {
            std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
        self.mode = TreeMode::Normal;
        self.refresh();
        Ok(())
    }

    pub fn rename_node(&mut self) -> Result<(), String> {
        let new_name = self.prompt_input.trim().to_string();
        if new_name.is_empty() {
            self.mode = TreeMode::Normal;
            return Ok(());
        }
        let node = self.current_flat_node().ok_or("Nothing selected")?;
        if node.depth == 0 {
            self.mode = TreeMode::Normal;
            return Err("Cannot rename root".to_string());
        }
        let old_path = node.path.clone();
        let new_path = old_path.parent().ok_or("No parent")?.join(&new_name);
        if new_path.exists() {
            return Err(format!("'{}' already exists", new_name));
        }
        std::fs::rename(&old_path, &new_path).map_err(|e| e.to_string())?;
        self.mode = TreeMode::Normal;
        self.refresh();
        Ok(())
    }

    pub fn cancel_prompt(&mut self) {
        self.mode = TreeMode::Normal;
        self.prompt_input.clear();
        self.prompt_cursor = 0;
    }

    pub fn prompt_insert_char(&mut self, ch: char) {
        let mut buf = [0u8; 4];
        let s = ch.encode_utf8(&mut buf);
        self.prompt_input.insert_str(self.prompt_cursor, s);
        self.prompt_cursor += s.len();
    }

    pub fn prompt_backspace(&mut self) {
        if self.prompt_cursor > 0 {
            let before = &self.prompt_input[..self.prompt_cursor];
            if let Some(ch) = before.chars().next_back() {
                let len = ch.len_utf8();
                let new_pos = self.prompt_cursor - len;
                self.prompt_input.drain(new_pos..self.prompt_cursor);
                self.prompt_cursor = new_pos;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Refresh
    // -----------------------------------------------------------------------

    pub fn refresh(&mut self) {
        // Re-scan root
        self.root.children = scan_dir(&self.root_path, 1, &self.ignored);
        self.root.children_loaded = true;
        // Re-expand previously expanded dirs
        self.re_expand_dirs(&self.collect_expanded_paths());
        self.rebuild_visible();
    }

    fn collect_expanded_paths(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        Self::collect_expanded_recursive(&self.root, &mut paths);
        paths
    }

    fn collect_expanded_recursive(node: &TreeNode, paths: &mut Vec<PathBuf>) {
        if node.kind == NodeKind::Directory && node.expanded {
            paths.push(node.path.clone());
            for child in &node.children {
                Self::collect_expanded_recursive(child, paths);
            }
        }
    }

    fn re_expand_dirs(&mut self, expanded: &[PathBuf]) {
        Self::re_expand_recursive(&mut self.root, expanded, &self.ignored);
    }

    fn re_expand_recursive(node: &mut TreeNode, expanded: &[PathBuf], ignored: &[String]) {
        if node.kind != NodeKind::Directory {
            return;
        }
        if expanded.contains(&node.path) {
            node.expanded = true;
            if !node.children_loaded {
                node.children = scan_dir(&node.path, node.depth + 1, ignored);
                node.children_loaded = true;
            }
            for child in &mut node.children {
                Self::re_expand_recursive(child, expanded, ignored);
            }
        } else {
            node.expanded = false;
        }
    }

    // -----------------------------------------------------------------------
    // Tree node lookup
    // -----------------------------------------------------------------------

    fn find_node_mut(&mut self, path: &Path) -> Option<&mut TreeNode> {
        Self::find_in_node(&mut self.root, path)
    }

    fn find_in_node<'a>(node: &'a mut TreeNode, path: &Path) -> Option<&'a mut TreeNode> {
        if node.path == path {
            return Some(node);
        }
        for child in &mut node.children {
            if let Some(found) = Self::find_in_node(child, path) {
                return Some(found);
            }
        }
        None
    }

    // -----------------------------------------------------------------------
    // Rendering
    // -----------------------------------------------------------------------

    pub fn render(&mut self, screen: &mut Screen, height: usize, is_focused: bool) {
        let w = self.width as usize;
        if w == 0 || height == 0 {
            return;
        }

        let title_fg = if is_focused {
            Color::Ansi(6) // cyan
        } else {
            Color::Ansi(7)
        };
        let title_bg = Color::Color256(236);
        let tree_bg = Color::Default;
        let dir_fg = Color::Ansi(4); // blue
        let file_fg = Color::Default;
        let cursor_fg = Color::Ansi(0); // black
        let cursor_bg = Color::Ansi(7); // white (reverse)
        let border_fg = Color::Color256(240);

        // Row 0: title bar
        let title = &self.root.name;
        for col in 0..w.saturating_sub(1) {
            screen.put_char(0, col, ' ', title_fg, title_bg, false);
        }
        let display_title: String = title.chars().take(w.saturating_sub(2)).collect();
        screen.put_str(0, 1, &display_title, title_fg, title_bg, true);
        // Right border
        screen.put_char(0, w - 1, '\u{2502}', border_fg, Color::Default, false);

        // Determine which items to render based on mode
        let items: Vec<usize> = if self.mode == TreeMode::Filter {
            self.filtered_visible.clone()
        } else {
            (0..self.visible.len()).collect()
        };

        // Calculate available rows for tree content
        let tree_start_row = 1;
        let prompt_rows = match self.mode {
            TreeMode::Filter
            | TreeMode::PromptNewFile
            | TreeMode::PromptNewDir
            | TreeMode::PromptRename
            | TreeMode::ConfirmDelete => 1,
            _ => 0,
        };
        let tree_height = height.saturating_sub(tree_start_row + prompt_rows);

        // Adjust scroll
        if !items.is_empty() {
            let cursor = self.cursor.min(items.len() - 1);
            self.cursor = cursor;
            if cursor < self.scroll_offset {
                self.scroll_offset = cursor;
            } else if cursor >= self.scroll_offset + tree_height {
                self.scroll_offset = cursor.saturating_sub(tree_height - 1);
            }
        } else {
            self.scroll_offset = 0;
        }

        // Render tree rows
        for row_i in 0..tree_height {
            let screen_row = tree_start_row + row_i;
            let item_idx = self.scroll_offset + row_i;

            // Clear row
            for col in 0..w.saturating_sub(1) {
                screen.put_char(screen_row, col, ' ', file_fg, tree_bg, false);
            }

            if item_idx < items.len() {
                let visible_idx = items[item_idx];
                if visible_idx < self.visible.len() {
                    let node = &self.visible[visible_idx];
                    let is_cursor = item_idx == self.cursor;

                    let indent = node.depth * 2;
                    let (fg, bg) = if is_cursor && is_focused {
                        (cursor_fg, cursor_bg)
                    } else if node.kind == NodeKind::Directory {
                        (dir_fg, tree_bg)
                    } else {
                        (file_fg, tree_bg)
                    };

                    // Fill bg if cursor row
                    if is_cursor && is_focused {
                        for col in 0..w.saturating_sub(1) {
                            screen.put_char(screen_row, col, ' ', fg, bg, false);
                        }
                    }

                    let mut col = 0;

                    // Indent
                    let indent_chars = indent.min(w.saturating_sub(3));
                    col += indent_chars;

                    // Directory indicator or space
                    if col < w.saturating_sub(2) {
                        if node.kind == NodeKind::Directory {
                            let indicator = if node.expanded {
                                '\u{25BC}' // ▼
                            } else {
                                '\u{25B6}' // ▶
                            };
                            screen.put_char(screen_row, col, indicator, fg, bg, false);
                            col += 1;
                            if col < w.saturating_sub(1) {
                                screen.put_char(screen_row, col, ' ', fg, bg, false);
                                col += 1;
                            }
                        } else {
                            // File: extra indent to align with dir names
                            if col + 2 < w.saturating_sub(1) {
                                col += 2;
                            }
                        }
                    }

                    // Name
                    let name_display = if node.kind == NodeKind::Directory {
                        format!("{}/", node.name)
                    } else {
                        node.name.clone()
                    };

                    let max_name = w.saturating_sub(1).saturating_sub(col);
                    let mut char_count = 0;
                    for ch in name_display.chars() {
                        let cw = crate::unicode::char_width(ch);
                        if char_count + cw > max_name {
                            break;
                        }
                        screen.put_char(screen_row, col + char_count, ch, fg, bg, false);
                        char_count += cw;
                    }
                }
            }

            // Right border
            screen.put_char(
                screen_row,
                w - 1,
                '\u{2502}',
                border_fg,
                Color::Default,
                false,
            );
        }

        // Prompt row at bottom of sidebar
        if prompt_rows > 0 {
            let prompt_row = tree_start_row + tree_height;
            let prompt_fg = Color::Ansi(3); // yellow
            let prompt_bg = Color::Color256(236);

            for col in 0..w.saturating_sub(1) {
                screen.put_char(prompt_row, col, ' ', prompt_fg, prompt_bg, false);
            }

            let label = match self.mode {
                TreeMode::Filter => "/",
                TreeMode::PromptNewFile => "New: ",
                TreeMode::PromptNewDir => "Dir: ",
                TreeMode::PromptRename => "Rename: ",
                TreeMode::ConfirmDelete => "Delete? y/n",
                _ => "",
            };

            screen.put_str(prompt_row, 0, label, prompt_fg, prompt_bg, true);

            if self.mode != TreeMode::ConfirmDelete {
                let input = match self.mode {
                    TreeMode::Filter => &self.filter_text,
                    _ => &self.prompt_input,
                };
                let input_start = crate::unicode::str_width(label);
                let max_input = w.saturating_sub(1).saturating_sub(input_start);
                let display_input: String = input.chars().take(max_input).collect();
                screen.put_str(
                    prompt_row,
                    input_start,
                    &display_input,
                    Color::Default,
                    prompt_bg,
                    false,
                );
            }

            // Right border
            screen.put_char(
                prompt_row,
                w - 1,
                '\u{2502}',
                border_fg,
                Color::Default,
                false,
            );
        }

        // Fill remaining rows (if sidebar is taller)
        let used_rows = tree_start_row + tree_height + prompt_rows;
        for row in used_rows..height {
            for col in 0..w.saturating_sub(1) {
                screen.put_char(row, col, ' ', file_fg, tree_bg, false);
            }
            screen.put_char(row, w - 1, '\u{2502}', border_fg, Color::Default, false);
        }
    }
}

// ---------------------------------------------------------------------------
// Directory scanning
// ---------------------------------------------------------------------------

fn scan_dir(path: &Path, depth: usize, ignored: &[String]) -> Vec<TreeNode> {
    let entries = match std::fs::read_dir(path) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut dirs = Vec::new();
    let mut files = Vec::new();

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip ignored
        if ignored.iter().any(|ig| ig == &name) {
            continue;
        }

        let entry_path = entry.path();

        // Skip symlinks pointing to ancestors (cycle prevention)
        if entry_path.is_symlink()
            && let Ok(target) = std::fs::read_link(&entry_path)
        {
            let target_abs = if target.is_absolute() {
                target
            } else {
                entry_path.parent().unwrap_or(path).join(&target)
            };
            if let (Ok(target_canon), Ok(path_canon)) = (
                std::fs::canonicalize(&target_abs),
                std::fs::canonicalize(path),
            ) && path_canon.starts_with(&target_canon)
            {
                continue;
            }
        }

        let is_dir = entry_path.is_dir();

        let node = TreeNode {
            name,
            path: entry_path,
            kind: if is_dir {
                NodeKind::Directory
            } else {
                NodeKind::File
            },
            depth,
            expanded: false,
            children: Vec::new(),
            children_loaded: false,
        };

        if is_dir {
            dirs.push(node);
        } else {
            files.push(node);
        }
    }

    // Sort: dirs first, then files, case-insensitive
    dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    dirs.extend(files);
    dirs
}

// ---------------------------------------------------------------------------
// Fuzzy contains (simple subsequence match)
// ---------------------------------------------------------------------------

fn fuzzy_contains(haystack: &str, needle: &[char]) -> bool {
    let mut ni = 0;
    for ch in haystack.chars() {
        if ni < needle.len() && ch == needle[ni] {
            ni += 1;
        }
    }
    ni == needle.len()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fuzzy_contains_basic() {
        assert!(fuzzy_contains("hello", &['h', 'l', 'o']));
        assert!(!fuzzy_contains("hello", &['x']));
        assert!(fuzzy_contains("filetree", &['f', 't']));
    }

    #[test]
    fn test_scan_dir_current() {
        // Test scanning the project root
        let nodes = scan_dir(
            Path::new("."),
            0,
            &["target".to_string(), ".git".to_string()],
        );
        // Should find at least src/ and Cargo.toml
        let has_src = nodes
            .iter()
            .any(|n| n.name == "src" && n.kind == NodeKind::Directory);
        let has_cargo = nodes
            .iter()
            .any(|n| n.name == "Cargo.toml" && n.kind == NodeKind::File);
        assert!(has_src, "should find src/");
        assert!(has_cargo, "should find Cargo.toml");

        // Dirs should come before files
        let first_file_idx = nodes.iter().position(|n| n.kind == NodeKind::File);
        let last_dir_idx = nodes.iter().rposition(|n| n.kind == NodeKind::Directory);
        if let (Some(fi), Some(di)) = (first_file_idx, last_dir_idx) {
            assert!(di < fi, "directories should be sorted before files");
        }
    }

    #[test]
    fn test_filetree_new() {
        let ft = FileTree::new(std::env::current_dir().unwrap(), 30, &[]);
        assert!(!ft.visible.is_empty());
        assert_eq!(ft.visible[0].depth, 0); // root node
        assert_eq!(ft.visible[0].kind, NodeKind::Directory);
    }
}
