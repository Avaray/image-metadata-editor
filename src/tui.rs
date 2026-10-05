use std::collections::HashMap;
use std::io::IsTerminal as _;
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
pub fn run(initial: &Path, power: bool, watch: bool, expand: bool) -> Result<(), Error> {
    let kind = std::fs::metadata(initial).map_err(|err| Error::runtime(format!("cannot open '{}': {err}", initial.display())))?;
    let absolute = absolutize(initial);
    let (dir, select) = if kind.is_dir() {
        (absolute, None)
    } else {
        let name = absolute.file_name().map(|name| name.to_string_lossy().into_owned());
        let parent = absolute.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("/"));
        (parent, name)
    };

    // Raw mode alone cannot detect a headless launch: crossterm enables it
    // via /dev/tty (the controlling terminal), which exists even when stdio
    // is piped, e.g. under `cargo test` in a real terminal. Gate on stdio
    // explicitly so the TUI fails fast instead of drawing into a pipe.
    if !std::io::stdin().is_terminal() {
        return Err(Error::runtime("cannot start TUI: standard input is not a terminal"));
    }
    if !std::io::stdout().is_terminal() {
        return Err(Error::runtime("cannot start TUI: standard output is not a terminal"));
    }
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
    let mut app = App::new(dir, select, power, watch, expand);
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
    LeafEdit { lines: Vec<Input>, line: usize, path: Vec<Segment>, error: Option<String> },
    SubtreeEdit { lines: Vec<Input>, line: usize, path: Vec<Segment>, error: Option<String> },
    NewKey { input: Input, error: Option<String> },
    NewValue { input: Input, key: Option<String>, error: Option<String> },
    Confirm { message: String, action: ConfirmAction, yes: bool },
    Help,
}

struct ScanResp {
    dir: PathBuf,
    entries: Vec<FileEntry>,
    error: Option<String>,
    seq: u64,
    /// Phase-two message: `entries` are *additional* images found by
    /// magic-sniffing files without an image extension. Merge, don't replace.
    extra: bool,
}

