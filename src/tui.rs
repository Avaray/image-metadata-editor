use crate::error::AppError;
use crate::{extract, inject};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use std::collections::BTreeMap;
use std::{fs, io, path::PathBuf};

#[derive(Default)]
struct InputState {
    value: String,
    cursor: usize,
}

impl InputState {
    fn new(value: String) -> Self {
        let cursor = value.chars().count();
        Self { value, cursor }
    }

    fn insert(&mut self, ch: char) {
        let mut chars: Vec<char> = self.value.chars().collect();
        chars.insert(self.cursor, ch);
        self.value = chars.into_iter().collect();
        self.cursor += 1;
    }

    fn insert_str(&mut self, s: &str) {
        let mut chars: Vec<char> = self.value.chars().collect();
        let s_chars: Vec<char> = s.chars().collect();
        for (i, &ch) in s_chars.iter().enumerate() {
            chars.insert(self.cursor + i, ch);
        }
        self.value = chars.into_iter().collect();
        self.cursor += s_chars.len();
    }

    fn remove(&mut self) {
        if self.cursor > 0 {
            let mut chars: Vec<char> = self.value.chars().collect();
            chars.remove(self.cursor - 1);
            self.value = chars.into_iter().collect();
            self.cursor -= 1;
        }
    }

    fn move_cursor_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    fn move_cursor_right(&mut self) {
        let max = self.value.chars().count();
        if self.cursor < max {
            self.cursor += 1;
        }
    }

    fn move_word_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let chars: Vec<char> = self.value.chars().collect();
        let mut i = self.cursor - 1;
        while i > 0 && chars[i].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !chars[i].is_whitespace() {
            i -= 1;
        }
        if i > 0 || chars[i].is_whitespace() {
            i += 1;
        }
        self.cursor = i;
    }

    fn move_word_right(&mut self) {
        let chars: Vec<char> = self.value.chars().collect();
        let len = chars.len();
        if self.cursor >= len {
            return;
        }
        let mut i = self.cursor;
        while i < len && !chars[i].is_whitespace() {
            i += 1;
        }
        while i < len && chars[i].is_whitespace() {
            i += 1;
        }
        self.cursor = i;
    }
}

enum Focus {
    FileList,
    Metadata,
}

enum AppState {
    Normal,
    Searching,
    Editing { tag: String, input: InputState },
    AddingTag { key: InputState, value: InputState, focus_value: bool },
    ConfirmStrip,
    ConfirmDelete { tag: String },
    ConfirmExit,
}

struct App {
    current_dir: PathBuf,
    files: Vec<PathBuf>,
    file_display_items: Vec<String>,
    file_state: ListState,

    current_metadata: Option<BTreeMap<String, String>>,
    meta_state: ListState,
    meta_keys: Vec<String>,
    meta_values: BTreeMap<String, String>,

    // all_meta_keys holds the full unfiltered list; meta_keys is the filtered view
    all_meta_keys: Vec<String>,
    search_query: String,

    pending_edits: BTreeMap<String, String>,
    pending_deletes: std::collections::BTreeSet<String>,
    /// Brief status message shown in the help bar (e.g. "Copied!" / "Deleted")
    status_msg: Option<String>,
    json_path: Vec<String>,

    focus: Focus,
    state: AppState,
    should_quit: bool,
    power_user: bool,
    explorer_mode: bool,
    /// Remember the previously selected directory when entering a subdirectory
    last_selected_dir: Option<PathBuf>,
    /// True when at the virtual root showing all drives (above C:\, D:\, etc.)
    at_virtual_root: bool,
}

impl App {
    fn new(start_path: PathBuf, power_user: bool, explorer_mode: bool) -> Result<Self, AppError> {
        let (current_dir, initial_file) = if start_path.is_dir() { (start_path.clone(), None) } else { (start_path.parent().unwrap_or_else(|| std::path::Path::new("")).to_path_buf(), Some(start_path.clone())) };

        let mut app = App {
            current_dir,
            files: Vec::new(),
            file_display_items: Vec::new(),
            file_state: ListState::default(),
            current_metadata: None,
            meta_state: ListState::default(),
            meta_keys: Vec::new(),
            meta_values: BTreeMap::new(),
            all_meta_keys: Vec::new(),
            search_query: String::new(),
            pending_edits: BTreeMap::new(),
            pending_deletes: std::collections::BTreeSet::new(),
            status_msg: None,
            json_path: Vec::new(),
            focus: Focus::FileList,
            state: AppState::Normal,
            should_quit: false,
            power_user,
            explorer_mode,
            last_selected_dir: None,
            at_virtual_root: false,
        };

        app.load_files()?;

        if let Some(f) = initial_file {
            if let Some(idx) = app.files.iter().position(|p| p == &f) {
                app.file_state.select(Some(idx));
                app.load_selected_metadata();
            }
        } else if !app.files.is_empty() {
            app.file_state.select(Some(0));
            app.load_selected_metadata();
        }

        Ok(app)
    }

