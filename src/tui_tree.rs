use serde_json::{Map, Value};

/// One breadcrumb segment from the metadata root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Key(String),
    Index(usize),
}

/// One visible row: a child of the currently drilled-into node.
pub struct Row {
    pub label: String,
    pub segment: Segment,
    pub is_branch: bool,
    pub preview: String,
}

/// An edit applied at a display path (segments from the root).
pub enum Edit {
    /// Replace the node at `path` (a leaf, a whole subtree, or a new or
    /// existing object key).
    Set(Value),
    /// Delete the node at `path` (an object member, a whole section, or an
    /// array element).
    Delete,
    /// Append to the array at `path`.
    Append(Value),
}

/// The metadata tree: `stored` is the exact on-disk shape (no traversal),
/// `display` is the traversed form the user browses and edits. Edits are
/// expressed as `--set` payloads computed against `stored`, so embedded JSON
/// documents keep their string-carrier shape across writes.
pub struct MetaTree {
    stored: Value,
    display: Value,
    drill: Vec<Segment>,
    cursor: usize,
}

impl MetaTree {
    pub fn new(stored: Value) -> Self {
        let mut display = stored.clone();
        crate::meta::traverse_value(&mut display);
        Self { stored, display, drill: Vec::new(), cursor: 0 }
    }

    pub fn drill(&self) -> &[Segment] {
        &self.drill
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let rows = self.rows().len();
        if rows == 0 {
            self.cursor = 0;
            return;
        }
        let next = self.cursor as isize + delta;
        self.cursor = next.clamp(0, rows as isize - 1) as usize;
    }

    pub fn set_cursor(&mut self, index: usize) {
        self.cursor = index.min(self.rows().len().saturating_sub(1));
    }

    /// The display node at the current drill level (the root when the drill
    /// path no longer resolves, e.g. right after a reload).
    pub fn current_node(&self) -> &Value {
        walk(&self.display, &self.drill).unwrap_or(&self.display)
    }

    /// The children of the current node; object keys are sorted so the order
    /// never depends on encoder output order.
    pub fn rows(&self) -> Vec<Row> {
        match self.current_node() {
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                keys.into_iter()
                    .map(|key| {
                        let value = &map[key];
                        Row { label: key.clone(), segment: Segment::Key(key.clone()), is_branch: value.is_object() || value.is_array(), preview: preview(value) }
                    })
                    .collect()
            }
            Value::Array(items) => items.iter().enumerate().map(|(index, value)| Row { label: format!("[{index}]"), segment: Segment::Index(index), is_branch: value.is_object() || value.is_array(), preview: preview(value) }).collect(),
            _ => Vec::new(),
        }
    }

    /// The full display path and row of the current cursor position, if any.
    pub fn selected(&self) -> Option<(Vec<Segment>, Row)> {
        let rows = self.rows();
        let row = rows.into_iter().nth(self.cursor)?;
        let mut path = self.drill.clone();
        path.push(row.segment.clone());
        Some((path, row))
    }

    pub fn value_at(&self, path: &[Segment]) -> Option<&Value> {
        walk(&self.display, path)
    }

    /// Drill into the selected branch. Returns false when the selection is a
    /// leaf (there is nowhere to drill) or there is no selection.
    pub fn drill_into_selected(&mut self) -> bool {
        let Some((path, row)) = self.selected() else {
            return false;
        };
        if !row.is_branch {
            return false;
        }
        self.drill = path;
        self.cursor = 0;
        true
    }

    /// Go back up one level. Returns false when already at the root.
    pub fn drill_up(&mut self) -> bool {
        if self.drill.pop().is_none() {
            return false;
        }
        self.cursor = 0;
        true
    }

    /// The panel header: `Metadata` at the root, otherwise the breadcrumb
    /// path, with array indices bracketed onto their parent segment.
    pub fn breadcrumb(&self) -> String {
        let mut out = String::from("Metadata");
        for segment in &self.drill {
            match segment {
                Segment::Key(key) => {
                    out.push_str(" > ");
                    out.push_str(key);
                }
                Segment::Index(index) => out.push_str(&format!("[{index}]")),
            }
        }
        out
    }

    /// Restore a drill path after a reload, dropping trailing segments that no
    /// longer resolve, and clamp the cursor into range.
    pub fn restore(&mut self, drill: Vec<Segment>, cursor: usize) {
        let mut kept = Vec::new();
        for segment in drill {
            kept.push(segment);
            if walk(&self.display, &kept).is_none() {
                kept.pop();
                break;
            }
        }
        self.drill = kept;
        self.set_cursor(cursor);
    }

    /// Depth-first search over keys and scalar values (case-insensitive
    /// substring), in the same order the tree displays. Returns matching
    /// display paths.
    pub fn search(&self, needle: &str) -> Vec<Vec<Segment>> {
        let needle = needle.to_lowercase();
        let mut out = Vec::new();
        let mut path = Vec::new();
        search_node(&self.display, &needle, &mut path, &mut out);
        out
    }

    /// Drill to the parent of `path` and select its last segment.
    pub fn jump_to(&mut self, path: &[Segment]) {
        let Some((last, parent)) = path.split_last() else {
            self.drill.clear();
            self.cursor = 0;
            return;
        };
        self.drill = parent.to_vec();
        self.cursor = self.rows().iter().position(|row| &row.segment == last).unwrap_or(0);
    }

    /// Build a `--set` payload (`{"exif"|"custom": ...}`) performing `edit` at
    /// `path`. Structured edits express the minimal nested fragment; array
    /// edits rebuild the full array (merge replaces arrays wholesale, keeping
    /// untouched siblings in stored form); edits passing through an embedded
    /// JSON string rewrite that ancestor as a re-serialized string.
    pub fn build_edit_payload(&self, path: &[Segment], edit: &Edit) -> Value {
        build_fragment(&self.stored, &self.display, path, edit)
    }
}

