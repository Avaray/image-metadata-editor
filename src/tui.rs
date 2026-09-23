use crate::error::AppError;
use crate::{extract, inject};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout, Rect, Alignment},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
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
        if self.cursor == 0 { return; }
        let chars: Vec<char> = self.value.chars().collect();
        let mut i = self.cursor - 1;
        while i > 0 && chars[i].is_whitespace() { i -= 1; }
        while i > 0 && !chars[i].is_whitespace() { i -= 1; }
        if i > 0 || chars[i].is_whitespace() { i += 1; }
        self.cursor = i;
    }

    fn move_word_right(&mut self) {
        let chars: Vec<char> = self.value.chars().collect();
        let len = chars.len();
        if self.cursor >= len { return; }
        let mut i = self.cursor;
        while i < len && !chars[i].is_whitespace() { i += 1; }
        while i < len && chars[i].is_whitespace() { i += 1; }
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
    ConfirmExit,
}

struct App {
    current_dir: PathBuf,
    files: Vec<PathBuf>,
    file_state: ListState,
    
    current_metadata: Option<BTreeMap<String, String>>,
    meta_state: ListState,
    meta_keys: Vec<String>,
    meta_values: BTreeMap<String, String>,

    // all_meta_keys holds the full unfiltered list; meta_keys is the filtered view
    all_meta_keys: Vec<String>,
    search_query: String,

    pending_edits: BTreeMap<String, String>,
    json_path: Vec<String>,
    
    focus: Focus,
    state: AppState,
    should_quit: bool,
}

