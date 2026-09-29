use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::{execute, terminal};
use notify::Watcher as _;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use serde_json::Value;
use tui_input::Input;
use tui_input::backend::crossterm::EventHandler as _;

use crate::error::Error;
use crate::tui_tree::{Edit, MetaTree, Segment};
use crate::tui_ui;

/// Open the interactive TUI for `initial` (a directory, or a file whose
/// parent is shown with the file selected). Fails cleanly when the path is
/// unusable or there is no terminal to run on.
pub fn run(initial: &Path, power: bool, watch: bool) -> Result<(), Error> {
    let kind = std::fs::metadata(initial).map_err(|err| Error::runtime(format!("cannot open '{}': {err}", initial.display())))?;
    let absolute = absolutize(initial);
    let (dir, select) = if kind.is_dir() {
        (absolute, None)
    } else {
        let name = absolute.file_name().map(|name| name.to_string_lossy().into_owned());
        let parent = absolute.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("/"));
        (parent, name)
    };

    terminal::enable_raw_mode().map_err(|err| Error::runtime(format!("cannot start TUI: {err}")))?;
    let mut stdout = std::io::stdout();
    execute!(stdout, terminal::EnterAlternateScreen).map_err(|err| Error::runtime(format!("cannot start TUI: {err}")))?;
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(std::io::stdout(), terminal::LeaveAlternateScreen);
        default_hook(info);
    }));

    let mut terminal = Terminal::new(CrosstermBackend::new(stdout)).map_err(|err| Error::runtime(format!("cannot start TUI: {err}")))?;
    let mut app = App::new(dir, select, power, watch);
    let result = event_loop(&mut app, &mut terminal);

    let _ = terminal::disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), terminal::LeaveAlternateScreen);
    let _ = terminal.show_cursor();
    result
}

fn absolutize(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join(path)
}

