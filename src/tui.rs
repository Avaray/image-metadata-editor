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
}

enum Focus {
    FileList,
    Metadata,
}

enum AppState {
    Normal,
    Editing { tag: String, input: InputState },
    ConfirmExit,
}

struct App {
    current_dir: PathBuf,
    files: Vec<PathBuf>,
    file_state: ListState,
    
    current_metadata: Option<BTreeMap<String, String>>,
    meta_state: ListState,
    meta_keys: Vec<String>,

    pending_edits: BTreeMap<String, String>,
    
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
            pending_edits: BTreeMap::new(),
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
        self.meta_keys.clear();
        self.pending_edits.clear();
        self.meta_state.select(None);

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
                    self.meta_keys = flat.keys().cloned().collect();
                    self.current_metadata = Some(flat);
                    if !self.meta_keys.is_empty() {
                        self.meta_state.select(Some(0));
                    }
                }
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
                            KeyCode::Char('q') | KeyCode::Esc => {
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
                            KeyCode::Enter => {
                                match app.focus {
                                    Focus::FileList => {
                                        app.focus = Focus::Metadata;
                                    }
                                    Focus::Metadata => {
                                        // Edit metadata just like 'e'
                                        if let Some(idx) = app.meta_state.selected() {
                                            if let Some(tag) = app.meta_keys.get(idx).cloned() {
                                                let current_val = app.pending_edits.get(&tag)
                                                    .or_else(|| app.current_metadata.as_ref().and_then(|m| m.get(&tag)))
                                                    .cloned()
                                                    .unwrap_or_default();
                                                app.state = AppState::Editing {
                                                    tag,
                                                    input: InputState::new(current_val),
                                                };
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
                            KeyCode::Char('s') => {
                                if key.modifiers.contains(KeyModifiers::CONTROL) {
                                    let _ = app.save_pending_edits();
                                } else {
                                    // Strip
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
                                            let current_val = app.pending_edits.get(&tag)
                                                .or_else(|| app.current_metadata.as_ref().and_then(|m| m.get(&tag)))
                                                .cloned()
                                                .unwrap_or_default();
                                            app.state = AppState::Editing {
                                                tag,
                                                input: InputState::new(current_val),
                                            };
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    AppState::Editing { tag, input } => {
                        match key.code {
                            KeyCode::Enter => {
                                let val = input.value.clone();
                                let tag_clone = tag.clone();
                                app.pending_edits.insert(tag_clone, val);
                                app.state = AppState::Normal;
                            }
                            KeyCode::Esc => {
                                app.state = AppState::Normal;
                            }
                            KeyCode::Backspace => {
                                input.remove();
                            }
                            KeyCode::Left => {
                                input.move_cursor_left();
                            }
                            KeyCode::Right => {
                                input.move_cursor_right();
                            }
                            KeyCode::Char(c) => {
                                input.insert(c);
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
    let mut meta_block = Block::default().borders(Borders::ALL).title(if app.pending_edits.is_empty() {
        " Metadata "
    } else {
        " Metadata (UNSAVED EDITS) "
    });
    
    if matches!(app.focus, Focus::Metadata) {
        meta_block = meta_block.style(Style::default().fg(Color::Yellow));
    } else if !app.pending_edits.is_empty() {
        meta_block = meta_block.style(Style::default().fg(Color::Red));
    }

    let mut meta_items = Vec::new();
    if let Some(m) = &app.current_metadata {
        for key in &app.meta_keys {
            let val = app.pending_edits.get(key).or_else(|| m.get(key)).unwrap_or(&String::new()).clone();
            
            let color = if app.pending_edits.contains_key(key) {
                Color::Green
            } else {
                Color::Reset
            };
            
            let line = Line::from(vec![
                Span::styled(format!("{}: ", key), Style::default().fg(Color::Cyan)),
                Span::styled(val, Style::default().fg(color)),
            ]);
            meta_items.push(ListItem::new(line));
        }
    } else {
        meta_items.push(ListItem::new("No metadata or invalid file."));
    }

    let meta_list = List::new(meta_items)
        .block(meta_block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    f.render_stateful_widget(meta_list, top_chunks[1], &mut app.meta_state);

    // ── Bottom: Help & Instructions ──
    let help_text = match app.state {
        AppState::Normal => {
            if !app.pending_edits.is_empty() {
                " [Tab] Focus | [↑/↓] Move | [e/Enter] Edit | [s] Strip | [r] Refresh | [Ctrl+S] Save | [q] Quit "
            } else {
                " [Tab] Focus | [↑/↓] Move | [e/Enter] Edit | [s] Strip | [r] Refresh | [q] Quit "
            }
        },
        AppState::Editing { .. } => " [Enter] Save edit | [Esc] Cancel ",
        AppState::ConfirmExit => " You have unsaved edits! Save before exit? ",
    };

    let bottom_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(20)].as_ref())
        .split(chunks[1]);

    let p = Paragraph::new(help_text)
        .block(Block::default().borders(Borders::ALL))
        .style(match app.state {
            AppState::ConfirmExit => Style::default().fg(Color::Red),
            _ => Style::default(),
        });
    f.render_widget(p, bottom_layout[0]);

    let version_text = format!(" {} ", env!("CARGO_PKG_VERSION"));
    let version_p = Paragraph::new(Line::from(Span::styled(version_text, Style::default().fg(Color::DarkGray))))
        .block(Block::default().borders(Borders::ALL))
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
            
            // Build the text cursor visually using Spans
            let chars: Vec<char> = input.value.chars().collect();
            let mut before = String::new();
            let mut cursor_char = " ".to_string();
            let mut after = String::new();

            for (i, &c) in chars.iter().enumerate() {
                if i < input.cursor {
                    before.push(c);
                } else if i == input.cursor {
                    cursor_char = c.to_string();
                } else {
                    after.push(c);
                }
            }

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

// Helper to create a centered rectangle (for popups)
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