impl App {
    fn new(start_path: PathBuf) -> Result<Self, AppError> {
        let (current_dir, initial_file) = if start_path.is_dir() {
            (start_path.clone(), None)
        } else {
            (
                start_path.parent().unwrap_or_else(|| std::path::Path::new("")).to_path_buf(),
                Some(start_path.clone()),
            )
        };

        let mut app = App {
            current_dir,
            files: Vec::new(),
            file_state: ListState::default(),
            current_metadata: None,
            meta_state: ListState::default(),
            meta_keys: Vec::new(),
            meta_values: BTreeMap::new(),
            all_meta_keys: Vec::new(),
            search_query: String::new(),
            pending_edits: BTreeMap::new(),
            json_path: Vec::new(),
            focus: Focus::FileList,
            state: AppState::Normal,
            should_quit: false,
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
        if let Ok(entries) = fs::read_dir(&self.current_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    self.files.push(path);
                }
            }
        }
        self.files.sort();
        Ok(())
    }

    fn load_selected_metadata(&mut self) {
        self.current_metadata = None;
        self.pending_edits.clear();
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
                for k in m.keys() { keys.insert(k.clone()); }
            }
            for k in self.pending_edits.keys() { keys.insert(k.clone()); }
            
            self.all_meta_keys = keys.into_iter().collect();
            for k in &self.all_meta_keys {
                let val = self.pending_edits.get(k)
                    .or_else(|| self.current_metadata.as_ref().and_then(|m| m.get(k)))
                    .cloned()
                    .unwrap_or_default();
                self.meta_values.insert(k.clone(), val);
            }
        } else {
            let root_key = &self.json_path[0];
            let root_val = self.pending_edits.get(root_key)
                .or_else(|| self.current_metadata.as_ref().and_then(|m| m.get(root_key)))
                .cloned()
                .unwrap_or_default();
            
            let mut valid = false;
            if let Ok(mut parsed) = serde_json::from_str::<serde_json::Value>(&root_val) {
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
                                let display_val = if v.is_string() {
                                    v.as_str().unwrap().to_string()
                                } else {
                                    serde_json::to_string(&v).unwrap_or_default()
                                };
                                self.meta_values.insert(k.clone(), display_val);
                            }
                        }
                        serde_json::Value::Array(arr) => {
                            self.all_meta_keys = (0..arr.len()).map(|i| i.to_string()).collect();
                            for (i, v) in arr.iter().enumerate() {
                                let display_val = if v.is_string() {
                                    v.as_str().unwrap().to_string()
                                } else {
                                    serde_json::to_string(v).unwrap_or_default()
                                };
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
            self.meta_keys = self.all_meta_keys.iter().filter(|k| {
                let val = self.meta_values.get(*k).map(|s| s.as_str()).unwrap_or("");
                k.to_lowercase().contains(&q) || val.to_lowercase().contains(&q)
            }).cloned().collect();
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
        let root_val = self.pending_edits.get(root_key)
            .or_else(|| self.current_metadata.as_ref().and_then(|m| m.get(root_key)))
            .cloned()
            .unwrap_or_default();
            
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&root_val) {
            let is_wrapped_string = parsed.is_string();
            let mut actual_json = if is_wrapped_string {
                serde_json::from_str(parsed.as_str().unwrap()).unwrap_or(serde_json::Value::Null)
            } else {
                parsed.clone()
            };

            let new_json: serde_json::Value = serde_json::from_str(&new_val).unwrap_or(serde_json::Value::String(new_val));
            
            if let Some(parent) = get_json_at_path_mut(&mut actual_json, &self.json_path[1..]) {
                match parent {
                    serde_json::Value::Object(map) => { map.insert(tag.to_string(), new_json); }
                    serde_json::Value::Array(arr) => {
                        if let Ok(idx) = tag.parse::<usize>() {
                            if idx < arr.len() { arr[idx] = new_json; }
                        }
                    }
                    _ => {}
                }
            }
            
            let final_root = if is_wrapped_string {
                serde_json::Value::String(serde_json::to_string(&actual_json).unwrap_or_default())
            } else {
                actual_json
            };

            if let Ok(new_root_str) = serde_json::to_string(&final_root) {
                self.pending_edits.insert(root_key.clone(), new_root_str);
            }
        }
    }

    fn next_file(&mut self) {
        if self.files.is_empty() { return; }
        let i = match self.file_state.selected() {
            Some(i) => if i >= self.files.len() - 1 { 0 } else { i + 1 },
            None => 0,
        };
        self.file_state.select(Some(i));
        self.load_selected_metadata();
    }

    fn previous_file(&mut self) {
        if self.files.is_empty() { return; }
        let i = match self.file_state.selected() {
            Some(i) => if i == 0 { self.files.len() - 1 } else { i - 1 },
            None => 0,
        };
        self.file_state.select(Some(i));
        self.load_selected_metadata();
    }

    fn next_meta(&mut self) {
        if self.meta_keys.is_empty() { return; }
        let i = match self.meta_state.selected() {
            Some(i) => if i >= self.meta_keys.len() - 1 { 0 } else { i + 1 },
            None => 0,
        };
        self.meta_state.select(Some(i));
    }

    fn previous_meta(&mut self) {
        if self.meta_keys.is_empty() { return; }
        let i = match self.meta_state.selected() {
            Some(i) => if i == 0 { self.meta_keys.len() - 1 } else { i - 1 },
            None => 0,
        };
        self.meta_state.select(Some(i));
    }

    fn save_pending_edits(&mut self) -> Result<(), String> {
        if self.pending_edits.is_empty() { return Ok(()); }
        if let Some(idx) = self.file_state.selected() {
            if let Some(path) = self.files.get(idx) {
                let path_str = path.to_string_lossy();
                let mut stripped_keys = BTreeMap::new();
                for (k, v) in &self.pending_edits {
                    let short_k = if let Some((_, tag)) = k.split_once('.') { tag } else { k };
                    stripped_keys.insert(short_k.to_string(), v.clone());
                }
                inject::inject_metadata(&path_str, None, &stripped_keys)?;
            }
        }
        self.pending_edits.clear();
        self.load_selected_metadata();
        Ok(())
    }
}

fn get_json_at_path<'a>(val: &'a serde_json::Value, path: &[String]) -> Option<&'a serde_json::Value> {
    let mut curr = val;
    for p in path {
        match curr {
            serde_json::Value::Object(map) => { curr = map.get(p)?; }
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
            serde_json::Value::Object(map) => { curr = map.get_mut(p)?; }
            serde_json::Value::Array(arr) => {
                let idx: usize = p.parse().ok()?;
                curr = arr.get_mut(idx)?;
            }
            _ => return None,
        }
    }
    Some(curr)
}

fn is_drillable_json(val: &str) -> bool {
    let mut trimmed = val.trim();
    
    let mut parsed_string = None;
    if trimmed.starts_with('"') && trimmed.ends_with('"') {
        if let Ok(serde_json::Value::String(inner)) = serde_json::from_str::<serde_json::Value>(trimmed) {
            parsed_string = Some(inner);
        }
    }
    
    if let Some(ref inner) = parsed_string {
        trimmed = inner.trim();
    }
    
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        return false;
    }
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
        match parsed {
            serde_json::Value::Object(map) => !map.is_empty(),
            serde_json::Value::Array(arr) => !arr.is_empty(),
            _ => false,
        }
    } else {
        false
    }
}