fn event_loop(app: &mut App, terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>) -> Result<(), Error> {
    loop {
        app.pump_workers();
        app.pump_watch();
        app.tick();
        terminal.draw(|frame| tui_ui::render(app, frame)).map_err(|err| Error::runtime(format!("cannot draw TUI: {err}")))?;
        if event::poll(Duration::from_millis(100)).map_err(|err| Error::runtime(format!("cannot read terminal events: {err}")))? {
            match event::read().map_err(|err| Error::runtime(format!("cannot read terminal events: {err}")))? {
                Event::Key(key) => app.on_key(key),
                // The next draw re-queries the size, so resizes need no handling here.
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
        if app.quit {
            return Ok(());
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    Files,
    Meta,
}

#[derive(Clone)]
pub(crate) struct FileEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_parent: bool,
}

pub(crate) enum ConfirmAction {
    DeleteNode(Vec<Segment>),
    WipeFile(PathBuf),
}

pub(crate) enum Overlay {
    Search { input: Input, panel: Focus },
    LeafEdit { input: Input, path: Vec<Segment>, error: Option<String> },
    SubtreeEdit { lines: Vec<Input>, line: usize, path: Vec<Segment>, error: Option<String> },
    NewKey { input: Input, error: Option<String> },
    NewValue { input: Input, key: Option<String>, error: Option<String> },
    Confirm { message: String, action: ConfirmAction, yes: bool },
    About,
}

struct ScanResp {
    dir: PathBuf,
    entries: Vec<FileEntry>,
    error: Option<String>,
}

struct MetaResp {
    path: PathBuf,
    stored: Result<Value, String>,
}

pub(crate) struct WatchState {
    dir: PathBuf,
    _watcher: notify::RecommendedWatcher,
    rx: mpsc::Receiver<Result<notify::Event, notify::Error>>,
}

pub(crate) struct App {
    pub dir: PathBuf,
    pub entries: Vec<FileEntry>,
    pub scan_error: Option<String>,
    pub file_cursor: usize,
    pub file_scroll: usize,
    pub pending_dir: Option<(PathBuf, Instant)>,
    pub dir_states: HashMap<PathBuf, usize>,
    pub initial_select: Option<String>,
    pub focus: Focus,
    pub preview_path: Option<PathBuf>,
    pub tree: Option<MetaTree>,
    pub preview_error: Option<String>,
    pub preview_loading: Option<(PathBuf, Instant)>,
    pub preview_pending: Option<(PathBuf, Instant)>,
    pub meta_scroll: usize,
    pub restore_drill: Option<(PathBuf, Vec<Segment>, usize)>,
    pub overlay: Option<Overlay>,
    pub status: Option<String>,
    pub power: bool,
    pub watch_requested: bool,
    pub watcher: Option<WatchState>,
    pub watch_dirty: Option<Instant>,
    pub quit: bool,
    scan_tx: mpsc::Sender<ScanResp>,
    scan_rx: mpsc::Receiver<ScanResp>,
    meta_tx: mpsc::Sender<MetaResp>,
    meta_rx: mpsc::Receiver<MetaResp>,
}

impl App {
    fn new(dir: PathBuf, select: Option<String>, power: bool, watch: bool) -> Self {
        let (scan_tx, scan_rx) = mpsc::channel();
        let (meta_tx, meta_rx) = mpsc::channel();
        let mut app = Self {
            dir: dir.clone(),
            entries: Vec::new(),
            scan_error: None,
            file_cursor: 0,
            file_scroll: 0,
            pending_dir: None,
            dir_states: HashMap::new(),
            initial_select: select,
            focus: Focus::Files,
            preview_path: None,
            tree: None,
            preview_error: None,
            preview_loading: None,
            preview_pending: None,
            meta_scroll: 0,
            restore_drill: None,
            overlay: None,
            status: None,
            power,
            watch_requested: watch,
            watcher: None,
            watch_dirty: None,
            quit: false,
            scan_tx,
            scan_rx,
            meta_tx,
            meta_rx,
        };
        app.request_scan(dir, true);
        app
    }

    // -- background workers -------------------------------------------------

    fn request_scan(&mut self, dir: PathBuf, navigation: bool) {
        if navigation {
            self.pending_dir = Some((dir.clone(), Instant::now()));
        }
        let tx = self.scan_tx.clone();
        std::thread::spawn(move || {
            let (entries, error) = scan_entries(&dir);
            let _ = tx.send(ScanResp { dir, entries, error });
        });
    }

    fn request_preview(&mut self, path: PathBuf) {
        self.preview_loading = Some((path.clone(), Instant::now()));
        let tx = self.meta_tx.clone();
        std::thread::spawn(move || {
            let stored = load_stored(&path);
            let _ = tx.send(MetaResp { path, stored });
        });
    }

    /// Collect finished worker results, applying only the ones that are still
    /// what the user is looking at; stale results are silently discarded.
    fn pump_workers(&mut self) {
        while let Ok(resp) = self.scan_rx.try_recv() {
            if self.pending_dir.as_ref().is_some_and(|(dir, _)| *dir == resp.dir) {
                self.apply_navigation(resp);
            } else if self.pending_dir.is_none() && resp.dir == self.dir {
                self.apply_refresh(resp);
            }
        }
        while let Ok(resp) = self.meta_rx.try_recv() {
            let wanted = self.selected_file();
            let loading = self.preview_loading.as_ref().is_some_and(|(path, _)| *path == resp.path);
            if loading && wanted.as_ref() == Some(&resp.path) {
                self.preview_loading = None;
                self.preview_path = Some(resp.path.clone());
                match resp.stored {
                    Ok(stored) => {
                        let mut tree = MetaTree::new(stored);
                        if let Some((path, drill, cursor)) = self.restore_drill.take()
                            && path == resp.path
                        {
                            tree.restore(drill, cursor);
                        }
                        self.tree = Some(tree);
                        self.preview_error = None;
                    }
                    Err(message) => {
                        self.tree = None;
                        self.preview_error = Some(message);
                    }
                }
                self.meta_scroll = 0;
            } else if loading {
                self.preview_loading = None;
            }
        }
    }

    fn apply_navigation(&mut self, resp: ScanResp) {
        self.pending_dir = None;
        self.dir = resp.dir.clone();
        self.entries = resp.entries;
        self.scan_error = resp.error;
        self.file_cursor = 0;
        self.file_scroll = 0;
        if let Some(name) = self.initial_select.take() {
            if let Some(index) = self.entries.iter().position(|entry| entry.name == name) {
                self.file_cursor = index;
            }
        } else if let Some(saved) = self.dir_states.get(&resp.dir) {
            self.file_cursor = (*saved).min(self.entries.len().saturating_sub(1));
        }
        self.refresh_watcher();
        self.selection_changed();
    }

    fn apply_refresh(&mut self, resp: ScanResp) {
        let selected = self.entries.get(self.file_cursor).map(|entry| entry.name.clone());
        self.entries = resp.entries;
        self.scan_error = resp.error;
        self.file_cursor = selected.and_then(|name| self.entries.iter().position(|entry| entry.name == name)).unwrap_or(0).min(self.entries.len().saturating_sub(1));
        self.selection_changed();
    }

    // -- filesystem watch ---------------------------------------------------

    fn refresh_watcher(&mut self) {
        if !self.watch_requested || is_drives(&self.dir) {
            self.watcher = None;
            return;
        }
        if self.watcher.as_ref().is_some_and(|state| state.dir == self.dir) {
            return;
        }
        let (tx, rx) = mpsc::channel();
        match notify::RecommendedWatcher::new(move |event| _ = tx.send(event), notify::Config::default()) {
            Ok(mut watcher) => match watcher.watch(&self.dir, notify::RecursiveMode::NonRecursive) {
                Ok(()) => self.watcher = Some(WatchState { dir: self.dir.clone(), _watcher: watcher, rx }),
                Err(err) => {
                    self.watcher = None;
                    self.status = Some(format!("watch unavailable: {err}"));
                }
            },
            Err(err) => {
                self.watcher = None;
                self.status = Some(format!("watch unavailable: {err}"));
            }
        }
    }

    fn pump_watch(&mut self) {
        let Some(state) = &self.watcher else {
            return;
        };
        let mut changed = false;
        while let Ok(event) = state.rx.try_recv() {
            // Watch errors (notably the WSL2/DrvFs unreliability on Windows
            // mounts) degrade to manual `r`: never an error, never a hang.
            if event.is_ok() {
                changed = true;
            }
        }
        if changed {
            self.watch_dirty = Some(Instant::now());
        }
    }

    /// Periodic work: debounce the metadata preview and coalesce bursts of
    /// watch events into a single refresh.
    fn tick(&mut self) {
        if let Some((path, since)) = self.preview_pending.clone()
            && since.elapsed() >= Duration::from_millis(50)
        {
            self.preview_pending = None;
            if self.selected_file().as_ref() == Some(&path) {
                self.request_preview(path);
            }
        }
        if let Some(since) = self.watch_dirty
            && since.elapsed() >= Duration::from_millis(250)
        {
            self.watch_dirty = None;
            let dir = self.dir.clone();
            self.request_scan(dir, false);
        }
    }

    // -- selection ----------------------------------------------------------

    fn selected_file(&self) -> Option<PathBuf> {
        match self.entries.get(self.file_cursor) {
            Some(entry) if !entry.is_dir => Some(entry.path.clone()),
            _ => None,
        }
    }

    fn selection_changed(&mut self) {
        match self.selected_file() {
            Some(path) => {
                if self.preview_path.as_ref() != Some(&path) && self.preview_loading.as_ref().is_none_or(|(loading, _)| *loading != path) {
                    self.preview_pending = Some((path, Instant::now()));
                }
            }
            None => {
                self.preview_path = None;
                self.tree = None;
                self.preview_error = None;
                self.preview_loading = None;
                self.preview_pending = None;
            }
        }
    }

    fn navigate_to(&mut self, dir: PathBuf) {
        self.dir_states.insert(self.dir.clone(), self.file_cursor);
        self.preview_path = None;
        self.tree = None;
        self.preview_error = None;
        self.preview_loading = None;
        self.preview_pending = None;
        self.restore_drill = None;
        self.request_scan(dir, true);
    }

    // -- keys ---------------------------------------------------------------

    fn on_key(&mut self, key: KeyEvent) {
        if matches!(key.kind, KeyEventKind::Release) {
            return;
        }
        self.status = None;
        if self.overlay.is_some() {
            self.overlay_key(key);
            return;
        }
        match key.code {
            KeyCode::F(1) => {
                self.overlay = Some(Overlay::About);
                return;
            }
            KeyCode::Tab => {
                self.focus = match self.focus {
                    Focus::Files => Focus::Meta,
                    Focus::Meta => Focus::Files,
                };
                return;
            }
            KeyCode::Char('/') => {
                self.overlay = Some(Overlay::Search { input: Input::default(), panel: self.focus });
                return;
            }
            KeyCode::Char('w') if key.modifiers == KeyModifiers::NONE => {
                self.wipe_key();
                return;
            }
            KeyCode::Char('q') if key.modifiers == KeyModifiers::NONE => {
                self.quit = true;
                return;
            }
            KeyCode::Esc => {
                // Give up waiting on an in-flight scan; the abandoned worker
                // finishes on its own and its result is discarded on arrival.
                self.pending_dir = None;
                return;
            }
            _ => {}
        }
        match self.focus {
            Focus::Files => self.files_key(key),
            Focus::Meta => self.meta_key(key),
        }
    }

    fn files_key(&mut self, key: KeyEvent) {
        match (key.code, key.modifiers) {
            (KeyCode::Up, KeyModifiers::NONE) => self.move_file_cursor(-1),
            (KeyCode::Down, KeyModifiers::NONE) => self.move_file_cursor(1),
            (KeyCode::Right, KeyModifiers::NONE) | (KeyCode::Enter, KeyModifiers::NONE) => {
                if self.pending_dir.is_some() {
                    return;
                }
                match self.entries.get(self.file_cursor).cloned() {
                    // Entering `..` via Right would duplicate Left; going up is Left's job.
                    Some(entry) if entry.is_dir && !entry.is_parent => self.navigate_to(entry.path),
                    _ => {}
                }
            }
            (KeyCode::Left, KeyModifiers::NONE) => {
                if self.pending_dir.is_some() {
                    return;
                }
                if let Some(parent) = parent_dir(&self.dir) {
                    self.navigate_to(parent);
                }
            }
            (KeyCode::Char('c'), KeyModifiers::NONE) => {
                if let Some(name) = self.entries.get(self.file_cursor).map(|entry| entry.name.clone()) {
                    self.copy_to_clipboard(&name);
                }
            }
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                if let Some(path) = self.entries.get(self.file_cursor).map(|entry| entry.path.display().to_string()) {
                    self.copy_to_clipboard(&path);
                }
            }
            (KeyCode::Char('r'), KeyModifiers::NONE) => {
                if self.pending_dir.is_none() {
                    let dir = self.dir.clone();
                    self.request_scan(dir, false);
                }
            }
            _ => {}
        }
    }

    fn meta_key(&mut self, key: KeyEvent) {
        match (key.code, key.modifiers) {
            (KeyCode::Up, KeyModifiers::NONE) => {
                if let Some(tree) = self.tree.as_mut() {
                    tree.move_cursor(-1);
                }
            }
            (KeyCode::Down, KeyModifiers::NONE) => {
                if let Some(tree) = self.tree.as_mut() {
                    tree.move_cursor(1);
                }
            }
            (KeyCode::Right, KeyModifiers::NONE) => {
                if let Some(tree) = self.tree.as_mut() {
                    tree.drill_into_selected();
                }
            }
            (KeyCode::Left, KeyModifiers::NONE) => {
                if let Some(tree) = self.tree.as_mut() {
                    tree.drill_up();
                }
            }
            (KeyCode::Enter, KeyModifiers::NONE) => {
                let selected = self.tree.as_ref().and_then(|tree| tree.selected());
                match selected {
                    Some((_, row)) if row.is_branch => {
                        if let Some(tree) = self.tree.as_mut() {
                            tree.drill_into_selected();
                        }
                    }
                    Some((path, _)) => self.open_leaf_editor(path),
                    None => {}
                }
            }
            (KeyCode::Char('e'), KeyModifiers::NONE) => self.open_subtree_editor(),
            (KeyCode::Char('n'), KeyModifiers::NONE) => {
                if let Some(tree) = self.tree.as_ref() {
                    let array = matches!(tree.current_node(), Value::Array(_));
                    self.overlay = Some(if array { Overlay::NewValue { input: Input::default(), key: None, error: None } } else { Overlay::NewKey { input: Input::default(), error: None } });
                }
            }
            (KeyCode::Char('d'), KeyModifiers::NONE) => {
                let selected = self.tree.as_ref().and_then(|tree| tree.selected()).map(|(path, _)| path);
                if let Some(path) = selected {
                    let label = crate::tui_tree::dot_path(&path);
                    self.confirm(format!("Delete '{label}'?"), ConfirmAction::DeleteNode(path.clone()), |app| app.delete_node(&path));
                }
            }
            (KeyCode::Char('c'), KeyModifiers::NONE) => {
                if let Some(text) = self.selected_copy_text() {
                    self.copy_to_clipboard(&text);
                }
            }
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                if let Some(tree) = self.tree.as_ref()
                    && let Some((path, _)) = tree.selected()
                {
                    self.copy_to_clipboard(&crate::tui_tree::dot_path(&path));
                }
            }
            _ => {}
        }
    }

    fn move_file_cursor(&mut self, delta: isize) {
        if self.entries.is_empty() {
            return;
        }
        let next = self.file_cursor as isize + delta;
        self.file_cursor = next.clamp(0, self.entries.len() as isize - 1) as usize;
        self.selection_changed();
    }

    fn confirm(&mut self, message: String, action: ConfirmAction, immediate: impl FnOnce(&mut Self)) {
        if self.power {
            immediate(self);
        } else {
            self.overlay = Some(Overlay::Confirm { message, action, yes: false });
        }
    }

    fn wipe_key(&mut self) {
        let Some(path) = self.preview_path.clone() else {
            self.status = Some("no file selected".to_string());
            return;
        };
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
        self.confirm(format!("Wipe all metadata from '{name}'?"), ConfirmAction::WipeFile(path.clone()), |app| app.wipe_file_at(&path));
    }

    fn delete_node(&mut self, path: &[Segment]) {
        match self.commit_edit(path, &Edit::Delete) {
            Ok(()) => {}
            Err(message) => self.status = Some(message),
        }
    }

    fn wipe_file_at(&mut self, path: &Path) {
        match wipe_file(path) {
            Ok(()) => {
                self.status = Some(format!("wiped '{}'", path.file_name().map(|name| name.to_string_lossy()).unwrap_or_default()));
                self.reload_preview();
            }
            Err(message) => self.status = Some(message),
        }
    }

    fn open_leaf_editor(&mut self, path: Vec<Segment>) {
        let text = self.tree.as_ref().and_then(|tree| tree.value_at(&path)).map(crate::tui_tree::leaf_text).unwrap_or_default();
        let mut input = Input::new(text);
        input.handle(tui_input::InputRequest::GoToEnd);
        self.overlay = Some(Overlay::LeafEdit { input, path, error: None });
    }

    fn open_subtree_editor(&mut self) {
        let selected = self.tree.as_ref().and_then(|tree| tree.selected()).map(|(path, _)| path);
        let Some(path) = selected else {
            return;
        };
        let text = self.tree.as_ref().and_then(|tree| tree.value_at(&path)).map(|value| serde_json::to_string_pretty(value).unwrap_or_default()).unwrap_or_default();
        let lines = text.lines().map(|line| Input::new(line.to_string())).collect::<Vec<_>>();
        let lines = if lines.is_empty() { vec![Input::default()] } else { lines };
        self.overlay = Some(Overlay::SubtreeEdit { lines, line: 0, path, error: None });
    }

    fn selected_copy_text(&self) -> Option<String> {
        let tree = self.tree.as_ref()?;
        let (path, _) = tree.selected()?;
        tree.value_at(&path).map(crate::tui_tree::copy_text)
    }

    fn copy_to_clipboard(&mut self, text: &str) {
        match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text.to_string())) {
            Ok(()) => self.status = Some("copied".to_string()),
            Err(err) => self.status = Some(format!("copy failed: {err}")),
        }
    }

    // -- overlay keys -------------------------------------------------------

    fn overlay_key(&mut self, key: KeyEvent) {
        let overlay = self.overlay.take();
        match overlay {
            None => {}
            Some(Overlay::About) => match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::F(1) | KeyCode::Char('q') => {}
                _ => self.overlay = Some(Overlay::About),
            },
            Some(Overlay::Confirm { message, action, mut yes }) => match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter if key.modifiers == KeyModifiers::NONE => {
                    if yes {
                        self.confirm_action(action);
                    }
                }
                KeyCode::Left | KeyCode::Right | KeyCode::Tab if key.modifiers == KeyModifiers::NONE => {
                    yes = !yes;
                    self.overlay = Some(Overlay::Confirm { message, action, yes });
                }
                KeyCode::Char('y' | 'Y') if key.modifiers == KeyModifiers::NONE => self.confirm_action(action),
                KeyCode::Char('n' | 'N') if key.modifiers == KeyModifiers::NONE => {}
                _ => self.overlay = Some(Overlay::Confirm { message, action, yes }),
            },
            Some(Overlay::Search { mut input, panel }) => match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter if key.modifiers == KeyModifiers::NONE => self.search_jump(input.value(), panel),
                _ => {
                    input_key(&mut input, key);
                    self.overlay = Some(Overlay::Search { input, panel });
                }
            },
            Some(Overlay::LeafEdit { mut input, path, error: _ }) => match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter if key.modifiers == KeyModifiers::NONE => {
                    let typed = input.value().to_string();
                    match self.leaf_value(&path, &typed) {
                        Ok(value) => match self.commit_edit(&path, &Edit::Set(value)) {
                            Ok(()) => {}
                            Err(message) => {
                                self.overlay = Some(Overlay::LeafEdit { input, path, error: Some(message) });
                            }
                        },
                        Err(message) => {
                            self.overlay = Some(Overlay::LeafEdit { input, path, error: Some(message) });
                        }
                    }
                }
                _ => {
                    input_key(&mut input, key);
                    self.overlay = Some(Overlay::LeafEdit { input, path, error: None });
                }
            },
            Some(Overlay::NewKey { mut input, error: _ }) => match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter if key.modifiers == KeyModifiers::NONE => {
                    if input.value().trim().is_empty() {
                        self.overlay = Some(Overlay::NewKey { input, error: Some("key must not be empty".to_string()) });
                    } else {
                        let key_name = input.value().to_string();
                        self.overlay = Some(Overlay::NewValue { input: Input::default(), key: Some(key_name), error: None });
                    }
                }
                _ => {
                    input_key(&mut input, key);
                    self.overlay = Some(Overlay::NewKey { input, error: None });
                }
            },
            Some(Overlay::NewValue { mut input, key: name, error: _ }) => match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter if key.modifiers == KeyModifiers::NONE => {
                    let typed = input.value().to_string();
                    match parse_json5(&typed) {
                        Ok(value) => {
                            let drill = self.tree.as_ref().map(|tree| tree.drill().to_vec()).unwrap_or_default();
                            let (target, edit) = match &name {
                                Some(key) => {
                                    let mut path = drill;
                                    path.push(Segment::Key(key.clone()));
                                    (path, Edit::Set(value))
                                }
                                None => (drill, Edit::Append(value)),
                            };
                            match self.commit_edit(&target, &edit) {
                                Ok(()) => {}
                                Err(message) => {
                                    self.overlay = Some(Overlay::NewValue { input, key: name, error: Some(message) });
                                }
                            }
                        }
                        Err(message) => {
                            self.overlay = Some(Overlay::NewValue { input, key: name, error: Some(message) });
                        }
                    }
                }
                _ => {
                    input_key(&mut input, key);
                    self.overlay = Some(Overlay::NewValue { input, key: name, error: None });
                }
            },
            Some(Overlay::SubtreeEdit { mut lines, mut line, path, mut error }) => {
                self.subtree_key(key, &mut lines, &mut line, path, &mut error);
            }
        }
    }

    fn subtree_key(&mut self, key: KeyEvent, lines: &mut Vec<Input>, line: &mut usize, path: Vec<Segment>, error: &mut Option<String>) {
        match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => {}
            (KeyCode::Enter, KeyModifiers::NONE) => {
                let text: Vec<&str> = lines.iter().map(|input| input.value()).collect();
                match parse_json5(&text.join("\n")) {
                    Ok(value) => match self.commit_edit(&path, &Edit::Set(value)) {
                        Ok(()) => {}
                        Err(message) => {
                            *error = Some(message);
                            self.overlay = Some(Overlay::SubtreeEdit { lines: std::mem::take(lines), line: *line, path, error: error.clone() });
                        }
                    },
                    Err(message) => {
                        *error = Some(message);
                        self.overlay = Some(Overlay::SubtreeEdit { lines: std::mem::take(lines), line: *line, path, error: error.clone() });
                    }
                }
            }
            // A newline inside the multi-line editor (Enter itself confirms).
            (KeyCode::Enter, KeyModifiers::ALT) | (KeyCode::Char('j'), KeyModifiers::CONTROL) => {
                *error = None;
                split_line(lines, line);
                self.overlay = Some(Overlay::SubtreeEdit { lines: std::mem::take(lines), line: *line, path, error: None });
            }
            (KeyCode::Up, KeyModifiers::NONE) => {
                *line = line.saturating_sub(1);
                self.overlay = Some(Overlay::SubtreeEdit { lines: std::mem::take(lines), line: *line, path, error: error.clone() });
            }
            (KeyCode::Down, KeyModifiers::NONE) => {
                *line = (*line + 1).min(lines.len().saturating_sub(1));
                self.overlay = Some(Overlay::SubtreeEdit { lines: std::mem::take(lines), line: *line, path, error: error.clone() });
            }
            (KeyCode::Backspace, KeyModifiers::NONE) if lines[*line].cursor() == 0 && *line > 0 => {
                *error = None;
                join_with_previous(lines, line);
                self.overlay = Some(Overlay::SubtreeEdit { lines: std::mem::take(lines), line: *line, path, error: None });
            }
            (KeyCode::Delete, KeyModifiers::NONE) if lines[*line].cursor() == lines[*line].value().chars().count() && *line + 1 < lines.len() => {
                *error = None;
                join_with_next(lines, line);
                self.overlay = Some(Overlay::SubtreeEdit { lines: std::mem::take(lines), line: *line, path, error: None });
            }
            (KeyCode::Left, KeyModifiers::NONE) if lines[*line].cursor() == 0 && *line > 0 => {
                *line -= 1;
                lines[*line].handle(tui_input::InputRequest::GoToEnd);
                self.overlay = Some(Overlay::SubtreeEdit { lines: std::mem::take(lines), line: *line, path, error: error.clone() });
            }
            (KeyCode::Right, KeyModifiers::NONE) if lines[*line].cursor() == lines[*line].value().chars().count() && *line + 1 < lines.len() => {
                *line += 1;
                lines[*line].handle(tui_input::InputRequest::GoToStart);
                self.overlay = Some(Overlay::SubtreeEdit { lines: std::mem::take(lines), line: *line, path, error: error.clone() });
            }
            _ => {
                *error = None;
                input_key(&mut lines[*line], key);
                self.overlay = Some(Overlay::SubtreeEdit { lines: std::mem::take(lines), line: *line, path, error: None });
            }
        }
    }

    fn confirm_action(&mut self, action: ConfirmAction) {
        match action {
            ConfirmAction::DeleteNode(path) => self.delete_node(&path),
            ConfirmAction::WipeFile(path) => self.wipe_file_at(&path),
        }
    }

    fn search_jump(&mut self, needle: &str, panel: Focus) {
        match panel {
            Focus::Files => {
                let needle = needle.to_lowercase();
                if let Some(index) = self.entries.iter().position(|entry| entry.name.to_lowercase().contains(&needle)) {
                    self.file_cursor = index;
                    self.selection_changed();
                } else {
                    self.status = Some("no matches found".to_string());
                }
            }
            Focus::Meta => {
                if let Some(tree) = self.tree.as_ref() {
                    let matches = tree.search(needle);
                    if let Some(first) = matches.into_iter().next() {
                        if let Some(tree) = self.tree.as_mut() {
                            tree.jump_to(&first);
                        }
                    } else {
                        self.status = Some("no matches found".to_string());
                    }
                }
            }
        }
    }

    // -- writes -------------------------------------------------------------

    /// The confirmed value of a leaf edit: strings stay raw text (what you
    /// type is what is stored), anything else parses as lenient JSON5.
    fn leaf_value(&self, path: &[Segment], typed: &str) -> Result<Value, String> {
        let current = self.tree.as_ref().and_then(|tree| tree.value_at(path));
        if matches!(current, Some(Value::String(_))) {
            return Ok(Value::String(typed.to_string()));
        }
        parse_json5(typed)
    }

    /// Validate and immediately persist one tree edit through the same
    /// atomic-write-and-verify pipeline `--set` uses, then reload the preview.
    fn commit_edit(&mut self, path: &[Segment], edit: &Edit) -> Result<(), String> {
        let file = self.preview_path.clone().ok_or_else(|| "no file loaded".to_string())?;
        let tree = self.tree.as_ref().ok_or_else(|| "no file loaded".to_string())?;
        let payload = tree.build_edit_payload(&path, edit);
        let text = serde_json::to_string(&payload).map_err(|err| format!("cannot encode edit: {err}"))?;
        let parsed = crate::merge::parse_set_payload(&text).map_err(|err| err.to_string())?;
        let bytes = std::fs::read(&file).map_err(|err| format!("cannot read file: {err}"))?;
        let format = crate::format::detect(&bytes).ok_or_else(|| "unsupported image format".to_string())?;
        let outcome = crate::write::apply_set(&bytes, format, std::slice::from_ref(&parsed)).map_err(|err| err.to_string())?;
        if outcome.changed {
            crate::write::verify(&bytes, &outcome.bytes, format).map_err(|err| err.to_string())?;
            crate::write::write_atomic(&file, &outcome.bytes).map_err(|err| err.to_string())?;
        }
        self.reload_preview();
        Ok(())
    }

    fn reload_preview(&mut self) {
        if let Some(tree) = self.tree.as_ref()
            && let Some(path) = self.preview_path.clone()
        {
            self.restore_drill = Some((path.clone(), tree.drill().to_vec(), tree.cursor()));
            self.preview_loading = None;
            self.preview_pending = None;
            self.request_preview(path);
        }
    }
}