/// The dot/bracket path of a display path, e.g. `custom.workflow.nodes[3]`.
pub fn dot_path(path: &[Segment]) -> String {
    let mut out = String::new();
    for segment in path {
        match segment {
            Segment::Key(key) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(key);
            }
            Segment::Index(index) => out.push_str(&format!("[{index}]")),
        }
    }
    out
}

/// The `c` copy text of a node: scalars copy raw (strings unquoted),
/// objects/arrays copy as compact JSON.
pub fn copy_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}

/// The initial leaf-editor content of a scalar: strings raw, anything else as
/// compact JSON.
pub fn leaf_text(value: &Value) -> String {
    copy_text(value)
}

fn walk<'a>(node: &'a Value, path: &[Segment]) -> Option<&'a Value> {
    let mut node = node;
    for segment in path {
        node = match (node, segment) {
            (Value::Object(map), Segment::Key(key)) => map.get(key)?,
            (Value::Array(items), Segment::Index(index)) => items.get(*index)?,
            _ => return None,
        };
    }
    Some(node)
}

fn preview(value: &Value) -> String {
    match value {
        Value::Object(map) => format!("{{{}}}", map.len()),
        Value::Array(items) => format!("[{}]", items.len()),
        Value::String(text) => truncate(text, 120),
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}

fn scalar_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn search_node(node: &Value, needle: &str, path: &mut Vec<Segment>, out: &mut Vec<Vec<Segment>>) {
    match node {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for key in keys {
                path.push(Segment::Key(key.clone()));
                let value = &map[key];
                let branch = value.is_object() || value.is_array();
                if key.to_lowercase().contains(needle) || (!branch && scalar_text(value).to_lowercase().contains(needle)) {
                    out.push(path.clone());
                }
                if branch {
                    search_node(value, needle, path, out);
                }
                path.pop();
            }
        }
        Value::Array(items) => {
            for (index, value) in items.iter().enumerate() {
                path.push(Segment::Index(index));
                if value.is_object() || value.is_array() {
                    search_node(value, needle, path, out);
                } else if scalar_text(value).to_lowercase().contains(needle) {
                    out.push(path.clone());
                }
                path.pop();
            }
        }
        _ => {}
    }
}

fn build_fragment(stored: &Value, display: &Value, path: &[Segment], edit: &Edit) -> Value {
    // Embedding boundary: structured on screen, a JSON string on disk.
    // Apply the edit to the display form and re-serialize it as a string.
    if stored.is_string() && (display.is_object() || display.is_array()) {
        let mut new_display = display.clone();
        apply_display_edit(&mut new_display, path, edit);
        return Value::String(serde_json::to_string(&new_display).unwrap_or_default());
    }
    match (path.split_first(), edit) {
        (None, Edit::Set(value)) => value.clone(),
        (None, Edit::Delete) => Value::Null,
        (None, Edit::Append(value)) => {
            let mut items = display.as_array().cloned().unwrap_or_default();
            items.push(value.clone());
            Value::Array(items)
        }
        (Some((Segment::Key(key), rest)), _) => {
            if rest.is_empty() && matches!(edit, Edit::Delete) {
                let mut map = Map::new();
                map.insert(key.clone(), Value::Null);
                return Value::Object(map);
            }
            let child = build_fragment(stored.get(key).unwrap_or(&Value::Null), display.get(key).unwrap_or(&Value::Null), rest, edit);
            let mut map = Map::new();
            map.insert(key.clone(), child);
            Value::Object(map)
        }
        (Some((Segment::Index(index), rest)), _) => {
            let stored_items = stored.as_array();
            let display_items = display.as_array().cloned().unwrap_or_default();
            let mut out = Vec::with_capacity(display_items.len());
            for (i, item) in display_items.iter().enumerate() {
                let keep_stored = || stored_items.and_then(|items| items.get(i)).cloned().unwrap_or_else(|| item.clone());
                if i == *index && rest.is_empty() {
                    match edit {
                        Edit::Delete => {}
                        Edit::Set(value) => out.push(value.clone()),
                        Edit::Append(_) => out.push(keep_stored()),
                    }
                } else if i == *index {
                    let stored_item = stored_items.and_then(|items| items.get(i)).unwrap_or(&Value::Null);
                    out.push(build_fragment(stored_item, item, rest, edit));
                } else {
                    out.push(keep_stored());
                }
            }
            Value::Array(out)
        }
    }
}

fn apply_display_edit(node: &mut Value, path: &[Segment], edit: &Edit) {
    let Some((first, rest)) = path.split_first() else {
        match edit {
            Edit::Set(value) => *node = value.clone(),
            Edit::Delete => *node = Value::Null,
            Edit::Append(value) => {
                if let Value::Array(items) = node {
                    items.push(value.clone());
                }
            }
        }
        return;
    };
    match (node, first) {
        (Value::Object(map), Segment::Key(key)) => {
            if rest.is_empty() && matches!(edit, Edit::Delete) {
                map.remove(key);
            } else {
                apply_display_edit(map.entry(key.clone()).or_insert(Value::Null), rest, edit);
            }
        }
        (Value::Array(items), Segment::Index(index)) => {
            if *index >= items.len() {
                return;
            }
            if rest.is_empty() {
                match edit {
                    Edit::Delete => {
                        items.remove(*index);
                    }
                    Edit::Set(value) => items[*index] = value.clone(),
                    Edit::Append(_) => {}
                }
            } else {
                apply_display_edit(&mut items[*index], rest, edit);
            }
        }
        _ => {}
    }
}