    fn load_files(&mut self) -> Result<(), AppError> {
        self.files.clear();

        if self.explorer_mode {
            if self.at_virtual_root {
                // At virtual root - show all drives/roots as folders.
                // current_dir is a dummy empty path here; skip read_dir below.
                for root in get_roots() {
                    self.files.push(root);
                }
                // Sort drives and return early — no read_dir needed.
                self.files.sort_by(|a, b| a.cmp(b));
                self.update_file_display_items();
                return Ok(());
            } else if self.is_at_drive_root() {
                // At a drive root (e.g., C:\) - show ".." to go up to virtual root.
                self.files.push(self.current_dir.join(".."));
            } else if let Some(parent) = self.current_dir.parent() {
                if !parent.as_os_str().is_empty() {
                    self.files.push(self.current_dir.join(".."));
                }
            }
        }

        if let Ok(entries) = fs::read_dir(&self.current_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Some(ext) = path.extension().and_then(|e| e.to_str()).map(|s| s.to_lowercase()) {
                        if matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp" | "heic" | "heif" | "avif" | "tiff" | "tif" | "cr3" | "raf" | "iiq" | "mp4" | "mov" | "3gp" | "mkv" | "webm") {
                            self.files.push(path);
                        }
                    }
                } else if path.is_dir() && self.explorer_mode {
                    self.files.push(path);
                }
            }
        }

        self.files.sort_by(|a, b| {
            let a_is_dotdot = App::path_is_dotdot(a);
            let b_is_dotdot = App::path_is_dotdot(b);
            let a_is_root = is_path_drive_root(a);
            let b_is_root = is_path_drive_root(b);

            // Root markers (drives) come first
            if a_is_root && !b_is_root {
                std::cmp::Ordering::Less
            } else if !a_is_root && b_is_root {
                std::cmp::Ordering::Greater
            } else if a_is_dotdot && !b_is_dotdot {
                std::cmp::Ordering::Less
            } else if !a_is_dotdot && b_is_dotdot {
                std::cmp::Ordering::Greater
            } else {
                let a_is_dir = a.is_dir() || a_is_dotdot || a_is_root;
                let b_is_dir = b.is_dir() || b_is_dotdot || b_is_root;
                if a_is_dir && !b_is_dir {
                    std::cmp::Ordering::Less
                } else if !a_is_dir && b_is_dir {
                    std::cmp::Ordering::Greater
                } else {
                    a.cmp(b)
                }
            }
        });

        self.update_file_display_items();

        Ok(())
    }

    fn update_file_display_items(&mut self) {
        self.file_display_items.clear();
        for p in &self.files {
            let name = if App::path_is_dotdot(p) {
                "..".to_string()
            } else if self.explorer_mode && is_path_drive_root(p) {
                p.to_string_lossy().into_owned()
            } else {
                p.file_name().unwrap_or_default().to_string_lossy().into_owned()
            };

            let mut prefix = "";
            if self.explorer_mode {
                if name == ".." {
                    prefix = "\u{f060} ";
                } else if p.is_dir() || is_path_drive_root(p) {
                    prefix = "\u{f07b} ";
                } else if let Some(ext) = p.extension().and_then(|e| e.to_str()).map(|s| s.to_lowercase()) {
                    if matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp" | "heic" | "heif" | "avif" | "tiff" | "tif" | "cr3" | "raf" | "iiq") {
                        prefix = "\u{f1c5} ";
                    } else if matches!(ext.as_str(), "mp4" | "mov" | "3gp" | "mkv" | "webm") {
                        prefix = "\u{f03d} ";
                    } else {
                        prefix = "\u{f15b} ";
                    }
                } else {
                    prefix = "\u{f15b} ";
                }
            }
            self.file_display_items.push(format!("{}{}", prefix, name));
        }
    }

    /// Check if current_dir is at a drive root (e.g., C:\ on Windows, / on Unix)
    fn is_at_drive_root(&self) -> bool {
        is_path_drive_root(&self.current_dir)
    }

    /// Returns true if the given path is the virtual `..` entry (go-up sentinel).
    fn path_is_dotdot(path: &PathBuf) -> bool {
        path.ends_with("..")
    }

    /// Navigate one level up (mirrors the `..` action). Works on both platforms.
    fn navigate_up(&mut self) {
        if self.at_virtual_root {
            // Already at the top — nothing to do.
            return;
        }
        if self.is_at_drive_root() {
            // Go up from a drive root to the virtual root (drive list).
            let previous_drive = self.current_dir.clone();

            self.at_virtual_root = true;
            self.current_dir = PathBuf::from(""); // dummy — not used while at_virtual_root
            let _ = self.load_files();

            // Try to restore selection to the drive we just came from.
            let mut selected_idx = if self.files.is_empty() { None } else { Some(0) };
            if let Some(idx) = self.files.iter().position(|p| p == &previous_drive) {
                selected_idx = Some(idx);
            }
            self.file_state.select(selected_idx);
            self.load_selected_metadata();
        } else if let Some(parent) = self.current_dir.parent() {
            let previous_dir_name = self.current_dir.file_name().and_then(|n| n.to_str()).map(|s| s.to_string());
            self.current_dir = parent.to_path_buf();
            let _ = self.load_files();
            // Try to restore selection to the directory we just came from.
            let mut selected_idx = if self.files.is_empty() { None } else { Some(0) };
            if let Some(dir_name) = previous_dir_name {
                if let Some(idx) = self.files.iter().position(|p| p.file_name().and_then(|n| n.to_str()) == Some(&dir_name)) {
                    selected_idx = Some(idx);
                }
            }
            self.file_state.select(selected_idx);
            self.load_selected_metadata();
            self.last_selected_dir = None;
        }
    }

    /// Navigate into a directory or drive. `path` must be a real directory, not `..`.
    fn navigate_into(&mut self, path: PathBuf) {
        let fname = path.file_name().unwrap_or_default().to_os_string();
        if self.at_virtual_root {
            // Entering a drive from the virtual root list.
            self.at_virtual_root = false;
            self.current_dir = path;
            let _ = self.load_files();
            self.file_state.select(if self.files.is_empty() { None } else { Some(0) });
            self.load_selected_metadata();
        } else if path != self.current_dir {
            if !is_path_drive_root(&path) {
                self.last_selected_dir = Some(fname.into());
            }
            self.current_dir = path;
            let _ = self.load_files();
            self.file_state.select(if self.files.is_empty() { None } else { Some(0) });
            self.load_selected_metadata();
        }
    }

    fn load_selected_metadata(&mut self) {
        self.current_metadata = None;
        self.pending_edits.clear();
        self.pending_deletes.clear();
        self.status_msg = None;
        self.json_path.clear();
        self.search_query.clear();
        self.all_meta_keys.clear();

        if let Some(idx) = self.file_state.selected() {
            if let Some(path) = self.files.get(idx) {
                let path_str = path.to_string_lossy();
                let mut parser = nom_exif::MediaParser::new();
                if let Ok(metadata) = extract::extract(&path_str, &mut parser) {
                    let mut flat = BTreeMap::new();
                    for (group, tags) in metadata.iter() {
                        for (tag, val) in tags {
                            flat.insert(format!("{}.{}", group, tag), val.clone());
                        }
                    }
                    self.current_metadata = Some(flat);
                }
            }
        }
        self.reload_meta_view();
    }

    fn reload_meta_view(&mut self) {
        self.meta_keys.clear();
        self.meta_values.clear();

        if self.json_path.is_empty() {
            let mut keys = std::collections::BTreeSet::new();
            if let Some(m) = &self.current_metadata {
                for k in m.keys() {
                    if !self.pending_deletes.contains(k) {
                        keys.insert(k.clone());
                    }
                }
            }
            for k in self.pending_edits.keys() {
                if !self.pending_deletes.contains(k) {
                    keys.insert(k.clone());
                }
            }

            self.all_meta_keys = keys.into_iter().collect();
            for k in &self.all_meta_keys {
                let val = self.pending_edits.get(k).or_else(|| self.current_metadata.as_ref().and_then(|m| m.get(k))).cloned().unwrap_or_default();
                self.meta_values.insert(k.clone(), val);
            }
        } else {
            let root_key = &self.json_path[0];
            let root_val = self.pending_edits.get(root_key).or_else(|| self.current_metadata.as_ref().and_then(|m| m.get(root_key))).cloned().unwrap_or_default();

            let mut valid = false;
            let root_val_sanitized = sanitize_json(&root_val);
            if let Ok(mut parsed) = serde_json::from_str::<serde_json::Value>(&root_val_sanitized) {
                if let serde_json::Value::String(ref s) = parsed {
                    if let Ok(inner) = serde_json::from_str::<serde_json::Value>(s) {
                        parsed = inner;
                    }
                }
                if let Some(curr) = get_json_at_path(&parsed, &self.json_path[1..]) {
                    valid = true;
                    match curr {
                        serde_json::Value::Object(map) => {
                            self.all_meta_keys = map.keys().cloned().collect();
                            for (k, v) in map {
                                let display_val = if v.is_string() { v.as_str().unwrap().to_string() } else { serde_json::to_string(&v).unwrap_or_default() };
                                self.meta_values.insert(k.clone(), display_val);
                            }
                        }
                        serde_json::Value::Array(arr) => {
                            self.all_meta_keys = (0..arr.len()).map(|i| i.to_string()).collect();
                            for (i, v) in arr.iter().enumerate() {
                                let display_val = if v.is_string() { v.as_str().unwrap().to_string() } else { serde_json::to_string(v).unwrap_or_default() };
                                self.meta_values.insert(i.to_string(), display_val);
                            }
                        }
                        _ => {}
                    }
                }
            }

            if !valid {
                self.json_path.clear();
                return self.reload_meta_view();
            }
        }

        self.apply_search_filter();
    }

    fn apply_search_filter(&mut self) {
        let q = self.search_query.to_lowercase();
        if q.is_empty() {
            self.meta_keys = self.all_meta_keys.clone();
        } else {
            self.meta_keys = self
                .all_meta_keys
                .iter()
                .filter(|k| {
                    let val = self.meta_values.get(*k).map(|s| s.as_str()).unwrap_or("");
                    k.to_lowercase().contains(&q) || val.to_lowercase().contains(&q)
                })
                .cloned()
                .collect();
        }

        if self.meta_keys.is_empty() {
            self.meta_state.select(None);
        } else {
            let i = self.meta_state.selected().unwrap_or(0);
            self.meta_state.select(Some(i.min(self.meta_keys.len() - 1)));
        }
    }

    fn apply_nested_edit(&mut self, tag: &str, new_val: String) {
        if self.json_path.is_empty() {
            self.pending_edits.insert(tag.to_string(), new_val);
            return;
        }

        let root_key = &self.json_path[0];
        let root_val = self.pending_edits.get(root_key).or_else(|| self.current_metadata.as_ref().and_then(|m| m.get(root_key))).cloned().unwrap_or_default();

        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&root_val) {
            let is_wrapped_string = parsed.is_string();
            let mut actual_json = if is_wrapped_string { serde_json::from_str(parsed.as_str().unwrap()).unwrap_or(serde_json::Value::Null) } else { parsed.clone() };

            let new_json: serde_json::Value = serde_json::from_str(&new_val).unwrap_or(serde_json::Value::String(new_val));

            if let Some(parent) = get_json_at_path_mut(&mut actual_json, &self.json_path[1..]) {
                match parent {
                    serde_json::Value::Object(map) => {
                        map.insert(tag.to_string(), new_json);
                    }
                    serde_json::Value::Array(arr) => {
                        if let Ok(idx) = tag.parse::<usize>() {
                            if idx < arr.len() {
                                arr[idx] = new_json;
                            }
                        }
                    }
                    _ => {}
                }
            }

            let final_root = if is_wrapped_string { serde_json::Value::String(serde_json::to_string(&actual_json).unwrap_or_default()) } else { actual_json };

            if let Ok(new_root_str) = serde_json::to_string(&final_root) {
                self.pending_edits.insert(root_key.clone(), new_root_str);
            }
        }
    }

    fn next_file(&mut self) {
        if self.files.is_empty() {
            return;
        }
        let i = match self.file_state.selected() {
            Some(i) => {
                if i >= self.files.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.file_state.select(Some(i));
        self.load_selected_metadata();
    }

    fn previous_file(&mut self) {
        if self.files.is_empty() {
            return;
        }
        let i = match self.file_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.files.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.file_state.select(Some(i));
        self.load_selected_metadata();
    }

    fn next_meta(&mut self) {
        if self.meta_keys.is_empty() {
            return;
        }
        let i = match self.meta_state.selected() {
            Some(i) => {
                if i >= self.meta_keys.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.meta_state.select(Some(i));
    }

    fn previous_meta(&mut self) {
        if self.meta_keys.is_empty() {
            return;
        }
        let i = match self.meta_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.meta_keys.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.meta_state.select(Some(i));
    }

    fn has_pending_changes(&self) -> bool {
        !self.pending_edits.is_empty() || !self.pending_deletes.is_empty()
    }

    fn save_pending_edits(&mut self) -> Result<(), String> {
        let res = (|| -> Result<(), String> {
            if let Some(idx) = self.file_state.selected() {
                if let Some(path) = self.files.get(idx) {
                    let path_str = path.to_string_lossy().into_owned();

                    // Apply edits first
                    if !self.pending_edits.is_empty() {
                        let mut stripped_keys = BTreeMap::new();
                        for (k, v) in &self.pending_edits {
                            let short_k = if let Some((_, tag)) = k.split_once('.') { tag } else { k };
                            stripped_keys.insert(short_k.to_string(), v.clone());
                        }
                        inject::inject_metadata(&path_str, None, &stripped_keys)?;
                    }

                    // Apply deletes
                    if !self.pending_deletes.is_empty() {
                        let keys: Vec<String> = self.pending_deletes.iter().cloned().collect();
                        inject::delete_metadata_keys(&path_str, None, &keys)?;
                    }
                }
            }
            Ok(())
        })();

        match res {
            Ok(_) => {
                self.pending_edits.clear();
                self.pending_deletes.clear();
                self.load_selected_metadata();
                self.status_msg = Some("Saved successfully!".to_string());
                Ok(())
            }
            Err(e) => {
                self.status_msg = Some(format!("Save error: {}", e));
                Err(e)
            }
        }
    }

    fn is_read_only(&self) -> bool {
        if let Some(idx) = self.file_state.selected() {
            if let Some(path) = self.files.get(idx) {
                if path.is_dir() || App::path_is_dotdot(path) || is_path_drive_root(&path) {
                    return true;
                }
                if let Some(ext) = path.extension().and_then(|e| e.to_str()).map(|s| s.to_lowercase()) {
                    return !matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp");
                }
                return true;
            }
        }
        true
    }

    /// Returns true if the selected item is a navigable directory, drive root, or the '..' virtual folder.
    fn is_navigable_dir(&self) -> bool {
        if let Some(idx) = self.file_state.selected() {
            if let Some(path) = self.files.get(idx) {
                return path.is_dir() || App::path_is_dotdot(path) || is_path_drive_root(&path);
            }
        }
        false
    }
}

fn get_json_at_path<'a>(val: &'a serde_json::Value, path: &[String]) -> Option<&'a serde_json::Value> {
    let mut curr = val;
    for p in path {
        match curr {
            serde_json::Value::Object(map) => {
                curr = map.get(p)?;
            }
            serde_json::Value::Array(arr) => {
                let idx: usize = p.parse().ok()?;
                curr = arr.get(idx)?;
            }
            _ => return None,
        }
    }
    Some(curr)
}

fn get_json_at_path_mut<'a>(val: &'a mut serde_json::Value, path: &[String]) -> Option<&'a mut serde_json::Value> {
    let mut curr = val;
    for p in path {
        match curr {
            serde_json::Value::Object(map) => {
                curr = map.get_mut(p)?;
            }
            serde_json::Value::Array(arr) => {
                let idx: usize = p.parse().ok()?;
                curr = arr.get_mut(idx)?;
            }
            _ => return None,
        }
    }
    Some(curr)
}