/// Route one key to a single-line input: `tui-input` already covers cursor
/// movement, deletion, and Ctrl+Left/Right word jumps; Alt+Left/Right is
/// mapped here as the required alias.
fn input_key(input: &mut Input, key: KeyEvent) {
    use tui_input::InputRequest::{GoToNextWord, GoToPrevWord};
    match (key.code, key.modifiers) {
        (KeyCode::Left, KeyModifiers::ALT) | (KeyCode::Left, KeyModifiers::META) => {
            input.handle(GoToPrevWord);
        }
        (KeyCode::Right, KeyModifiers::ALT) | (KeyCode::Right, KeyModifiers::META) => {
            input.handle(GoToNextWord);
        }
        _ => {
            input.handle_event(&Event::Key(key));
        }
    }
}

/// Split the active editor line at the cursor into two lines.
fn split_line(lines: &mut Vec<Input>, line: &mut usize) {
    let cursor = lines[*line].cursor();
    let value = lines[*line].value().to_string();
    let (head, tail) = split_at_char(&value, cursor);
    lines[*line] = Input::new(head);
    lines[*line].handle(tui_input::InputRequest::GoToEnd);
    let mut rest = Input::new(tail);
    rest.handle(tui_input::InputRequest::GoToStart);
    lines.insert(*line + 1, rest);
    *line += 1;
}