struct MetaResp {
    path: PathBuf,
    stored: Result<(Vec<u8>, Value), String>,
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
    pub scan_seq: u64,
    pub dir_states: HashMap<PathBuf, usize>,
    pub initial_select: Option<String>,
    pub focus: Focus,
    pub preview_path: Option<PathBuf>,
    pub preview_bytes: Option<Vec<u8>>,
    pub preview_modified: bool,
    pub tree: Option<MetaTree>,
    pub preview_error: Option<String>,
    pub preview_loading: Option<(PathBuf, Instant)>,
    pub preview_pending: Option<(PathBuf, Instant)>,
    pub meta_scroll: usize,
    pub file_view_height: usize,
    pub meta_view_height: usize,
    pub restore_drill: Option<(PathBuf, Vec<Segment>, usize, Vec<usize>)>,
    pub overlay: Option<Overlay>,
    pub status: Option<String>,
    pub status_time: Option<Instant>,
    pub jump_back_file: Option<PathBuf>,
    pub jump_back_meta: Option<Vec<crate::tui_tree::Segment>>,
    pub power: bool,
    pub expand: bool,
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
    fn new(dir: PathBuf, select: Option<String>, power: bool, watch: bool, expand: bool) -> Self {
        let (scan_tx, scan_rx) = mpsc::channel();
        let (meta_tx, meta_rx) = mpsc::channel();
        let mut app = Self {
            dir: dir.clone(),
            entries: Vec::new(),
            scan_error: None,
            file_cursor: 0,
            file_scroll: 0,
            pending_dir: None,
            scan_seq: 0,
            dir_states: HashMap::new(),
            initial_select: select,
            focus: Focus::Files,
            preview_path: None,
            preview_bytes: None,
            preview_modified: false,
            tree: None,
            preview_error: None,
            preview_loading: None,
            preview_pending: None,
            meta_scroll: 0,
            file_view_height: 0,
            meta_view_height: 0,
            restore_drill: None,
            overlay: None,
            status: None,
            status_time: None,
            jump_back_file: None,
            jump_back_meta: None,
            power,
            expand,
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
        self.scan_seq += 1;
        let seq = self.scan_seq;
        let tx = self.scan_tx.clone();
        std::thread::spawn(move || {
            // Phase one lists directories and image extensions without
            // touching file contents, so huge directories appear instantly.
            // Phase two magic-sniffs the remaining files in the background
            // and merges whatever turns out to be an image.
            let (entries, candidates, error) = scan_entries_fast(&dir);
            let _ = tx.send(ScanResp { dir: dir.clone(), entries, error: error.clone(), seq, extra: false });
            if error.is_none() {
                let found = sniff_candidates(&candidates);
                if !found.is_empty() {
                    let _ = tx.send(ScanResp { dir, entries: found, error: None, seq, extra: true });
                }
            }
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
            if resp.extra {
                // Stale extras (a newer scan or refresh already ran) are
                // silently discarded: only the current generation may merge.
                if self.pending_dir.is_none() && resp.dir == self.dir && resp.seq == self.scan_seq {
                    self.apply_extra(resp);
                }
            } else if self.pending_dir.as_ref().is_some_and(|(dir, _)| *dir == resp.dir) {
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
                    Ok((bytes, stored)) => {
                        let mut tree = MetaTree::new(stored);
                        if let Some((path, drill, cursor, history)) = self.restore_drill.take()
                            && path == resp.path
                        {
                            tree.restore(drill, cursor, history);
                        }
                        self.tree = Some(tree);
                        self.preview_bytes = Some(bytes);
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
        let initial = self.initial_select.take();
        let mut selecting_initial = false;
        if let Some(name) = initial.as_ref() {
            if let Some(index) = self.entries.iter().position(|entry| entry.name == *name) {
                self.file_cursor = index;
                selecting_initial = true;
                if !self.entries[index].is_dir {
                    self.focus = Focus::Meta;
                }
            } else {
                self.initial_select = initial;
            }
        }
        if !selecting_initial && let Some(saved) = self.dir_states.get(&resp.dir) {
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

    /// Merge phase-two images into the visible list, keeping the cursor on
    /// the same file (entries are only added, so the selection survives).
    fn apply_extra(&mut self, resp: ScanResp) {
        let mut selected = self.entries.get(self.file_cursor).map(|entry| entry.name.clone());
        let mut focus_meta = false;

        if let Some(name) = self.initial_select.as_ref()
            && resp.entries.iter().any(|entry| entry.name == *name)
        {
            selected = Some(name.clone());
            self.initial_select = None;
            focus_meta = true;
        }

        self.entries.extend(resp.entries);
        sort_entries(&mut self.entries);
        self.file_cursor = selected.and_then(|name| self.entries.iter().position(|entry| entry.name == name)).unwrap_or(0).min(self.entries.len().saturating_sub(1));
        if focus_meta && !self.entries[self.file_cursor].is_dir {
            self.focus = Focus::Meta;
        }
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
                    self.set_status(format!("watch unavailable: {err}"));
                }
            },
            Err(err) => {
                self.watcher = None;
                self.set_status(format!("watch unavailable: {err}"));
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
        if let Some(time) = self.status_time
            && time.elapsed() >= Duration::from_secs(3)
        {
            self.status = None;
            self.status_time = None;
        }

        if let Some((path, since)) = self.preview_pending.clone()
            && since.elapsed() >= Duration::from_millis(200)
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
                // Clear stale data but intentionally leave preview_pending
                // untouched: this keeps the dots animation running while the
                // cursor passes through directory entries during fast
                // navigation. preview_pending is None at startup, so the
                // empty-state message still shows before any file is visited.
                self.preview_path = None;
                self.tree = None;
                self.preview_error = None;
                self.preview_loading = None;
            }
        }
    }

    fn navigate_to(&mut self, dir: PathBuf, mut select: Option<String>) {
        let mut keep_jump_back = false;
        if let Some(target) = &self.jump_back_file {
            if dir.as_os_str().is_empty() || target.starts_with(&dir) {
                keep_jump_back = true;
                if select.is_none() && target != &dir {
                    if dir.as_os_str().is_empty() {
                        if let Some(c) = target.components().next() {
                            let mut s = c.as_os_str().to_string_lossy().into_owned();
                            if !s.ends_with('\\') && cfg!(windows) {
                                s.push('\\');
                            }
                            select = Some(s);
                        }
                    } else if let Ok(suffix) = target.strip_prefix(&dir) {
                        select = suffix.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned());
                    }
                }
            }
        }
        if !keep_jump_back {
            self.jump_back_file = None;
        }

        self.initial_select = select;
        self.dir_states.insert(self.dir.clone(), self.file_cursor);
        self.preview_path = None;
        self.tree = None;
        self.preview_error = None;
        self.preview_loading = None;
        self.preview_pending = None;
        self.restore_drill = None;
        self.jump_back_meta = None;
        self.request_scan(dir, true);
    }

    // -- keys ---------------------------------------------------------------

    fn set_status(&mut self, text: impl Into<String>) {
        self.status = Some(text.into());
        self.status_time = Some(Instant::now());
    }

    fn on_key(&mut self, key: KeyEvent) {
        if matches!(key.kind, KeyEventKind::Release) {
            return;
        }
        self.status = None;
        self.status_time = None;
        if self.overlay.is_some() {
            self.overlay_key(key);
            return;
        }
        match key.code {
            KeyCode::F(1) => {
                self.overlay = Some(Overlay::Help);
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
                if !self.check_unsaved() {
                    self.quit = true;
                }
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
            (KeyCode::Char('s'), KeyModifiers::CONTROL) => self.save_preview(),
            (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                if self.preview_modified {
                    self.preview_modified = false;
                    self.reload_preview();
                    self.set_status("Discarded changes".to_string());
                }
            }
            (KeyCode::Up, KeyModifiers::NONE) => {
                if !self.check_unsaved() {
                    self.move_file_cursor(-1);
                }
            }
            (KeyCode::Down, KeyModifiers::NONE) => {
                if !self.check_unsaved() {
                    self.move_file_cursor(1);
                }
            }
            (KeyCode::PageUp, KeyModifiers::NONE) => {
                if !self.check_unsaved() {
                    self.page_file_cursor(-(self.file_view_height.max(1) as isize));
                }
            }
            (KeyCode::PageDown, KeyModifiers::NONE) => {
                if !self.check_unsaved() {
                    self.page_file_cursor(self.file_view_height.max(1) as isize);
                }
            }
            (KeyCode::Right, KeyModifiers::NONE) | (KeyCode::Enter, KeyModifiers::NONE) => {
                if self.pending_dir.is_some() {
                    return;
                }
                let target = self.entries.get(self.file_cursor).map(|e| (e.is_dir, e.is_parent, e.path.clone()));
                match target {
                    Some((true, false, path)) => {
                        if !self.check_unsaved() {
                            self.navigate_to(path, None);
                        }
                    }
                    Some((true, true, path)) => {
                        if !self.check_unsaved() {
                            let select = self.dir.file_name().map(|n| n.to_string_lossy().into_owned());
                            self.navigate_to(path, select);
                        }
                    }
                    Some((false, _, _)) => self.focus = Focus::Meta,
                    _ => {}
                }
            }
            (KeyCode::Left, KeyModifiers::NONE) => {
                if self.check_unsaved() {
                    return;
                }
                if self.pending_dir.is_some() {
                    return;
                }
                if let Some(parent) = parent_dir(&self.dir) {
                    let select = self.dir.file_name().map(|n| n.to_string_lossy().into_owned());
                    self.navigate_to(parent, select);
                }
            }
            (KeyCode::Left, KeyModifiers::CONTROL) => {
                if self.check_unsaved() {
                    return;
                }
                if self.pending_dir.is_some() {
                    return;
                }
                let mut current = self.dir.clone();
                while let Some(parent) = parent_dir(&current) {
                    current = parent;
                }
                if current != self.dir {
                    if self.jump_back_file.as_ref().is_none_or(|t| !t.starts_with(&self.dir)) {
                        self.jump_back_file = Some(self.dir.clone());
                    }
                    self.navigate_to(current, None);
                }
            }
            (KeyCode::Right, KeyModifiers::CONTROL) => {
                if self.check_unsaved() {
                    return;
                }
                if self.pending_dir.is_some() {
                    return;
                }
                if let Some(target) = self.jump_back_file.take() {
                    self.navigate_to(target, None);
                }
            }
            (KeyCode::Char('c'), KeyModifiers::NONE) => {
                if let Some(name) = self.entries.get(self.file_cursor).map(|entry| entry.name.clone()) {
                    self.copy_to_clipboard(&name, &format!("Copied '{name}'"));
                }
            }
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                if let Some(path) = self.entries.get(self.file_cursor).map(|entry| entry.path.display().to_string()) {
                    self.copy_to_clipboard(&path, &format!("Copied '{path}'"));
                }
            }
            (KeyCode::Char('r'), KeyModifiers::NONE) if self.pending_dir.is_none() => {
                let dir = self.dir.clone();
                self.request_scan(dir, false);
            }
            _ => {}
        }
    }

    fn meta_key(&mut self, key: KeyEvent) {
        match (key.code, key.modifiers) {
            (KeyCode::Char('s'), KeyModifiers::CONTROL) => self.save_preview(),
            (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                if self.preview_modified {
                    self.preview_modified = false;
                    self.reload_preview();
                    self.set_status("Discarded changes".to_string());
                }
            }
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
            (KeyCode::PageUp, KeyModifiers::NONE) => {
                let step = self.meta_view_height.max(1) as isize;
                if let Some(tree) = self.tree.as_mut() {
                    tree.move_cursor_clamped(-step);
                }
            }
            (KeyCode::PageDown, KeyModifiers::NONE) => {
                let step = self.meta_view_height.max(1) as isize;
                if let Some(tree) = self.tree.as_mut() {
                    tree.move_cursor_clamped(step);
                }
            }
            (KeyCode::Right, KeyModifiers::NONE) => {
                if let Some(tree) = self.tree.as_mut() {
                    tree.drill_into_selected();
                    let mut keep = false;
                    if let Some(target) = &self.jump_back_meta {
                        if target.starts_with(tree.drill()) {
                            keep = true;
                            if let Some(next_segment) = target.get(tree.drill().len()) {
                                if let Some(idx) = tree.rows(false).iter().position(|r| &r.segment == next_segment) {
                                    tree.set_cursor(idx);
                                }
                            }
                        }
                    }
                    if !keep {
                        self.jump_back_meta = None;
                    }
                }
            }
            (KeyCode::Left, KeyModifiers::NONE) => {
                let at_root = self.tree.as_ref().is_none_or(|tree| tree.drill().is_empty());
                if at_root {
                    if self.check_unsaved() {
                        return;
                    }
                    self.focus = Focus::Files;
                } else if let Some(tree) = self.tree.as_mut() {
                    tree.drill_up();
                    let mut keep = false;
                    if let Some(target) = &self.jump_back_meta {
                        if target.starts_with(tree.drill()) {
                            keep = true;
                        }
                    }
                    if !keep {
                        self.jump_back_meta = None;
                    }
                }
            }
            (KeyCode::Left, KeyModifiers::CONTROL) => {
                if let Some(tree) = self.tree.as_mut() {
                    if !tree.drill().is_empty() {
                        if self.jump_back_meta.as_ref().is_none_or(|t| !t.starts_with(tree.drill())) {
                            self.jump_back_meta = Some(tree.drill().to_vec());
                        }
                        while tree.drill_up() {}
                    }
                }
            }
            (KeyCode::Right, KeyModifiers::CONTROL) => {
                if let Some(target) = self.jump_back_meta.take() {
                    if let Some(tree) = self.tree.as_mut() {
                        tree.jump_to(&target);
                    }
                }
            }
            (KeyCode::Enter, KeyModifiers::NONE) => {
                let selected = self.tree.as_ref().and_then(|tree| tree.selected());
                match selected {
                    Some((_, row)) if row.is_branch => {
                        if let Some(tree) = self.tree.as_mut() {
                            tree.drill_into_selected();
                            let mut keep = false;
                            if let Some(target) = &self.jump_back_meta {
                                if target.starts_with(tree.drill()) {
                                    keep = true;
                                    if let Some(next_segment) = target.get(tree.drill().len()) {
                                        if let Some(idx) = tree.rows(false).iter().position(|r| &r.segment == next_segment) {
                                            tree.set_cursor(idx);
                                        }
                                    }
                                }
                            }
                            if !keep {
                                self.jump_back_meta = None;
                            }
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
                    self.copy_to_clipboard(&text, "Copied value");
                }
            }
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                if let Some(tree) = self.tree.as_ref()
                    && let Some((path, _)) = tree.selected()
                {
                    let dot_path = crate::tui_tree::dot_path(&path);
                    self.copy_to_clipboard(&dot_path, &format!("Copied '{dot_path}'"));
                }
            }
            _ => {}
        }
    }

    fn check_unsaved(&mut self) -> bool {
        if self.preview_modified {
            self.set_status("Unsaved changes! Press Ctrl+S to save, or Ctrl+D to discard.".to_string());
            true
        } else {
            false
        }
    }

    fn move_file_cursor(&mut self, delta: isize) {
        if self.entries.is_empty() {
            return;
        }
        let len = self.entries.len() as isize;
        self.file_cursor = (self.file_cursor as isize + delta).rem_euclid(len) as usize;
        self.selection_changed();
    }

    fn page_file_cursor(&mut self, delta: isize) {
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
            self.set_status("no file selected".to_string());
            return;
        };
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
        self.confirm(format!("Wipe all metadata from '{name}'?"), ConfirmAction::WipeFile(path.clone()), |app| app.wipe_file_at(&path));
    }

    fn delete_node(&mut self, path: &[Segment]) {
        match self.commit_edit(path, &Edit::Delete) {
            Ok(()) => {}
            Err(message) => self.set_status(message),
        }
    }

    fn save_preview(&mut self) {
        if !self.preview_modified {
            return;
        }
        let Some(path) = self.preview_path.as_ref() else { return };
        let Some(bytes) = self.preview_bytes.as_ref() else { return };
        match crate::write::write_atomic(path, bytes) {
            Ok(()) => {
                self.preview_modified = false;
                self.set_status("File saved".to_string());
            }
            Err(err) => self.set_status(format!("save failed: {err}")),
        }
    }

    fn wipe_file_at(&mut self, path: &Path) {
        let Some(bytes) = self.preview_bytes.as_ref() else { return };
        let format = match crate::format::detect(bytes) {
            Some(f) => f,
            None => return,
        };
        match crate::write::apply_wipe(bytes, format) {
            Ok(outcome) => {
                if outcome.changed {
                    if let Err(err) = crate::write::verify(bytes, &outcome.bytes, format) {
                        self.set_status(err.to_string());
                        return;
                    }
                    self.preview_bytes = Some(outcome.bytes);
                    self.preview_modified = true;
                    self.reload_preview_memory();
                }
                self.set_status(format!("wiped '{}'", path.file_name().map(|name| name.to_string_lossy()).unwrap_or_default()));
            }
            Err(err) => self.set_status(err.to_string()),
        }
    }

    fn open_leaf_editor(&mut self, path: Vec<Segment>) {
        let text = self.tree.as_ref().and_then(|tree| tree.value_at(&path)).map(crate::tui_tree::leaf_text).unwrap_or_default();
        let lines = text
            .lines()
            .map(|line| {
                let mut input = Input::new(line.to_string());
                input.handle(tui_input::InputRequest::GoToEnd);
                input
            })
            .collect::<Vec<_>>();
        let lines = if lines.is_empty() { vec![Input::default()] } else { lines };
        self.overlay = Some(Overlay::LeafEdit { lines, line: 0, path, error: None });
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

    fn copy_to_clipboard(&mut self, text: &str, label: &str) {
        match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text.to_string())) {
            Ok(()) => self.set_status(label.to_string()),
            Err(err) => self.set_status(format!("copy failed: {err}")),
        }
    }

    // -- overlay keys -------------------------------------------------------

    fn overlay_key(&mut self, key: KeyEvent) {
        let overlay = self.overlay.take();
        match overlay {
            None => {}
            Some(Overlay::Help) => match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::F(1) | KeyCode::Char('q') => {}
                _ => self.overlay = Some(Overlay::Help),
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
            Some(Overlay::LeafEdit { mut lines, mut line, path, mut error }) => {
                self.multi_line_key(key, &mut lines, &mut line, path, &mut error, false);
            }
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
                    match parse_leaf_value(&typed) {
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
                self.multi_line_key(key, &mut lines, &mut line, path, &mut error, true);
            }
        }
    }

    fn multi_line_key(&mut self, key: KeyEvent, lines: &mut Vec<Input>, line: &mut usize, path: Vec<Segment>, error: &mut Option<String>, json: bool) {
        let make_overlay = |lines, line, error| {
            if json { Overlay::SubtreeEdit { lines, line, path: path.clone(), error } } else { Overlay::LeafEdit { lines, line, path: path.clone(), error } }
        };
        match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => {}
            (KeyCode::Enter, KeyModifiers::NONE) => {
                let text: Vec<&str> = lines.iter().map(|input| input.value()).collect();
                let parsed = if json { parse_json5(&text.join("\n")) } else { parse_leaf_value(&text.join("\n")) };
                match parsed {
                    Ok(value) => match self.commit_edit(&path, &Edit::Set(value)) {
                        Ok(()) => {}
                        Err(message) => {
                            *error = Some(message);
                            self.overlay = Some(make_overlay(std::mem::take(lines), *line, error.clone()));
                        }
                    },
                    Err(message) => {
                        *error = Some(message);
                        self.overlay = Some(make_overlay(std::mem::take(lines), *line, error.clone()));
                    }
                }
            }
            // A newline inside the multi-line editor (Enter itself confirms).
            (KeyCode::Enter, KeyModifiers::SHIFT) | (KeyCode::Char('j'), KeyModifiers::CONTROL) => {
                *error = None;
                split_line(lines, line);
                self.overlay = Some(make_overlay(std::mem::take(lines), *line, None));
            }
            (KeyCode::Up, KeyModifiers::NONE) => {
                *line = line.saturating_sub(1);
                self.overlay = Some(make_overlay(std::mem::take(lines), *line, error.clone()));
            }
            (KeyCode::Down, KeyModifiers::NONE) => {
                *line = (*line + 1).min(lines.len().saturating_sub(1));
                self.overlay = Some(make_overlay(std::mem::take(lines), *line, error.clone()));
            }
            (KeyCode::Backspace, KeyModifiers::NONE) if lines[*line].cursor() == 0 && *line > 0 => {
                *error = None;
                join_with_previous(lines, line);
                self.overlay = Some(make_overlay(std::mem::take(lines), *line, None));
            }
            (KeyCode::Delete, KeyModifiers::NONE) if lines[*line].cursor() == lines[*line].value().chars().count() && *line + 1 < lines.len() => {
                *error = None;
                join_with_next(lines, line);
                self.overlay = Some(make_overlay(std::mem::take(lines), *line, None));
            }
            (KeyCode::Left, KeyModifiers::NONE) if lines[*line].cursor() == 0 && *line > 0 => {
                *line -= 1;
                lines[*line].handle(tui_input::InputRequest::GoToEnd);
                self.overlay = Some(make_overlay(std::mem::take(lines), *line, error.clone()));
            }
            (KeyCode::Right, KeyModifiers::NONE) if lines[*line].cursor() == lines[*line].value().chars().count() && *line + 1 < lines.len() => {
                *line += 1;
                lines[*line].handle(tui_input::InputRequest::GoToStart);
                self.overlay = Some(make_overlay(std::mem::take(lines), *line, error.clone()));
            }
            _ => {
                *error = None;
                input_key(&mut lines[*line], key);
                self.overlay = Some(make_overlay(std::mem::take(lines), *line, None));
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
                    self.set_status("no matches found".to_string());
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
                        self.set_status("no matches found".to_string());
                    }
                }
            }
        }
    }

    // -- writes -------------------------------------------------------------

    /// Validate and immediately persist one tree edit through the same
    /// atomic-write-and-verify pipeline `--set` uses, then reload the preview.
    fn commit_edit(&mut self, path: &[Segment], edit: &Edit) -> Result<(), String> {
        let bytes = self.preview_bytes.as_ref().ok_or_else(|| "no file loaded".to_string())?;
        let tree = self.tree.as_ref().ok_or_else(|| "no file loaded".to_string())?;
        let payload = tree.build_edit_payload(path, edit);
        let text = serde_json::to_string(&payload).map_err(|err| format!("cannot encode edit: {err}"))?;
        let parsed = crate::merge::parse_set_payload(&text).map_err(|err| err.to_string())?;
        let format = crate::format::detect(bytes).ok_or_else(|| "unsupported image format".to_string())?;
        let outcome = crate::write::apply_set(bytes, format, std::slice::from_ref(&parsed)).map_err(|err| err.to_string())?;
        if outcome.changed {
            crate::write::verify(bytes, &outcome.bytes, format).map_err(|err| err.to_string())?;
            self.preview_bytes = Some(outcome.bytes);
            self.preview_modified = true;
        }
        self.reload_preview_memory();
        Ok(())
    }

    fn reload_preview(&mut self) {
        if let Some(tree) = self.tree.as_ref()
            && let Some(path) = self.preview_path.clone()
        {
            self.restore_drill = Some((path.clone(), tree.drill().to_vec(), tree.cursor(), tree.cursor_history().to_vec()));
            self.preview_loading = None;
            self.preview_pending = None;
            self.request_preview(path);
        }
    }

    fn reload_preview_memory(&mut self) {
        if let Some(tree) = self.tree.as_ref()
            && let Some(bytes) = self.preview_bytes.as_ref()
        {
            let format = match crate::format::detect(bytes) {
                Some(f) => f,
                None => {
                    self.preview_error = Some("unsupported image format".to_string());
                    return;
                }
            };
            match crate::meta::read_metadata_raw(bytes, format) {
                Ok(metadata) => {
                    let mut new_tree = MetaTree::new(metadata.to_value());
                    new_tree.restore(tree.drill().to_vec(), tree.cursor(), tree.cursor_history().to_vec());
                    self.tree = Some(new_tree);
                    self.preview_error = None;
                }
                Err(err) => {
                    self.preview_error = Some(err.to_string());
                }
            }
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

/// The confirmed value of a leaf edit or `n` value prompt: text starting
/// with `{`/`[` parses strictly as JSON5 (errors stay open), anything else
/// tries JSON5 first and falls back to a plain string.
fn parse_leaf_value(typed: &str) -> Result<Value, String> {
    if typed.trim_start().starts_with(['{', '[']) {
        return parse_json5(typed);
    }
    match parse_json5(typed) {
        Ok(value) => Ok(value),
        Err(_) => Ok(Value::String(typed.to_string())),
    }
}

fn load_stored(path: &Path) -> Result<(Vec<u8>, Value), String> {
    let bytes = std::fs::read(path).map_err(|err| format!("cannot read file: {err}"))?;
    let format = crate::format::detect(&bytes).ok_or_else(|| "unsupported image format".to_string())?;
    crate::meta::read_metadata_raw(&bytes, format).map(|metadata| (bytes, metadata.to_value())).map_err(|err| err.to_string())
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

/// Phase one of the directory scan: `..` (except at the root),
/// subdirectories, and files with an image extension. Reads names only — no
/// `stat` and no file contents — so even huge directories list instantly.
/// Everything else that is a regular file comes back as `candidates` for the
/// phase-two magic sniff, which finds images under unusual names.
fn scan_entries_fast(dir: &Path) -> (Vec<FileEntry>, Vec<PathBuf>, Option<String>) {
    if is_drives(dir) {
        #[cfg(windows)]
        return (drives(), Vec::new(), None);
        #[cfg(not(windows))]
        return (Vec::new(), Vec::new(), Some("cannot read directory".to_string()));
    }
    let mut entries = Vec::new();
    let mut candidates = Vec::new();
    let read = match std::fs::read_dir(dir) {
        Ok(read) => read,
        Err(err) => return (entries, candidates, Some(format!("cannot read directory: {err}"))),
    };
    for item in read {
        let Ok(item) = item else {
            continue;
        };
        let path = item.path();
        let name = item.file_name().to_string_lossy().into_owned();
        // `file_type` comes free with the directory entry; only symlinks
        // need a resolving `stat`, and those are rare.
        let is_dir = match item.file_type() {
            Ok(kind) if kind.is_dir() => true,
            Ok(kind) if kind.is_file() => false,
            Ok(_) => match std::fs::metadata(&path) {
                Ok(resolved) if resolved.is_dir() => true,
                Ok(resolved) if resolved.is_file() => false,
                _ => continue,
            },
            Err(_) => continue,
        };
        if is_dir {
            entries.push(FileEntry { name, path, is_dir: true, is_parent: false });
        } else if has_image_extension(&name) {
            entries.push(FileEntry { name, path, is_dir: false, is_parent: false });
        } else {
            candidates.push(path);
        }
    }
    sort_entries(&mut entries);
    if let Some(parent) = parent_dir(dir) {
        entries.insert(0, FileEntry { name: "..".to_string(), path: parent, is_dir: true, is_parent: true });
    }
    (entries, candidates, None)
}

/// Phase two of the directory scan: magic-sniff the files phase one could
/// not classify by name. Runs in the background after the list is already
/// visible, so its cost never blocks navigation.
fn sniff_candidates(candidates: &[PathBuf]) -> Vec<FileEntry> {
    let mut found = Vec::new();
    for path in candidates {
        if is_image(path) {
            let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            found.push(FileEntry { name, path: path.clone(), is_dir: false, is_parent: false });
        }
    }
    found
}

/// Directories first, then names case-insensitively. Keys are computed once
/// per entry: a naive `sort_by_key` would re-lowercase every name on each of
/// its O(n log n) comparisons.
fn sort_entries(entries: &mut [FileEntry]) {
    entries.sort_by_cached_key(|entry| (!entry.is_dir, entry.name.to_lowercase()));
}

fn has_image_extension(name: &str) -> bool {
    match Path::new(name).extension().and_then(|ext| ext.to_str()) {
        Some(ext) => ext.eq_ignore_ascii_case("png") || ext.eq_ignore_ascii_case("jpg") || ext.eq_ignore_ascii_case("jpeg") || ext.eq_ignore_ascii_case("webp"),
        None => false,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Phase one must classify by name alone: garbage bytes under an image
    /// extension are still listed (the old magic-sniff-everything scan
    /// omitted them — and paid a file open per entry for it).
    #[test]
    fn scan_fast_lists_by_extension_without_reading_content() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        for index in 0..300 {
            std::fs::write(dir.path().join(format!("photo{index:03}.png")), b"definitely not image bytes").unwrap();
        }
        std::fs::write(dir.path().join("notes.txt"), b"sidecar").unwrap();

        let (entries, candidates, error) = scan_entries_fast(dir.path());
        assert!(error.is_none());
        // `..`, one subdirectory, and all 300 extension matches.
        assert_eq!(entries.len(), 302);
        assert_eq!(entries[0].name, "..");
        assert!(entries.iter().any(|entry| entry.name == "sub" && entry.is_dir));
        assert_eq!(entries.iter().filter(|entry| !entry.is_dir && !entry.is_parent).count(), 300);
        assert_eq!(candidates.len(), 1);
        assert!(entries.iter().all(|entry| entry.name != "notes.txt"));
    }

    /// Phase two still finds real images hiding under unusual names, and
    /// leaves actual non-images out.
    #[test]
    fn scan_sniff_finds_extensionless_images() {
        let dir = tempfile::tempdir().unwrap();
        let png = std::fs::read("tests/fixtures/photo.png").unwrap();
        std::fs::write(dir.path().join("no-extension"), &png).unwrap();
        std::fs::write(dir.path().join("not-an-image"), b"just text").unwrap();

        let (entries, candidates, error) = scan_entries_fast(dir.path());
        assert!(error.is_none());
        assert_eq!(candidates.len(), 2);
        let found = sniff_candidates(&candidates);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "no-extension");

        // Merging keeps the list sorted with the cursor file stable.
        let mut merged = entries;
        merged.extend(found);
        sort_entries(&mut merged);
        let names: Vec<&str> = merged.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, vec!["..", "no-extension"]);
    }

    /// Every panel keeps one space of padding inside its borders, and the
    /// Files/Metadata titles sit one cell right of the corner (`┌ Files ┐`).
    #[test]
    fn panels_pad_content_and_offset_titles() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.png");
        std::fs::copy("tests/fixtures/photo.png", &path).unwrap();
        let mut app = App::new(dir.path().to_path_buf(), None, false, false, false);
        app.pending_dir = None;
        app.entries = vec![FileEntry { name: "a.png".to_string(), path, is_dir: false, is_parent: false }];

        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| crate::tui_ui::render(&mut app, frame)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let symbol = |x: u16, y: u16| buffer[(x, y)].symbol().to_string();

        // Top path panel: `│ /tmp/...`, not `│/tmp/...`.
        assert_eq!(symbol(0, 1), "│");
        assert_eq!(symbol(1, 1), " ");
        // Bottom legend panel: `│ [Up/Down] ...`.
        assert_eq!(symbol(0, 22), "│");
        assert_eq!(symbol(1, 22), " ");
        assert_eq!(symbol(2, 22), "[");
        // Files panel title and first row (28 columns wide at 80x24).
        assert_eq!(symbol(0, 3), "┌");
        assert_eq!(symbol(1, 3), " ");
        assert_eq!(symbol(2, 3), "F");
        assert_eq!(symbol(0, 4), "│");
        assert_eq!(symbol(1, 4), " ");
        assert_eq!(symbol(2, 4), "•");
        // Metadata panel title and placeholder row.
        assert_eq!(symbol(28, 3), "┌");
        assert_eq!(symbol(29, 3), " ");
        assert_eq!(symbol(30, 3), "M");
        assert_eq!(symbol(28, 4), "│");
        assert_eq!(symbol(29, 4), " ");
        assert_eq!(symbol(30, 4), "O");
    }

    /// Symlinks resolve like the old `stat`-everything scan did.
    #[cfg(unix)]
    #[test]
    fn scan_fast_resolves_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let png = std::fs::read("tests/fixtures/photo.png").unwrap();
        std::fs::create_dir(dir.path().join("real")).unwrap();
        std::fs::write(dir.path().join("real.png"), &png).unwrap();
        std::os::unix::fs::symlink(dir.path().join("real"), dir.path().join("linkdir")).unwrap();
        std::os::unix::fs::symlink(dir.path().join("real.png"), dir.path().join("link.png")).unwrap();

        let (entries, candidates, error) = scan_entries_fast(dir.path());
        assert!(error.is_none());
        assert!(entries.iter().any(|entry| entry.name == "linkdir" && entry.is_dir));
        assert!(entries.iter().any(|entry| entry.name == "link.png" && !entry.is_dir));
        assert!(candidates.is_empty());
    }
}