/// Returns a list of root paths (drives on Windows, / on Unix)
fn get_roots() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        // On Windows, check all drive letters A: through Z:
        (b'A'..=b'Z')
            .filter_map(|letter| {
                let drive = format!("{}:\\", letter as char);
                let path = PathBuf::from(&drive);
                if path.exists() { Some(path) } else { None }
            })
            .collect()
    }
    #[cfg(not(windows))]
    {
        // On Unix/Linux, root is /
        vec![PathBuf::from("/")]
    }
}

/// Check if a filename is a root marker (drive letter or /)
fn is_path_drive_root(path: &std::path::Path) -> bool {
    #[cfg(windows)]
    {
        let s = path.to_string_lossy();
        s.len() == 3 && s.chars().nth(0).map_or(false, |c| c.is_ascii_alphabetic()) && s.chars().nth(1) == Some(':') && (s.chars().nth(2) == Some('\\') || s.chars().nth(2) == Some('/'))
    }
    #[cfg(not(windows))]
    {
        path == std::path::Path::new("/")
    }
}

/// Replaces bare `NaN`, `Infinity`, `-Infinity` (invalid JSON but legal in JS/ComfyUI)
/// with `null` so serde_json can parse the resulting string. Only replaces tokens
/// that appear outside of JSON strings (respects `\"` escape sequences).
fn sanitize_json(input: &str) -> std::borrow::Cow<'_, str> {
    if !input.contains("NaN") && !input.contains("Infinity") {
        return std::borrow::Cow::Borrowed(input);
    }
    let chars: Vec<char> = input.chars().collect();
    let n = chars.len();
    let mut result = String::with_capacity(n);
    let mut i = 0;
    let mut in_string = false;

    while i < n {
        let c = chars[i];
        if in_string {
            result.push(c);
            if c == '\\' && i + 1 < n {
                result.push(chars[i + 1]);
                i += 2;
                continue;
            } else if c == '"' {
                in_string = false;
            }
        } else {
            match c {
                '"' => {
                    result.push(c);
                    in_string = true;
                }
                'N' if i + 2 < n && chars[i + 1] == 'a' && chars[i + 2] == 'N' => {
                    result.push_str("null");
                    i += 3;
                    continue;
                }
                'I' if i + 7 < n && &chars[i + 1..=i + 7].iter().collect::<String>() == "nfinity" => {
                    result.push_str("null");
                    i += 8;
                    continue;
                }
                '-' if i + 8 < n && chars[i + 1] == 'I' && &chars[i + 2..=i + 8].iter().collect::<String>() == "nfinity" => {
                    result.push_str("null");
                    i += 9;
                    continue;
                }
                _ => {
                    result.push(c);
                }
            }
        }
        i += 1;
    }
    std::borrow::Cow::Owned(result)
}