fn join_with_previous(lines: &mut Vec<Input>, line: &mut usize) {
    let current = lines[*line].value().to_string();
    let previous = lines[*line - 1].value().to_string();
    let boundary = previous.chars().count();
    let mut joined = Input::new(format!("{previous}{current}"));
    joined.handle(tui_input::InputRequest::GoToStart);
    for _ in 0..boundary {
        joined.handle(tui_input::InputRequest::GoToNextChar);
    }
    lines[*line - 1] = joined;
    lines.remove(*line);
    *line -= 1;
}

fn join_with_next(lines: &mut Vec<Input>, line: &mut usize) {
    let current = lines[*line].value().to_string();
    let cursor = lines[*line].cursor();
    let next = lines[*line + 1].value().to_string();
    let mut joined = Input::new(format!("{current}{next}"));
    joined.handle(tui_input::InputRequest::GoToStart);
    for _ in 0..cursor {
        joined.handle(tui_input::InputRequest::GoToNextChar);
    }
    lines[*line] = joined;
    lines.remove(*line + 1);
}

fn split_at_char(text: &str, cursor: usize) -> (String, String) {
    let boundary = text.char_indices().map(|(i, _)| i).chain(std::iter::once(text.len())).nth(cursor).unwrap_or(text.len());
    (text[..boundary].to_string(), text[boundary..].to_string())
}