fn render_cursor_spans(chars: &[char], cursor: usize) -> (String, String, String) {
    let mut before = String::new();
    let mut cursor_char = " ".to_string();
    let mut after = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if i < cursor { before.push(c); }
        else if i == cursor { cursor_char = c.to_string(); }
        else { after.push(c); }
    }
    (before, cursor_char, after)
}

pub fn run(path: &str) -> Result<(), AppError> {
    enable_raw_mode().map_err(|e| AppError::Runtime(e.to_string()))?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture).map_err(|e| AppError::Runtime(e.to_string()))?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(|e| AppError::Runtime(e.to_string()))?;

    let app = App::new(PathBuf::from(path))?;
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
                match &mut app.state {
                    AppState::Normal => {
                        match key.code {
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                app.should_quit = true;
                            }
                            KeyCode::Backspace | KeyCode::Esc => {
                                if matches!(app.focus, Focus::Metadata) && !app.json_path.is_empty() {
                                    app.json_path.pop();
                                    app.reload_meta_view();
                                }
                            }
                            KeyCode::Char('q') => {
                                if !app.pending_edits.is_empty() {
                                    app.state = AppState::ConfirmExit;
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
                            KeyCode::Left => {
                                match app.focus {
                                    Focus::FileList => {}
                                    Focus::Metadata => {
                                        if !app.json_path.is_empty() {
                                            app.json_path.pop();
                                            app.reload_meta_view();
                                        } else {
                                            app.focus = Focus::FileList;
                                        }
                                    }
                                }
                            }
                            KeyCode::Right => {
                                match app.focus {
                                    Focus::FileList => {
                                        app.focus = Focus::Metadata;
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
                                }
                            }
                            KeyCode::Enter => {
                                match app.focus {
                                    Focus::FileList => {
                                        app.focus = Focus::Metadata;
                                    }
                                    Focus::Metadata => {
                                        if let Some(idx) = app.meta_state.selected() {
                                            if let Some(tag) = app.meta_keys.get(idx).cloned() {
                                                let val = app.meta_values.get(&tag).cloned().unwrap_or_default();
                                                if is_drillable_json(&val) {
                                                    app.json_path.push(tag);
                                                    app.reload_meta_view();
                                                } else {
                                                    app.state = AppState::Editing {
                                                        tag,
                                                        input: InputState::new(val),
                                                    };
                                                }
                                            }
                                        }
                                    }
                                }
                            }
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
                                    if let Some(idx) = app.file_state.selected() {
                                        if let Some(path) = app.files.get(idx) {
                                            let _ = crate::strip::strip_metadata(&path.to_string_lossy(), None);
                                            app.load_selected_metadata();
                                        }
                                    }
                                }
                            }
                            KeyCode::Char('e') => {
                                if matches!(app.focus, Focus::Metadata) {
                                    if let Some(idx) = app.meta_state.selected() {
                                        if let Some(tag) = app.meta_keys.get(idx).cloned() {
                                            let val = app.meta_values.get(&tag).cloned().unwrap_or_default();
                                            app.state = AppState::Editing {
                                                tag,
                                                input: InputState::new(val),
                                            };
                                        }
                                    } else {
                                        // No items visible (empty metadata or all filtered out):
                                        // open the "add new tag" dialog
                                        app.state = AppState::AddingTag {
                                            key: InputState::default(),
                                            value: InputState::default(),
                                            focus_value: false,
                                        };
                                    }
                                }
                            }
                            KeyCode::Char('/') => {
                                if matches!(app.focus, Focus::Metadata) {
                                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                                        // Ctrl+/ clears the active filter immediately
                                        app.search_query.clear();
                                        app.apply_search_filter();
                                    } else {
                                        app.state = AppState::Searching;
                                    }
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
                            KeyCode::Down => { app.next_meta(); }
                            KeyCode::Up => { app.previous_meta(); }
                            KeyCode::Char(c) => {
                                app.search_query.push(c);
                                app.apply_search_filter();
                            }
                            _ => {}
                        }
                    }
                    AppState::Editing { tag, input } => {
                        match key.code {
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                app.state = AppState::Normal;
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
                        }
                    }
                    AppState::AddingTag { key: key_inp, value: val_inp, focus_value } => {
                        match key.code {
                            KeyCode::Esc => {
                                app.state = AppState::Normal;
                            }
                            KeyCode::Tab => {
                                *focus_value = !*focus_value;
                            }
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                app.state = AppState::Normal;
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
                                if *focus_value { val_inp.remove(); } else { key_inp.remove(); }
                            }
                            KeyCode::Left => {
                                if *focus_value { val_inp.move_cursor_left(); } else { key_inp.move_cursor_left(); }
                            }
                            KeyCode::Right => {
                                if *focus_value { val_inp.move_cursor_right(); } else { key_inp.move_cursor_right(); }
                            }
                            KeyCode::Char(c) => {
                                if *focus_value { val_inp.insert(c); } else { key_inp.insert(c); }
                            }
                            _ => {}
                        }
                    }
                    AppState::ConfirmExit => {
                        match key.code {
                            KeyCode::Char('y') | KeyCode::Char('Y') => {
                                let _ = app.save_pending_edits();
                                app.should_quit = true;
                            }
                            KeyCode::Char('n') | KeyCode::Char('N') => {
                                app.should_quit = true;
                            }
                            KeyCode::Char('c') | KeyCode::Char('C') | KeyCode::Esc => {
                                app.state = AppState::Normal;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
}

fn ui(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)].as_ref())
        .split(f.area());

    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)].as_ref())
        .split(chunks[0]);

    // ── Left: File List ──
    let files: Vec<ListItem> = app
        .files
        .iter()
        .map(|p| {
            let name = p.file_name().unwrap_or_default().to_string_lossy();
            ListItem::new(name.into_owned())
        })
        .collect();

    let mut file_block = Block::default().borders(Borders::ALL).title(" Files ");
    if matches!(app.focus, Focus::FileList) {
        file_block = file_block.style(Style::default().fg(Color::Yellow));
    }

    let file_list = List::new(files)
        .block(file_block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    f.render_stateful_widget(file_list, top_chunks[0], &mut app.file_state);

    // ── Right: Metadata ──
    let meta_title = {
        let base = if app.json_path.is_empty() {
            " Metadata".to_string()
        } else {
            format!(" Metadata > {}", app.json_path.join(" > "))
        };
        let search_part = if !app.search_query.is_empty() {
            let count = app.meta_keys.len();
            format!(" / {}█ ({}) ", app.search_query, count)
        } else if matches!(app.state, AppState::Searching) {
            " / █ ".to_string()
        } else {
            String::new()
        };
        let edit_part = if !app.pending_edits.is_empty() { " (UNSAVED EDITS)" } else { "" };
        format!("{}{}{} ", base, search_part, edit_part)
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

            let line = Line::from(vec![
                Span::styled(display_key, Style::default().fg(Color::Cyan)),
                Span::styled(val, Style::default().fg(color)),
            ]);
            meta_items.push(ListItem::new(line));
        }
    }

    let meta_list = List::new(meta_items)
        .block(meta_block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    f.render_stateful_widget(meta_list, top_chunks[1], &mut app.meta_state);

    // ── Bottom: Help & Instructions ──
    let help_text = match app.state {
        AppState::Normal => {
            let back = if !app.json_path.is_empty() { " | [←/Backspace] Back Up" } else { "" };
            let search_hint = if !app.search_query.is_empty() { " | [Ctrl+/] Clear filter" } else { "" };
            if !app.pending_edits.is_empty() {
                format!(" [Tab] Focus | [←/→/↑/↓] Navigate | [e/Enter] Edit/Open{}{} | [s] Strip | [r] Refresh | [Ctrl+S] Save | [q] Quit ", back, search_hint)
            } else {
                format!(" [Tab] Focus | [←/→/↑/↓] Navigate | [e/Enter] Edit/Open | [/] Search{}{} | [s] Strip | [r] Refresh | [q] Quit ", back, search_hint)
            }
        },
        AppState::Searching => " [↑/↓] Navigate results | [Enter] Confirm filter | [Esc] Clear & exit search ".to_string(),
        AppState::Editing { .. } => " [Enter] Save edit | [Esc/Ctrl+C] Cancel | [Ctrl+←/→] Jump ".to_string(),
        AppState::AddingTag { ref focus_value, .. } => {
            if *focus_value {
                " [Enter] Save tag | [Tab] Back to Key | [Esc/Ctrl+C] Cancel ".to_string()
            } else {
                " [Enter/Tab] Move to Value | [Esc/Ctrl+C] Cancel ".to_string()
            }
        },
        AppState::ConfirmExit => " You have unsaved edits! Save before exit? ".to_string(),
    };

    let version_text = format!(" 🧬 ime v{} ", env!("CARGO_PKG_VERSION"));
    // Use display width (each emoji = 2 terminal columns) for correct layout sizing
    let version_width = version_text.chars().fold(0u16, |acc, c| {
        acc + if (c as u32) > 0x7F { 2 } else { 1 }
    }) + 2;

    let bottom_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(version_width)].as_ref())
        .split(chunks[1]);

    let p = Paragraph::new(help_text)
        .block(Block::default().borders(Borders::ALL))
        .style(match app.state {
            AppState::ConfirmExit => Style::default().fg(Color::Red),
            _ => Style::default(),
        });
    f.render_widget(p, bottom_layout[0]);

    let version_p = Paragraph::new(Line::from(Span::raw(version_text)))
        .block(Block::default().borders(Borders::ALL))
        .style(Style::default())
        .alignment(Alignment::Right);
    f.render_widget(version_p, bottom_layout[1]);

    // ── Floating Dialogs ──
    match &app.state {
        AppState::Editing { tag, input } => {
            let area = centered_rect(80, 60, f.area());
            f.render_widget(Clear, area);
            
            let block = Block::default()
                .title(format!(" Edit: {} ", tag))
                .borders(Borders::ALL)
                .style(Style::default().fg(Color::Green));
            
            let chars: Vec<char> = input.value.chars().collect();
            let (before, cursor_char, after) = render_cursor_spans(&chars, input.cursor);

            let text = Line::from(vec![
                Span::raw(before),
                Span::styled(cursor_char, Style::default().bg(Color::White).fg(Color::Black)),
                Span::raw(after),
            ]);

            let p = Paragraph::new(text)
                .block(block)
                .wrap(Wrap { trim: false });
            
            f.render_widget(p, area);
        }
        AppState::AddingTag { key, value, focus_value } => {
            let area = centered_rect(60, 30, f.area());
            f.render_widget(Clear, area);

            let popup_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(3), Constraint::Length(3), Constraint::Min(0)].as_ref())
                .split(area.inner(ratatui::layout::Margin { horizontal: 1, vertical: 1 }));

            let outer_block = Block::default()
                .title(" Add New Tag ")
                .borders(Borders::ALL)
                .style(Style::default().fg(Color::Cyan));
            f.render_widget(outer_block, area);

            // Key field
            let key_style = if !*focus_value { Style::default().fg(Color::Yellow) } else { Style::default() };
            let key_block = Block::default().title(" Tag Name ").borders(Borders::ALL).style(key_style);
            let key_chars: Vec<char> = key.value.chars().collect();
            let (kb, kc, ka) = render_cursor_spans(&key_chars, key.cursor);
            let key_p = Paragraph::new(Line::from(vec![
                Span::raw(kb), Span::styled(kc, Style::default().bg(Color::White).fg(Color::Black)), Span::raw(ka),
            ])).block(key_block);
            f.render_widget(key_p, popup_chunks[0]);

            // Value field
            let val_style = if *focus_value { Style::default().fg(Color::Yellow) } else { Style::default() };
            let val_block = Block::default().title(" Value ").borders(Borders::ALL).style(val_style);
            let val_chars: Vec<char> = value.value.chars().collect();
            let (vb, vc, va) = render_cursor_spans(&val_chars, value.cursor);
            let val_p = Paragraph::new(Line::from(vec![
                Span::raw(vb), Span::styled(vc, Style::default().bg(Color::White).fg(Color::Black)), Span::raw(va),
            ])).block(val_block);
            f.render_widget(val_p, popup_chunks[1]);
        }
        AppState::ConfirmExit => {
            let area = centered_rect(40, 20, f.area());
            f.render_widget(Clear, area);
            
            let block = Block::default()
                .title(" Unsaved Changes ")
                .borders(Borders::ALL)
                .style(Style::default().fg(Color::Red));
            
            let p = Paragraph::new("\nSave changes before exiting?\n\n[y] Yes    [n] No    [c] Cancel")
                .block(block)
                .alignment(ratatui::layout::Alignment::Center);
            
            f.render_widget(p, area);
        }
        _ => {}
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Percentage((100 - percent_y) / 2),
                Constraint::Percentage(percent_y),
                Constraint::Percentage((100 - percent_y) / 2),
            ]
            .as_ref(),
        )
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints(
            [
                Constraint::Percentage((100 - percent_x) / 2),
                Constraint::Percentage(percent_x),
                Constraint::Percentage((100 - percent_x) / 2),
            ]
            .as_ref(),
        )
        .split(popup_layout[1])[1]
}