fn is_drillable_json(val: &str) -> bool {
    let trimmed = val.trim();

    // Handle string-wrapped JSON (e.g. PngText chunks stored as JSON string)
    let unwrapped_storage;
    let to_check: &str = if trimmed.starts_with('"') && trimmed.ends_with('"') {
        if let Ok(serde_json::Value::String(inner)) = serde_json::from_str::<serde_json::Value>(trimmed) {
            unwrapped_storage = inner;
            unwrapped_storage.trim()
        } else {
            return false;
        }
    } else {
        trimmed
    };

    if !to_check.starts_with('{') && !to_check.starts_with('[') {
        return false;
    }

    let sanitized = sanitize_json(to_check);
    match serde_json::from_str::<serde_json::Value>(&sanitized) {
        Ok(serde_json::Value::Object(map)) => !map.is_empty(),
        Ok(serde_json::Value::Array(arr)) => !arr.is_empty(),
        _ => false,
    }
}

fn render_cursor_spans(chars: &[char], cursor: usize) -> (String, String, String) {
    let mut before = String::new();
    let mut cursor_char = " ".to_string();
    let mut after = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if i < cursor {
            before.push(c);
        } else if i == cursor {
            cursor_char = c.to_string();
        } else {
            after.push(c);
        }
    }
    (before, cursor_char, after)
}