fn parse_json5(text: &str) -> Result<Value, String> {
    json5::from_str(text).map_err(|err| format!("invalid JSON: {err}"))
}

fn wipe_file(path: &Path) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|err| format!("cannot read file: {err}"))?;
    let format = crate::format::detect(&bytes).ok_or_else(|| "unsupported image format".to_string())?;
    let outcome = crate::write::apply_wipe(&bytes, format).map_err(|err| err.to_string())?;
    if outcome.changed {
        crate::write::verify(&bytes, &outcome.bytes, format).map_err(|err| err.to_string())?;
        crate::write::write_atomic(path, &outcome.bytes).map_err(|err| err.to_string())?;
    }
    Ok(())
}

fn load_stored(path: &Path) -> Result<Value, String> {
    let bytes = std::fs::read(path).map_err(|err| format!("cannot read file: {err}"))?;
    let format = crate::format::detect(&bytes).ok_or_else(|| "unsupported image format".to_string())?;
    crate::meta::read_metadata_raw(&bytes, format).map(|metadata| metadata.to_value()).map_err(|err| err.to_string())
}

/// The parent directory, or `None` at the filesystem root. On Windows the
/// parent of a drive root is the virtual drives list (an empty path).
fn parent_dir(dir: &Path) -> Option<PathBuf> {
    if is_drives(dir) {
        return None;
    }
    match dir.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => Some(parent.to_path_buf()),
        #[cfg(windows)]
        _ => Some(PathBuf::new()),
        #[cfg(not(windows))]
        _ => None,
    }
}

