// ---------------------------------------------------------------------------
// Layout & Pane System
// ---------------------------------------------------------------------------
//
// Recursive layout tree that resolves into screen rectangles.
// Each leaf holds a PaneId and a buffer index; splits divide space
// between children either horizontally (left|right) or vertically
// (top|bottom).

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PaneId(pub u32);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaneContent {
    Buffer(usize),
    Terminal(usize),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SplitDir {
    Horizontal, // children go left | right
    Vertical,   // children go top | bottom
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    pub fn contains(&self, col: u16, row: u16) -> bool {
        col >= self.x && col < self.x + self.width && row >= self.y && row < self.y + self.height
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

// ---------------------------------------------------------------------------
// Layout tree
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum LayoutNode {
    Leaf {
        id: PaneId,
        content: PaneContent,
    },
    Split {
        dir: SplitDir,
        children: Vec<LayoutNode>,
        /// Fractional ratios (sum to 1.0). Same length as `children`.
        ratios: Vec<f64>,
    },
}

// ---------------------------------------------------------------------------
// Resolved pane info (pane id → rect + buffer)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct PaneInfo {
    pub id: PaneId,
    pub rect: Rect,
    pub content: PaneContent,
}

impl PaneInfo {
    /// Convenience: get buffer_index if this pane holds a buffer.
    #[allow(dead_code)]
    pub fn buffer_index(&self) -> Option<usize> {
        match self.content {
            PaneContent::Buffer(idx) => Some(idx),
            PaneContent::Terminal(_) => None,
        }
    }
}

// ---------------------------------------------------------------------------
// LayoutState
// ---------------------------------------------------------------------------

pub struct LayoutState {
    root: LayoutNode,
    next_id: u32,
    resolved: Vec<PaneInfo>,
}

impl LayoutState {
    /// Create a layout with a single pane showing the given buffer index.
    pub fn new(buffer_index: usize) -> Self {
        let id = PaneId(0);
        Self {
            root: LayoutNode::Leaf {
                id,
                content: PaneContent::Buffer(buffer_index),
            },
            next_id: 1,
            resolved: Vec::new(),
        }
    }

    /// Resolve the tree into concrete screen rectangles.
    pub fn resolve(&mut self, total: Rect) {
        self.resolved.clear();
        resolve_node(&self.root, total, &mut self.resolved);
    }

    /// Get the resolved pane list (call after `resolve`).
    pub fn panes(&self) -> &[PaneInfo] {
        &self.resolved
    }

    /// Number of leaf panes.
    pub fn pane_count(&self) -> usize {
        self.resolved.len()
    }

    /// Look up the rect for a pane.
    pub fn pane_rect(&self, id: PaneId) -> Option<Rect> {
        self.resolved.iter().find(|p| p.id == id).map(|p| p.rect)
    }

    /// Look up the buffer index for a pane (returns Some only for Buffer panes).
    pub fn pane_buffer(&self, id: PaneId) -> Option<usize> {
        self.resolved
            .iter()
            .find(|p| p.id == id)
            .and_then(|p| match p.content {
                PaneContent::Buffer(idx) => Some(idx),
                PaneContent::Terminal(_) => None,
            })
    }

    /// Look up the content for a pane.
    pub fn pane_content(&self, id: PaneId) -> Option<PaneContent> {
        self.resolved.iter().find(|p| p.id == id).map(|p| p.content)
    }

    /// Set the buffer index for a pane.
    pub fn set_pane_buffer(&mut self, id: PaneId, buffer_index: usize) {
        set_content_in_node(&mut self.root, id, PaneContent::Buffer(buffer_index));
        if let Some(info) = self.resolved.iter_mut().find(|p| p.id == id) {
            info.content = PaneContent::Buffer(buffer_index);
        }
    }

    /// Set the content for a pane.
    #[allow(dead_code)]
    pub fn set_pane_content(&mut self, id: PaneId, content: PaneContent) {
        set_content_in_node(&mut self.root, id, content);
        if let Some(info) = self.resolved.iter_mut().find(|p| p.id == id) {
            info.content = content;
        }
    }

    /// Split a leaf pane into two. Returns the id of the new pane.
    /// The original pane keeps its content; the new pane gets `new_buffer_index`.
    pub fn split_pane(
        &mut self,
        pane_id: PaneId,
        dir: SplitDir,
        new_buffer_index: usize,
    ) -> Option<PaneId> {
        self.split_pane_with_content(pane_id, dir, PaneContent::Buffer(new_buffer_index))
    }

    /// Split a leaf pane into two with arbitrary content for the new pane.
    pub fn split_pane_with_content(
        &mut self,
        pane_id: PaneId,
        dir: SplitDir,
        new_content: PaneContent,
    ) -> Option<PaneId> {
        let new_id = PaneId(self.next_id);
        self.next_id += 1;
        if split_node(&mut self.root, pane_id, dir, new_id, new_content) {
            Some(new_id)
        } else {
            self.next_id -= 1;
            None
        }
    }

    /// Wrap the entire layout tree in a new `Split(Vertical)`, placing the
    /// current content as the top child and a new leaf with `content` at the
    /// bottom.  Initial ratio is 50/50; the caller may resize afterwards.
    /// Returns the id of the new bottom leaf.
    pub fn wrap_root_bottom(&mut self, content: PaneContent) -> PaneId {
        let new_id = PaneId(self.next_id);
        self.next_id += 1;
        // Swap old root out with a temporary placeholder, then build the new tree.
        let old_root = std::mem::replace(
            &mut self.root,
            LayoutNode::Leaf {
                id: PaneId(u32::MAX),
                content: PaneContent::Buffer(0),
            },
        );
        self.root = LayoutNode::Split {
            dir: SplitDir::Vertical,
            children: vec![old_root, LayoutNode::Leaf { id: new_id, content }],
            ratios: vec![0.5, 0.5],
        };
        new_id
    }

    /// Close a pane. Returns true if closed. If it's the last pane, returns false.
    pub fn close_pane(&mut self, pane_id: PaneId) -> bool {
        if self.leaf_count(&self.root) <= 1 {
            return false;
        }
        close_node(&mut self.root, pane_id)
    }

    /// Find the first leaf pane id.
    pub fn first_pane(&self) -> PaneId {
        first_leaf(&self.root)
    }

    /// Find an adjacent pane in the given direction (based on resolved rects).
    pub fn adjacent_pane(&self, from: PaneId, direction: Direction) -> Option<PaneId> {
        let from_rect = self.pane_rect(from)?;
        let (cx, cy) = rect_center(&from_rect);

        let mut best: Option<(PaneId, i32)> = None;
        for info in &self.resolved {
            if info.id == from {
                continue;
            }
            let (ox, oy) = rect_center(&info.rect);
            let ok = match direction {
                Direction::Left => ox < cx && info.rect.x + info.rect.width <= from_rect.x,
                Direction::Right => ox > cx && info.rect.x >= from_rect.x + from_rect.width,
                Direction::Up => oy < cy && info.rect.y + info.rect.height <= from_rect.y,
                Direction::Down => oy > cy && info.rect.y >= from_rect.y + from_rect.height,
            };
            if !ok {
                continue;
            }
            let dist = match direction {
                Direction::Left | Direction::Right => (ox - cx).abs() * 2 + (oy - cy).abs(),
                Direction::Up | Direction::Down => (oy - cy).abs() * 2 + (ox - cx).abs(),
            };
            if best.is_none() || dist < best.unwrap().1 {
                best = Some((info.id, dist));
            }
        }
        best.map(|(id, _)| id)
    }

    /// Resize the split that contains `pane_id` by shifting ratios.
    /// `delta` is in screen units (cols or rows depending on split direction).
    /// Positive = grow the pane, negative = shrink.
    /// `axis` restricts which splits are adjusted:
    /// - `Horizontal` → Left/Right keys move vertical boundaries between side-by-side panes.
    /// - `Vertical`   → Up/Down keys move horizontal boundaries between stacked panes.
    pub fn resize_split(&mut self, pane_id: PaneId, delta: i16, axis: SplitDir, total: Rect) {
        resize_in_node(&mut self.root, pane_id, delta, axis, total);
    }

    /// Check if a pane id exists.
    pub fn pane_exists(&self, id: PaneId) -> bool {
        find_leaf(&self.root, id)
    }

    /// Collect all pane ids that reference the given buffer index.
    #[allow(dead_code)]
    pub fn panes_with_buffer(&self, buffer_index: usize) -> Vec<PaneId> {
        let mut result = Vec::new();
        collect_panes_with_buffer(&self.root, buffer_index, &mut result);
        result
    }

    /// Update all buffer panes: decrement indices > removed_index, redirect removed to previous.
    pub fn adjust_buffer_indices_after_remove(&mut self, removed_index: usize) {
        adjust_indices(&mut self.root, removed_index);
    }

    /// Get buffer index for a pane, if it's a buffer pane.
    #[allow(dead_code)]
    pub fn pane_buffer_index(&self, id: PaneId) -> Option<usize> {
        self.pane_buffer(id)
    }

    fn leaf_count(&self, node: &LayoutNode) -> usize {
        leaf_count_recursive(node)
    }
}

// ---------------------------------------------------------------------------
// Tree operations (recursive helpers)
// ---------------------------------------------------------------------------

fn leaf_count_recursive(node: &LayoutNode) -> usize {
    match node {
        LayoutNode::Leaf { .. } => 1,
        LayoutNode::Split { children, .. } => children.iter().map(leaf_count_recursive).sum(),
    }
}

fn resolve_node(node: &LayoutNode, rect: Rect, out: &mut Vec<PaneInfo>) {
    match node {
        LayoutNode::Leaf { id, content } => {
            out.push(PaneInfo {
                id: *id,
                rect,
                content: *content,
            });
        }
        LayoutNode::Split {
            dir,
            children,
            ratios,
        } => {
            let total_size = match dir {
                SplitDir::Horizontal => rect.width as f64,
                SplitDir::Vertical => rect.height as f64,
            };
            // We reserve 1 col/row for each border between children
            let border_count = children.len().saturating_sub(1);
            let usable = (total_size - border_count as f64).max(0.0);

            let mut offset = 0u16;
            for (i, (child, ratio)) in children.iter().zip(ratios.iter()).enumerate() {
                let is_last = i == children.len() - 1;
                let size = if is_last {
                    // Give the last child whatever remains to avoid rounding gaps
                    let total_dim = match dir {
                        SplitDir::Horizontal => rect.width,
                        SplitDir::Vertical => rect.height,
                    };
                    total_dim.saturating_sub(offset)
                } else {
                    let raw = (usable * ratio).round() as u16;
                    // +1 for the border after this child
                    raw + 1
                };

                let child_rect = match dir {
                    SplitDir::Horizontal => Rect {
                        x: rect.x + offset,
                        y: rect.y,
                        width: if is_last {
                            size
                        } else {
                            size.saturating_sub(1)
                        },
                        height: rect.height,
                    },
                    SplitDir::Vertical => Rect {
                        x: rect.x,
                        y: rect.y + offset,
                        width: rect.width,
                        height: if is_last {
                            size
                        } else {
                            size.saturating_sub(1)
                        },
                    },
                };
                resolve_node(child, child_rect, out);
                offset += size;
            }
        }
    }
}

fn split_node(
    node: &mut LayoutNode,
    target: PaneId,
    dir: SplitDir,
    new_id: PaneId,
    new_content: PaneContent,
) -> bool {
    match node {
        LayoutNode::Leaf { id, content } if *id == target => {
            let old_id = *id;
            let old_content = *content;
            *node = LayoutNode::Split {
                dir,
                children: vec![
                    LayoutNode::Leaf {
                        id: old_id,
                        content: old_content,
                    },
                    LayoutNode::Leaf {
                        id: new_id,
                        content: new_content,
                    },
                ],
                ratios: vec![0.5, 0.5],
            };
            true
        }
        LayoutNode::Leaf { .. } => false,
        LayoutNode::Split { children, .. } => {
            for child in children.iter_mut() {
                if split_node(child, target, dir, new_id, new_content) {
                    return true;
                }
            }
            false
        }
    }
}

fn close_node(node: &mut LayoutNode, target: PaneId) -> bool {
    match node {
        LayoutNode::Leaf { .. } => false,
        LayoutNode::Split {
            children, ratios, ..
        } => {
            // Check if any direct child is the target leaf
            if let Some(idx) = children
                .iter()
                .position(|c| matches!(c, LayoutNode::Leaf { id, .. } if *id == target))
            {
                children.remove(idx);
                ratios.remove(idx);
                // Normalize ratios
                let sum: f64 = ratios.iter().sum();
                if sum > 0.0 {
                    for r in ratios.iter_mut() {
                        *r /= sum;
                    }
                }
                // If only one child remains, collapse
                if children.len() == 1 {
                    let remaining = children.remove(0);
                    *node = remaining;
                }
                return true;
            }
            // Recurse into children
            for child in children.iter_mut() {
                if close_node(child, target) {
                    return true;
                }
            }
            false
        }
    }
}

fn first_leaf(node: &LayoutNode) -> PaneId {
    match node {
        LayoutNode::Leaf { id, .. } => *id,
        LayoutNode::Split { children, .. } => first_leaf(&children[0]),
    }
}

fn find_leaf(node: &LayoutNode, target: PaneId) -> bool {
    match node {
        LayoutNode::Leaf { id, .. } => *id == target,
        LayoutNode::Split { children, .. } => children.iter().any(|c| find_leaf(c, target)),
    }
}

fn set_content_in_node(node: &mut LayoutNode, target: PaneId, new_content: PaneContent) {
    match node {
        LayoutNode::Leaf { id, content } if *id == target => {
            *content = new_content;
        }
        LayoutNode::Leaf { .. } => {}
        LayoutNode::Split { children, .. } => {
            for child in children.iter_mut() {
                set_content_in_node(child, target, new_content);
            }
        }
    }
}

#[allow(dead_code)]
fn collect_panes_with_buffer(node: &LayoutNode, buf_idx: usize, out: &mut Vec<PaneId>) {
    match node {
        LayoutNode::Leaf {
            id,
            content: PaneContent::Buffer(bi),
        } if *bi == buf_idx => {
            out.push(*id);
        }
        LayoutNode::Leaf { .. } => {}
        LayoutNode::Split { children, .. } => {
            for child in children {
                collect_panes_with_buffer(child, buf_idx, out);
            }
        }
    }
}

fn adjust_indices(node: &mut LayoutNode, removed: usize) {
    match node {
        LayoutNode::Leaf {
            content: PaneContent::Buffer(bi),
            ..
        } => {
            if *bi == removed {
                *bi = removed.saturating_sub(1);
            } else if *bi > removed {
                *bi -= 1;
            }
        }
        LayoutNode::Leaf { .. } => {} // Terminal panes: no index adjustment
        LayoutNode::Split { children, .. } => {
            for child in children.iter_mut() {
                adjust_indices(child, removed);
            }
        }
    }
}

fn resize_in_node(node: &mut LayoutNode, target: PaneId, delta: i16, axis: SplitDir, total: Rect) {
    match node {
        LayoutNode::Leaf { .. } => {}
        LayoutNode::Split {
            dir,
            children,
            ratios,
        } => {
            // Find which child contains the target
            let idx = children.iter().position(|c| find_leaf(c, target));
            if let Some(idx) = idx {
                // Only adjust this split when its axis matches the key direction.
                // Horizontal splits (side-by-side) → Left/Right; Vertical (stacked) → Up/Down.
                if children.len() >= 2 && *dir == axis {
                    let total_size = match dir {
                        SplitDir::Horizontal => total.width as f64,
                        SplitDir::Vertical => total.height as f64,
                    };
                    if total_size > 0.0 {
                        let ratio_delta = delta as f64 / total_size;
                        // Grow this child, shrink the next (or previous if last)
                        let other = if idx + 1 < children.len() {
                            idx + 1
                        } else {
                            idx.saturating_sub(1)
                        };
                        if other != idx {
                            ratios[idx] = (ratios[idx] + ratio_delta).clamp(0.1, 0.9);
                            ratios[other] = (ratios[other] - ratio_delta).clamp(0.1, 0.9);
                            // Normalize so ratios always sum to 1.0
                            let sum: f64 = ratios.iter().sum();
                            for r in ratios.iter_mut() {
                                *r /= sum;
                            }
                        }
                    }
                }
                // Always recurse so nested splits of the correct axis are reachable.
                resize_in_node(&mut children[idx], target, delta, axis, total);
            }
        }
    }
}

fn rect_center(r: &Rect) -> (i32, i32) {
    (
        r.x as i32 + r.width as i32 / 2,
        r.y as i32 + r.height as i32 / 2,
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_pane_resolves_to_full_rect() {
        let mut layout = LayoutState::new(0);
        let total = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        };
        layout.resolve(total);
        assert_eq!(layout.pane_count(), 1);
        assert_eq!(layout.panes()[0].rect, total);
        assert_eq!(layout.panes()[0].content, PaneContent::Buffer(0));
    }

    #[test]
    fn split_creates_two_panes() {
        let mut layout = LayoutState::new(0);
        let first = layout.first_pane();
        let new = layout.split_pane(first, SplitDir::Horizontal, 1).unwrap();

        let total = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        };
        layout.resolve(total);
        assert_eq!(layout.pane_count(), 2);

        let r1 = layout.pane_rect(first).unwrap();
        let r2 = layout.pane_rect(new).unwrap();

        // Both panes should fit within total width
        assert!(r1.width + r2.width < total.width + 2); // +border
        assert_eq!(r1.height, total.height);
        assert_eq!(r2.height, total.height);
        // Left pane starts at 0
        assert_eq!(r1.x, 0);
    }

    #[test]
    fn close_pane_collapses() {
        let mut layout = LayoutState::new(0);
        let first = layout.first_pane();
        let new = layout.split_pane(first, SplitDir::Horizontal, 1).unwrap();

        assert!(layout.close_pane(new));
        let total = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        };
        layout.resolve(total);
        assert_eq!(layout.pane_count(), 1);
        assert_eq!(layout.panes()[0].rect, total);
    }

    #[test]
    fn cannot_close_last_pane() {
        let mut layout = LayoutState::new(0);
        let first = layout.first_pane();
        assert!(!layout.close_pane(first));
    }

    #[test]
    fn adjacent_pane_horizontal() {
        let mut layout = LayoutState::new(0);
        let first = layout.first_pane();
        let second = layout.split_pane(first, SplitDir::Horizontal, 1).unwrap();
        let total = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        };
        layout.resolve(total);

        assert_eq!(layout.adjacent_pane(first, Direction::Right), Some(second));
        assert_eq!(layout.adjacent_pane(second, Direction::Left), Some(first));
        assert_eq!(layout.adjacent_pane(first, Direction::Left), None);
        assert_eq!(layout.adjacent_pane(second, Direction::Right), None);
    }

    #[test]
    fn set_pane_buffer() {
        let mut layout = LayoutState::new(0);
        let first = layout.first_pane();
        layout.set_pane_buffer(first, 5);
        let total = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        };
        layout.resolve(total);
        assert_eq!(layout.pane_buffer(first), Some(5));
    }

    #[test]
    fn vertical_split() {
        let mut layout = LayoutState::new(0);
        let first = layout.first_pane();
        let second = layout.split_pane(first, SplitDir::Vertical, 1).unwrap();
        let total = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        };
        layout.resolve(total);
        assert_eq!(layout.pane_count(), 2);

        let r1 = layout.pane_rect(first).unwrap();
        let r2 = layout.pane_rect(second).unwrap();

        assert_eq!(r1.width, total.width);
        assert_eq!(r2.width, total.width);
        assert!(r1.height + r2.height < total.height + 2);
    }

    #[test]
    fn rect_contains() {
        let r = Rect {
            x: 10,
            y: 5,
            width: 20,
            height: 10,
        };
        assert!(r.contains(10, 5));
        assert!(r.contains(29, 14));
        assert!(!r.contains(30, 5));
        assert!(!r.contains(10, 15));
        assert!(!r.contains(9, 5));
    }

    #[test]
    fn adjust_buffer_indices() {
        let mut layout = LayoutState::new(2);
        let first = layout.first_pane();
        layout.split_pane(first, SplitDir::Horizontal, 3).unwrap();
        layout.adjust_buffer_indices_after_remove(1);
        let total = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        };
        layout.resolve(total);
        // buffer 2 -> 1, buffer 3 -> 2
        assert_eq!(layout.pane_buffer(first), Some(1));
    }

    #[test]
    fn panes_with_buffer() {
        let mut layout = LayoutState::new(0);
        let first = layout.first_pane();
        let second = layout.split_pane(first, SplitDir::Horizontal, 0).unwrap();
        let total = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        };
        layout.resolve(total);
        let panes = layout.panes_with_buffer(0);
        assert_eq!(panes.len(), 2);
        assert!(panes.contains(&first));
        assert!(panes.contains(&second));
    }
}