pub fn run(path: &str, power_user: bool, explorer_mode: bool) -> Result<(), AppError> {
    enable_raw_mode().map_err(|e| AppError::Runtime(e.to_string()))?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture).map_err(|e| AppError::Runtime(e.to_string()))?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(|e| AppError::Runtime(e.to_string()))?;

    let app = App::new(PathBuf::from(path), power_user, explorer_mode)?;
    let res = run_app(&mut terminal, app);

    disable_raw_mode().map_err(|e| AppError::Runtime(e.to_string()))?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture).map_err(|e| AppError::Runtime(e.to_string()))?;
    terminal.show_cursor().map_err(|e| AppError::Runtime(e.to_string()))?;

    res
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, mut app: App) -> Result<(), AppError> {
    loop {
        terminal.draw(|f| ui(f, &mut app)).map_err(|e| AppError::Runtime(e.to_string()))?;

        if app.should_quit {
            return Ok(());
        }

        if let Event::Key(key) = event::read().map_err(|e| AppError::Runtime(e.to_string()))? {
            if key.kind == event::KeyEventKind::Press {
                // Dismiss any transient status message on the next keypress
                app.status_msg = None;
                match &mut app.state {
                    AppState::Normal => {
                        match key.code {
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                app.should_quit = true;
                            }
                            KeyCode::Backspace | KeyCode::Esc => {
                                if matches!(app.focus, Focus::Metadata) {
                                    if !app.json_path.is_empty() {
                                        app.json_path.pop();
                                        app.reload_meta_view();
                                    } else if !app.search_query.is_empty() {
                                        // Clear active search filter
                                        app.search_query.clear();
                                        app.apply_search_filter();
                                    }
                                }
                            }
                            KeyCode::Char('q') => {
                                if app.has_pending_changes() {
                                    if app.power_user {
                                        let _ = app.save_pending_edits();
                                        app.should_quit = true;
                                    } else {
                                        app.state = AppState::ConfirmExit;
                                    }
                                } else {
                                    app.should_quit = true;
                                }
                            }
                            KeyCode::Char('r') => {
                                let _ = app.load_files();
                                app.load_selected_metadata();
                            }
                            KeyCode::Tab => {
                                app.focus = match app.focus {
                                    Focus::FileList => Focus::Metadata,
                                    Focus::Metadata => Focus::FileList,
                                };
                            }
                            KeyCode::Left => match app.focus {
                                Focus::FileList => {
                                    if app.explorer_mode {
                                        // Left always navigates up one level in explorer mode.
                                        app.navigate_up();
                                    }
                                }
                                Focus::Metadata => {
                                    if !app.json_path.is_empty() {
                                        app.json_path.pop();
                                        app.reload_meta_view();
                                    } else {
                                        app.focus = Focus::FileList;
                                    }
                                }
                            },
                            KeyCode::Right => match app.focus {
                                Focus::FileList => {
                                    if app.explorer_mode {
                                        if let Some(idx) = app.file_state.selected() {
                                            if let Some(path) = app.files.get(idx).cloned() {
                                                if App::path_is_dotdot(&path) {
                                                    // ".." entry — Right does nothing; use Left or Enter to go up.
                                                } else if path.is_dir() || is_path_drive_root(&path) {
                                                    app.navigate_into(path);
                                                } else {
                                                    // Regular file — move focus to metadata panel.
                                                    app.focus = Focus::Metadata;
                                                }
                                            }
                                        }
                                    } else {
                                        app.focus = Focus::Metadata;
                                    }
                                }
                                Focus::Metadata => {
                                    if let Some(idx) = app.meta_state.selected() {
                                        if let Some(tag) = app.meta_keys.get(idx).cloned() {
                                            let val = app.meta_values.get(&tag).cloned().unwrap_or_default();
                                            if is_drillable_json(&val) {
                                                app.json_path.push(tag);
                                                app.reload_meta_view();
                                            }
                                        }
                                    }
                                }
                            },
                            KeyCode::Enter => match app.focus {
                                Focus::FileList => {
                                    if app.explorer_mode {
                                        if let Some(idx) = app.file_state.selected() {
                                            if let Some(path) = app.files.get(idx).cloned() {
                                                if App::path_is_dotdot(&path) {
                                                    // ".." sentinel — go up.
                                                    app.navigate_up();
                                                    continue;
                                                } else if path.is_dir() || is_path_drive_root(&path) {
                                                    app.navigate_into(path);
                                                    continue;
                                                }
                                            }
                                        }
                                    }
                                    app.focus = Focus::Metadata;
                                }
                                Focus::Metadata => {
                                    if let Some(idx) = app.meta_state.selected() {
                                        if let Some(tag) = app.meta_keys.get(idx).cloned() {
                                            let val = app.meta_values.get(&tag).cloned().unwrap_or_default();
                                            if is_drillable_json(&val) {
                                                app.json_path.push(tag);
                                                app.reload_meta_view();
                                            } else if app.is_read_only() {
                                                app.status_msg = Some("Cannot edit read-only file format".to_string());
                                            } else {
                                                app.state = AppState::Editing { tag, input: InputState::new(val) };
                                            }
                                        }
                                    } else if app.is_read_only() {
                                        app.status_msg = Some("Cannot edit read-only file format".to_string());
                                    } else {
                                        app.state = AppState::AddingTag { key: InputState::default(), value: InputState::default(), focus_value: false };
                                    }
                                }
                            },
                            KeyCode::Down => match app.focus {
                                Focus::FileList => app.next_file(),
                                Focus::Metadata => app.next_meta(),
                            },
                            KeyCode::Up => match app.focus {
                                Focus::FileList => app.previous_file(),
                                Focus::Metadata => app.previous_meta(),
                            },
                            KeyCode::Home => match app.focus {
                                Focus::FileList => {
                                    app.file_state.select(Some(0));
                                    app.load_selected_metadata();
                                }
                                Focus::Metadata => {
                                    if !app.meta_keys.is_empty() {
                                        app.meta_state.select(Some(0));
                                    }
                                }
                            },
                            KeyCode::End => match app.focus {
                                Focus::FileList => {
                                    let len = app.files.len();
                                    if len > 0 {
                                        app.file_state.select(Some(len - 1));
                                        app.load_selected_metadata();
                                    }
                                }
                                Focus::Metadata => {
                                    let len = app.meta_keys.len();
                                    if len > 0 {
                                        app.meta_state.select(Some(len - 1));
                                    }
                                }
                            },
                            KeyCode::Char('s') => {
                                if key.modifiers.contains(KeyModifiers::CONTROL) {
                                    let _ = app.save_pending_edits();
                                } else {
                                    if app.is_read_only() {
                                        app.status_msg = Some("Cannot edit read-only file format".to_string());
                                    } else if app.power_user {
                                        if let Some(idx) = app.file_state.selected() {
                                            if let Some(path) = app.files.get(idx) {
                                                let _ = crate::strip::strip_metadata(&path.to_string_lossy(), None);
                                                app.load_selected_metadata();
                                            }
                                        }
                                    } else {
                                        app.state = AppState::ConfirmStrip;
                                    }
                                }
                            }
                            KeyCode::Char('e') => {
                                if matches!(app.focus, Focus::Metadata) {
                                    if app.is_read_only() {
                                        app.status_msg = Some("Cannot edit read-only file format".to_string());
                                    } else if let Some(idx) = app.meta_state.selected() {
                                        if let Some(tag) = app.meta_keys.get(idx).cloned() {
                                            let val = app.meta_values.get(&tag).cloned().unwrap_or_default();
                                            app.state = AppState::Editing { tag, input: InputState::new(val) };
                                        }
                                    } else {
                                        // No items visible (empty metadata or all filtered out):
                                        // open the "add new tag" dialog
                                        app.state = AppState::AddingTag { key: InputState::default(), value: InputState::default(), focus_value: false };
                                    }
                                }
                            }
                            KeyCode::Char('c') => {
                                if matches!(app.focus, Focus::Metadata) {
                                    if let Some(idx) = app.meta_state.selected() {
                                        if let Some(tag) = app.meta_keys.get(idx) {
                                            let val = app.meta_values.get(tag).cloned().unwrap_or_default();
                                            match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(val)) {
                                                Ok(_) => app.status_msg = Some("Copied to clipboard!".to_string()),
                                                Err(_) => app.status_msg = Some("Clipboard unavailable".to_string()),
                                            }
                                        }
                                    }
                                }
                            }
                            KeyCode::Char('d') => {
                                if matches!(app.focus, Focus::Metadata) {
                                    if app.is_read_only() {
                                        app.status_msg = Some("Cannot edit read-only file format".to_string());
                                    } else if let Some(idx) = app.meta_state.selected() {
                                        if let Some(tag) = app.meta_keys.get(idx).cloned() {
                                            if app.power_user {
                                                app.pending_deletes.insert(tag);
                                                app.reload_meta_view();
                                            } else {
                                                app.state = AppState::ConfirmDelete { tag };
                                            }
                                        }
                                    }
                                }
                            }
                            KeyCode::Char('/') | KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) || key.code == KeyCode::Char('/') && !key.modifiers.contains(KeyModifiers::CONTROL) => {
                                if matches!(app.focus, Focus::Metadata) {
                                    app.state = AppState::Searching;
                                }
                            }
                            _ => {}
                        }
                    }
                    AppState::Searching => {
                        match key.code {
                            KeyCode::Esc => {
                                // Clear filter and exit search mode
                                app.search_query.clear();
                                app.apply_search_filter();
                                app.state = AppState::Normal;
                            }
                            KeyCode::Enter => {
                                // Confirm filter, return to normal navigation
                                app.state = AppState::Normal;
                            }
                            KeyCode::Backspace => {
                                app.search_query.pop();
                                app.apply_search_filter();
                            }
                            KeyCode::Down => {
                                app.next_meta();
                            }
                            KeyCode::Up => {
                                app.previous_meta();
                            }
                            KeyCode::Char(c) => {
                                app.search_query.push(c);
                                app.apply_search_filter();
                            }
                            _ => {}
                        }
                    }
                    AppState::Editing { tag, input } => match key.code {
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            app.state = AppState::Normal;
                        }
                        KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if let Ok(mut cb) = arboard::Clipboard::new() {
                                if let Ok(text) = cb.get_text() {
                                    input.insert_str(&text);
                                }
                            }
                        }
                        KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                            input.insert('\n');
                        }
                        KeyCode::Enter => {
                            let val = input.value.clone();
                            let tag_clone = tag.clone();
                            app.apply_nested_edit(&tag_clone, val);
                            app.reload_meta_view();
                            app.state = AppState::Normal;
                        }
                        KeyCode::Esc => {
                            app.state = AppState::Normal;
                        }
                        KeyCode::Backspace => {
                            input.remove();
                        }
                        KeyCode::Left => {
                            if key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
                                input.move_word_left();
                            } else {
                                input.move_cursor_left();
                            }
                        }
                        KeyCode::Right => {
                            if key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
                                input.move_word_right();
                            } else {
                                input.move_cursor_right();
                            }
                        }
                        KeyCode::Char(c) => {
                            input.insert(c);
                        }
                        _ => {}
                    },
                    AppState::AddingTag { key: key_inp, value: val_inp, focus_value } => match key.code {
                        KeyCode::Esc => {
                            app.state = AppState::Normal;
                        }
                        KeyCode::Tab => {
                            *focus_value = !*focus_value;
                        }
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            app.state = AppState::Normal;
                        }
                        KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if let Ok(mut cb) = arboard::Clipboard::new() {
                                if let Ok(text) = cb.get_text() {
                                    if *focus_value {
                                        val_inp.insert_str(&text);
                                    } else {
                                        // Usually tag keys don't have newlines, but we can just insert anyway
                                        key_inp.insert_str(&text);
                                    }
                                }
                            }
                        }
                        KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                            if *focus_value {
                                val_inp.insert('\n');
                            }
                        }
                        KeyCode::Enter => {
                            if !*focus_value {
                                *focus_value = true;
                            } else {
                                let k = key_inp.value.trim().to_string();
                                let v = val_inp.value.clone();
                                if !k.is_empty() {
                                    let full_key = if k.contains('.') { k.clone() } else { format!("Custom.{}", k) };
                                    app.pending_edits.insert(full_key, v);
                                    app.reload_meta_view();
                                }
                                app.state = AppState::Normal;
                            }
                        }
                        KeyCode::Backspace => {
                            if *focus_value {
                                val_inp.remove();
                            } else {
                                key_inp.remove();
                            }
                        }
                        KeyCode::Left => {
                            if *focus_value {
                                val_inp.move_cursor_left();
                            } else {
                                key_inp.move_cursor_left();
                            }
                        }
                        KeyCode::Right => {
                            if *focus_value {
                                val_inp.move_cursor_right();
                            } else {
                                key_inp.move_cursor_right();
                            }
                        }
                        KeyCode::Char(c) => {
                            if *focus_value {
                                val_inp.insert(c);
                            } else {
                                key_inp.insert(c);
                            }
                        }
                        _ => {}
                    },
                    AppState::ConfirmStrip => match key.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            if let Some(idx) = app.file_state.selected() {
                                if let Some(path) = app.files.get(idx) {
                                    let _ = crate::strip::strip_metadata(&path.to_string_lossy(), None);
                                    app.load_selected_metadata();
                                }
                            }
                            app.state = AppState::Normal;
                        }
                        KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                            app.state = AppState::Normal;
                        }
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            app.should_quit = true;
                        }
                        _ => {}
                    },
                    AppState::ConfirmDelete { tag } => match key.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            let tag_owned = tag.clone();
                            app.pending_deletes.insert(tag_owned);
                            app.reload_meta_view();
                            app.state = AppState::Normal;
                        }
                        KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                            app.state = AppState::Normal;
                        }
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            app.should_quit = true;
                        }
                        _ => {}
                    },
                    AppState::ConfirmExit => match key.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            if let Err(_) = app.save_pending_edits() {
                                app.state = AppState::Normal;
                            } else {
                                app.should_quit = true;
                            }
                        }
                        KeyCode::Char('n') | KeyCode::Char('N') => {
                            app.should_quit = true;
                        }
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            app.should_quit = true;
                        }
                        KeyCode::Char('c') | KeyCode::Char('C') | KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                            app.state = AppState::Normal;
                        }
                        _ => {}
                    },
                }
            }
        }
    }
}