fn is_drives(dir: &Path) -> bool {
    dir.as_os_str().is_empty()
}

/// Scan one directory: `..` (except at the root), subdirectories, and files
/// detected as PNG/JPEG/WebP by magic bytes. Anything else is omitted.
fn scan_entries(dir: &Path) -> (Vec<FileEntry>, Option<String>) {
    if is_drives(dir) {
        #[cfg(windows)]
        return (drives(), None);
        #[cfg(not(windows))]
        return (Vec::new(), Some("cannot read directory".to_string()));
    }
    let mut entries = Vec::new();
    let read = match std::fs::read_dir(dir) {
        Ok(read) => read,
        Err(err) => return (entries, Some(format!("cannot read directory: {err}"))),
    };
    for item in read {
        let Ok(item) = item else {
            continue;
        };
        let path = item.path();
        let name = item.file_name().to_string_lossy().into_owned();
        let Ok(kind) = std::fs::metadata(&path) else {
            continue;
        };
        if kind.is_dir() {
            entries.push(FileEntry { name, path, is_dir: true, is_parent: false });
        } else if kind.is_file() && is_image(&path) {
            entries.push(FileEntry { name, path, is_dir: false, is_parent: false });
        }
    }
    entries.sort_by(|a, b| (!a.is_dir, a.name.to_lowercase()).cmp(&(!b.is_dir, b.name.to_lowercase())));
    if let Some(parent) = parent_dir(dir) {
        entries.insert(0, FileEntry { name: "..".to_string(), path: parent, is_dir: true, is_parent: true });
    }
    (entries, None)
}

fn is_image(path: &Path) -> bool {
    use std::io::Read as _;
    let mut head = [0u8; 12];
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let Ok(read) = file.read(&mut head) else {
        return false;
    };
    crate::format::detect(&head[..read]).is_some()
}

#[cfg(windows)]
fn drives() -> Vec<FileEntry> {
    let mut entries = Vec::new();
    for drive in b'A'..=b'Z' {
        let root = format!("{}:\\", drive as char);
        let path = PathBuf::from(&root);
        if path.exists() {
            entries.push(FileEntry { name: root, path, is_dir: true, is_parent: false });
        }
    }
    entries
}