fn ui(f: &mut Frame, app: &mut App) {
    // In explorer mode, reserve space for the path bar at the top
    let top_bar_height = if app.explorer_mode { 3 } else { 0 };
    let vertical_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(top_bar_height), // Top path bar (only in explorer mode)
            Constraint::Min(0),                 // Main content
            Constraint::Length(3),              // Bottom help bar
        ])
        .split(f.area());

    let main_area = vertical_chunks[1];
    let bottom_area = vertical_chunks[2];

    // ── Top: Path Bar (Explorer Mode) ──
    if app.explorer_mode {
        let path_display = if app.at_virtual_root {
            #[cfg(windows)]
            {
                "This PC".to_string()
            }
            #[cfg(not(windows))]
            {
                "Computer".to_string()
            }
        } else {
            app.current_dir.to_string_lossy().into_owned()
        };
        let path_bar = Paragraph::new(path_display).block(Block::default().borders(Borders::ALL).title(" Path ")).style(Style::default().fg(Color::Cyan));
        f.render_widget(path_bar, vertical_chunks[0]);
    }

    let top_chunks = Layout::default().direction(Direction::Horizontal).constraints([Constraint::Percentage(30), Constraint::Percentage(70)].as_ref()).split(main_area);

    // ── Left: File List ──
    let files_iter = app.file_display_items.iter().map(|name| ListItem::new(name.as_str()));

    let mut file_block = Block::default().borders(Borders::ALL).title(" Files ");
    if matches!(app.focus, Focus::FileList) {
        file_block = file_block.style(Style::default().fg(Color::Yellow));
    }

    let file_list = List::new(files_iter).block(file_block).highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    f.render_stateful_widget(file_list, top_chunks[0], &mut app.file_state);

    // ── Right: Metadata ──
    let meta_title = {
        let base = if app.json_path.is_empty() { " Metadata".to_string() } else { format!(" Metadata > {}", app.json_path.join(" > ")) };

        let mut spans = vec![Span::raw(base)];

        if app.is_read_only() && !app.is_navigable_dir() {
            spans.push(Span::styled(" [READ ONLY]", Style::default().add_modifier(Modifier::DIM)));
        }

        if !app.search_query.is_empty() {
            let count = app.meta_keys.len();
            spans.push(Span::raw(format!(" / {}█ ({}) ", app.search_query, count)));
        } else if matches!(app.state, AppState::Searching) {
            spans.push(Span::raw(" / █ "));
        }

        if !app.pending_edits.is_empty() {
            spans.push(Span::raw(" (UNSAVED EDITS) "));
        } else {
            spans.push(Span::raw(" "));
        }

        ratatui::text::Line::from(spans)
    };

    let mut meta_block = Block::default().borders(Borders::ALL).title(meta_title);

    if matches!(app.focus, Focus::Metadata) {
        meta_block = meta_block.style(Style::default().fg(Color::Yellow));
    } else if !app.pending_edits.is_empty() {
        meta_block = meta_block.style(Style::default().fg(Color::Red));
    }

    let mut meta_items = Vec::new();
    if app.meta_keys.is_empty() {
        let msg = if !app.search_query.is_empty() {
            "No matching metadata entries.".to_string()
        } else if app.is_navigable_dir() {
            "".to_string() // Don't show confusing error for directories
        } else {
            "No metadata or invalid file.".to_string()
        };
        meta_items.push(ListItem::new(msg));
    } else {
        for key in &app.meta_keys {
            let val = app.meta_values.get(key).cloned().unwrap_or_default();
            let is_edited = app.pending_edits.contains_key(if app.json_path.is_empty() { key } else { &app.json_path[0] });
            let color = if is_edited { Color::Green } else { Color::Reset };

            let is_json = is_drillable_json(&val);
            let display_key = if is_json { format!("{} [+] ", key) } else { format!("{}: ", key) };

            let line = Line::from(vec![Span::styled(display_key, Style::default().fg(Color::Cyan)), Span::styled(val, Style::default().fg(color))]);
            meta_items.push(ListItem::new(line));
        }
    }

    let meta_list = List::new(meta_items).block(meta_block).highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    f.render_stateful_widget(meta_list, top_chunks[1], &mut app.meta_state);

    // ── Bottom: Help & Instructions ──
    let help_text = match &app.state {
        AppState::Normal => {
            if let Some(msg) = &app.status_msg {
                format!(" {msg} ")
            } else {
                let back = if !app.json_path.is_empty() { " | [←/Backspace] Back Up" } else { "" };
                let search_hint = if !app.search_query.is_empty() { " | [Esc] Clear filter" } else { "" };
                let save_hint = if app.has_pending_changes() { " | [Ctrl+S] Save" } else { "" };
                let del_hint = if !app.pending_deletes.is_empty() { format!(" | {} pending delete(s)", app.pending_deletes.len()) } else { String::new() };
                format!(" [Tab] Focus | [←/→/↑/↓] Navigate | [e/Enter] Edit | [c] Copy | [d] Delete | [/] Search{back}{search_hint}{del_hint}{save_hint} | [s] Strip | [r] Refresh | [q] Quit ")
            }
        }
        AppState::Searching => " [↑/↓] Navigate results | [Enter] Confirm filter | [Esc] Clear & exit search ".to_string(),
        AppState::Editing { .. } => " [Enter] Save | [Shift+Enter] Newline | [Ctrl+V] Paste | [Esc/Ctrl+C] Cancel | [Ctrl+←/→] Jump ".to_string(),
        AppState::AddingTag { focus_value, .. } => {
            if *focus_value {
                " [Enter] Save | [Shift+Enter] Newline | [Ctrl+V] Paste | [Tab] Back to Key | [Esc/Ctrl+C] Cancel ".to_string()
            } else {
                " [Enter/Tab] Move to Value | [Ctrl+V] Paste | [Esc/Ctrl+C] Cancel ".to_string()
            }
        }
        AppState::ConfirmStrip => " Strip all metadata? [y] Yes  [n/Esc] No ".to_string(),
        AppState::ConfirmDelete { tag } => format!(" Delete '{tag}'? [y] Yes  [n/Esc] No "),
        AppState::ConfirmExit => " You have unsaved changes! Save before exit? ".to_string(),
    };

    let version_text = if app.power_user { format!(" 🧨 🧬 ime v{} ", env!("CARGO_PKG_VERSION")) } else { format!(" 🧬 ime v{} ", env!("CARGO_PKG_VERSION")) };
    // Use display width (each emoji = 2 terminal columns) for correct layout sizing
    let version_width = version_text.chars().fold(0u16, |acc, c| acc + if (c as u32) > 0x7F { 2 } else { 1 }) + 2;

    let bottom_layout = Layout::default().direction(Direction::Horizontal).constraints([Constraint::Min(0), Constraint::Length(version_width)].as_ref()).split(bottom_area);

    let p = Paragraph::new(help_text).block(Block::default().borders(Borders::ALL)).style(match app.state {
        AppState::ConfirmExit | AppState::ConfirmStrip => Style::default().fg(Color::Red),
        _ => Style::default(),
    });
    f.render_widget(p, bottom_layout[0]);

    let version_p = Paragraph::new(Line::from(Span::raw(version_text))).block(Block::default().borders(Borders::ALL)).style(Style::default()).alignment(Alignment::Right);
    f.render_widget(version_p, bottom_layout[1]);

    // ── Floating Dialogs ──
    match &app.state {
        AppState::Editing { tag, input } => {
            let area = centered_rect(80, 60, f.area());
            f.render_widget(Clear, area);

            let block = Block::default().title(format!(" Edit: {} ", tag)).borders(Borders::ALL).style(Style::default().fg(Color::Green));

            let chars: Vec<char> = input.value.chars().collect();
            let (before, cursor_char, after) = render_cursor_spans(&chars, input.cursor);

            let text = Line::from(vec![Span::raw(before), Span::styled(cursor_char, Style::default().bg(Color::White).fg(Color::Black)), Span::raw(after)]);

            let p = Paragraph::new(text).block(block).wrap(Wrap { trim: false });

            f.render_widget(p, area);
        }
        AppState::AddingTag { key, value, focus_value } => {
            let area = centered_rect(60, 30, f.area());
            f.render_widget(Clear, area);

            let popup_chunks = Layout::default().direction(Direction::Vertical).constraints([Constraint::Length(3), Constraint::Length(3), Constraint::Min(0)].as_ref()).split(area.inner(ratatui::layout::Margin { horizontal: 1, vertical: 1 }));

            let outer_block = Block::default().title(" Add New Tag ").borders(Borders::ALL).style(Style::default().fg(Color::Cyan));
            f.render_widget(outer_block, area);

            // Key field
            let key_style = if !*focus_value { Style::default().fg(Color::Yellow) } else { Style::default() };
            let key_block = Block::default().title(" Tag Name ").borders(Borders::ALL).style(key_style);
            let key_chars: Vec<char> = key.value.chars().collect();
            let (kb, kc, ka) = render_cursor_spans(&key_chars, key.cursor);
            let key_p = Paragraph::new(Line::from(vec![Span::raw(kb), Span::styled(kc, Style::default().bg(Color::White).fg(Color::Black)), Span::raw(ka)])).block(key_block);
            f.render_widget(key_p, popup_chunks[0]);

            // Value field
            let val_style = if *focus_value { Style::default().fg(Color::Yellow) } else { Style::default() };
            let val_block = Block::default().title(" Value ").borders(Borders::ALL).style(val_style);
            let val_chars: Vec<char> = value.value.chars().collect();
            let (vb, vc, va) = render_cursor_spans(&val_chars, value.cursor);
            let val_p = Paragraph::new(Line::from(vec![Span::raw(vb), Span::styled(vc, Style::default().bg(Color::White).fg(Color::Black)), Span::raw(va)])).block(val_block);
            f.render_widget(val_p, popup_chunks[1]);
        }
        AppState::ConfirmExit => {
            let area = centered_rect(40, 20, f.area());
            f.render_widget(Clear, area);

            let block = Block::default().title(" Unsaved Changes ").borders(Borders::ALL).style(Style::default().fg(Color::Red));

            let p = Paragraph::new("\nSave changes before exiting?\n\n[y] Yes    [n] No    [c] Cancel").block(block).alignment(ratatui::layout::Alignment::Center);

            f.render_widget(p, area);
        }
        _ => {}
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default().direction(Direction::Vertical).constraints([Constraint::Percentage((100 - percent_y) / 2), Constraint::Percentage(percent_y), Constraint::Percentage((100 - percent_y) / 2)].as_ref()).split(r);

    Layout::default().direction(Direction::Horizontal).constraints([Constraint::Percentage((100 - percent_x) / 2), Constraint::Percentage(percent_x), Constraint::Percentage((100 - percent_x) / 2)].as_ref()).split(popup_layout[1])[1]
}
