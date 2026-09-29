use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, Paragraph};

use crate::tui::{App, Focus, Overlay};

pub fn render(app: &mut App, frame: &mut Frame) {
    let area = frame.area();
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)]).split(area);
    let panels = Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)]).split(rows[1]);

    render_top(app, frame, rows[0]);
    render_files(app, frame, panels[0]);
    render_meta(app, frame, panels[1]);
    render_bottom(app, frame, rows[2]);
    render_overlay(app, frame, area);
}

fn render_top(app: &App, frame: &mut Frame, area: Rect) {
    let mut text = if app.dir.as_os_str().is_empty() { "Drives".to_string() } else { app.dir.display().to_string() };
    if app.watcher.is_some() {
        text.push_str("  [watching]");
    }
    frame.render_widget(Paragraph::new(text).style(Style::default().add_modifier(Modifier::BOLD)), area);
}

fn render_bottom(app: &App, frame: &mut Frame, area: Rect) {
    let legend = if let Some(status) = &app.status { Line::styled(status.clone(), Style::default().fg(Color::Yellow)) } else { Line::raw(legend_text(app)) };
    frame.render_widget(Paragraph::new(legend), area);
}

fn legend_text(app: &App) -> &'static str {
    if let Some(overlay) = &app.overlay {
        return match overlay {
            Overlay::Search { .. } => "Enter jump to match · Esc cancel",
            Overlay::LeafEdit { .. } | Overlay::NewKey { .. } | Overlay::NewValue { .. } => "Enter confirm · Esc cancel",
            Overlay::SubtreeEdit { .. } => "Enter confirm · Alt+Enter newline · Esc cancel",
            Overlay::Confirm { .. } => "Left/Right select · Enter confirm · Esc cancel",
            Overlay::About => "Esc close",
        };
    }
    match app.focus {
        Focus::Files => "Up/Down move · Right open · Left up · Tab metadata · / search · c copy name · Ctrl+C copy path · r rescan · w wipe · F1 about · q quit",
        Focus::Meta => "Up/Down move · Right drill · Left back · Enter open/edit · e edit JSON · n new · d delete · c copy value · Ctrl+C copy path · Tab files · / search · w wipe · F1 about · q quit",
    }
}

fn panel_block(title: String, focused: bool) -> Block<'static> {
    let border = if focused { Style::default().fg(Color::Cyan) } else { Style::default().fg(Color::DarkGray) };
    Block::bordered().title(title).border_style(border)
}

fn follow(scroll: &mut usize, cursor: usize, visible: usize) {
    if visible == 0 {
        return;
    }
    if cursor < *scroll {
        *scroll = cursor;
    } else if cursor >= *scroll + visible {
        *scroll = cursor + 1 - visible;
    }
}

fn spinner(since: Instant) -> Option<char> {
    let elapsed = since.elapsed();
    if elapsed < Duration::from_secs(1) {
        return None;
    }
    Some(b"|/-\\"[(elapsed.as_millis() / 100 % 4) as usize] as char)
}

fn render_files(app: &mut App, frame: &mut Frame, area: Rect) {
    let focused = app.focus == Focus::Files && app.overlay.is_none();
    let block = panel_block("Files".to_string(), focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if let Some((_, since)) = &app.pending_dir {
        if let Some(frame_char) = spinner(*since) {
            frame.render_widget(Paragraph::new(format!("{frame_char} Loading...")), inner);
        }
        return;
    }
    if let Some(error) = &app.scan_error {
        frame.render_widget(Paragraph::new(error.clone()).style(Style::default().fg(Color::Red)), inner);
        return;
    }
    if app.entries.is_empty() {
        frame.render_widget(Paragraph::new("(empty directory)").style(Style::default().fg(Color::DarkGray)), inner);
        return;
    }

    follow(&mut app.file_scroll, app.file_cursor, inner.height as usize);
    let items: Vec<ListItem> = app
        .entries
        .iter()
        .enumerate()
        .skip(app.file_scroll)
        .take(inner.height as usize)
        .map(|(index, entry)| {
            let mut style = Style::default();
            if entry.is_parent {
                style = style.fg(Color::DarkGray);
            } else if entry.is_dir {
                style = style.fg(Color::Cyan);
            }
            if index == app.file_cursor {
                style = style.add_modifier(Modifier::REVERSED);
            }
            let text = if entry.is_parent {
                "..".to_string()
            } else if entry.is_dir {
                format!("▸ {}/", entry.name)
            } else {
                format!("• {}", entry.name)
            };
            ListItem::new(Line::styled(text, style))
        })
        .collect();
    frame.render_widget(List::new(items), inner);
}

fn render_meta(app: &mut App, frame: &mut Frame, area: Rect) {
    let focused = app.focus == Focus::Meta && app.overlay.is_none();
    let breadcrumb = app.tree.as_ref().map(|tree| tree.breadcrumb()).unwrap_or_else(|| "Metadata".to_string());
    let block = panel_block(left_truncate(&breadcrumb, area.width as usize), focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if app.preview_path.is_none() && app.preview_loading.is_none() {
        frame.render_widget(Paragraph::new("Open an image to load metadata.").style(Style::default().fg(Color::DarkGray)), inner);
        return;
    }
    if let Some((_, since)) = &app.preview_loading
        && let Some(frame_char) = spinner(*since)
    {
        frame.render_widget(Paragraph::new(format!("{frame_char} Loading...")), inner);
        return;
    }
    if let Some(error) = &app.preview_error {
        frame.render_widget(Paragraph::new(error.clone()).style(Style::default().fg(Color::Red)), inner);
        return;
    }
    let Some(tree) = &app.tree else {
        return;
    };
    let rows = tree.rows();
    if rows.is_empty() {
        frame.render_widget(Paragraph::new("(no metadata)").style(Style::default().fg(Color::DarkGray)), inner);
        return;
    }

    follow(&mut app.meta_scroll, tree.cursor(), inner.height as usize);
    let error_target = error_target_path(app);
    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .skip(app.meta_scroll)
        .take(inner.height as usize)
        .map(|(index, row)| {
            let selected = index == tree.cursor();
            let failed = error_target.as_ref().is_some_and(|target| {
                let mut path = tree.drill().to_vec();
                path.push(row.segment.clone());
                path == *target
            });
            let mut label = Style::default().add_modifier(Modifier::BOLD);
            if failed {
                label = label.fg(Color::Red);
            }
            if selected {
                label = label.add_modifier(Modifier::REVERSED);
            }
            let mut rest = Style::default().fg(Color::DarkGray);
            if selected {
                rest = rest.add_modifier(Modifier::REVERSED);
            }
            let separator = if row.is_branch { " ▸ " } else { ": " };
            ListItem::new(Line::from(vec![Span::styled(row.label.clone(), label), Span::styled(separator, rest), Span::styled(row.preview.clone(), rest)]))
        })
        .collect();
    frame.render_widget(List::new(items), inner);
}

/// The tree node an open editor failed to validate, for red highlighting.
fn error_target_path(app: &App) -> Option<Vec<crate::tui_tree::Segment>> {
    match &app.overlay {
        Some(Overlay::LeafEdit { path, error: Some(_), .. }) | Some(Overlay::SubtreeEdit { path, error: Some(_), .. }) => Some(path.clone()),
        _ => None,
    }
}

fn left_truncate(text: &str, width: usize) -> String {
    if width <= 1 || text.chars().count() <= width.saturating_sub(2) {
        return text.to_string();
    }
    let keep = width.saturating_sub(3);
    let tail: String = text.chars().rev().take(keep).collect::<String>().chars().rev().collect();
    format!("…{tail}")
}

// -- overlays ---------------------------------------------------------------

fn popup(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2)).max(1);
    let height = height.min(area.height.saturating_sub(2)).max(1);
    Rect::new(area.x + area.width.saturating_sub(width) / 2, area.y + area.height.saturating_sub(height) / 2, width, height)
}

fn render_overlay(app: &mut App, frame: &mut Frame, area: Rect) {
    match &app.overlay {
        None => {}
        Some(Overlay::Search { input, panel }) => {
            let matches = match panel {
                Focus::Files => {
                    let needle = input.value().to_lowercase();
                    app.entries.iter().filter(|entry| entry.name.to_lowercase().contains(&needle)).count()
                }
                Focus::Meta => app.tree.as_ref().map(|tree| tree.search(input.value()).len()).unwrap_or(0),
            };
            let title = match panel {
                Focus::Files => "Search files",
                Focus::Meta => "Search metadata",
            };
            let area = popup(area, 60, 6);
            frame.render_widget(ratatui::widgets::Clear, area);
            let block = Block::bordered().title(title);
            let inner = block.inner(area);
            frame.render_widget(block, area);
            render_input(frame, input, inner, 0);
            let count = if matches == 1 { "1 match".to_string() } else { format!("{matches} matches") };
            frame.render_widget(Paragraph::new(count).style(Style::default().fg(Color::DarkGray)), Rect::new(inner.x, inner.y + 2, inner.width, 1));
        }
        Some(Overlay::LeafEdit { input, error, .. }) => {
            overlay_editor(frame, area, "Edit value", input, error, &["Enter confirms · Esc cancels"]);
        }
        Some(Overlay::NewKey { input, error }) => {
            overlay_editor(frame, area, "New key name", input, error, &["Enter continues to the value prompt · Esc cancels"]);
        }
        Some(Overlay::NewValue { input, key, error }) => {
            let title = match key {
                Some(name) => format!("Value for '{name}'"),
                None => "Value to append".to_string(),
            };
            overlay_editor(frame, area, &title, input, error, &["Lenient JSON5 · Enter confirms · Esc cancels"]);
        }
        Some(Overlay::SubtreeEdit { lines, line, error, .. }) => {
            let area = popup(area, area.width * 70 / 100, area.height * 70 / 100);
            frame.render_widget(ratatui::widgets::Clear, area);
            let block = Block::bordered().title("Edit JSON");
            let inner = block.inner(area);
            frame.render_widget(block, area);
            if inner.width < 3 || inner.height < 3 {
                return;
            }
            let hint_lines = 2u16 + u16::from(error.is_some());
            let visible = (inner.height.saturating_sub(hint_lines)) as usize;
            let mut scroll = line.saturating_sub(visible.saturating_sub(1));
            if *line < scroll {
                scroll = *line;
            }
            for (row, input) in lines.iter().enumerate().skip(scroll).take(visible) {
                let y = inner.y + (row - scroll) as u16;
                let width = inner.width.saturating_sub(1) as usize;
                let view_scroll = input.visual_scroll(width);
                let mut style = Style::default();
                if row == *line {
                    style = style.bg(Color::DarkGray);
                }
                frame.render_widget(Paragraph::new(input.value()).style(style).scroll((0, view_scroll as u16)), Rect::new(inner.x, y, inner.width, 1));
            }
            let active = &lines[*line];
            let width = inner.width.saturating_sub(1) as usize;
            let view_scroll = active.visual_scroll(width);
            frame.set_cursor_position(((inner.x + (active.visual_cursor().max(view_scroll) - view_scroll) as u16), inner.y + (*line - scroll) as u16));
            let mut hints = vec![Line::styled("Enter confirms · Alt+Enter newline · Esc cancels", Style::default().fg(Color::DarkGray))];
            if let Some(error) = error {
                hints.push(Line::styled(error.clone(), Style::default().fg(Color::Red)));
            }
            frame.render_widget(Paragraph::new(hints), Rect::new(inner.x, inner.y + visible as u16, inner.width, hint_lines));
        }
        Some(Overlay::Confirm { message, yes, .. }) => {
            let area = popup(area, 56, 7);
            frame.render_widget(ratatui::widgets::Clear, area);
            let block = Block::bordered().title("Confirm");
            let inner = block.inner(area);
            frame.render_widget(block, area);
            frame.render_widget(Paragraph::new(message.clone()), Rect::new(inner.x, inner.y, inner.width, 2));
            let yes_style = if *yes { Style::default().add_modifier(Modifier::REVERSED) } else { Style::default() };
            let no_style = if *yes { Style::default() } else { Style::default().add_modifier(Modifier::REVERSED) };
            frame.render_widget(Paragraph::new(Line::from(vec![Span::styled(" Yes ", yes_style), Span::raw("  "), Span::styled(" No ", no_style)])), Rect::new(inner.x, inner.y + 3, inner.width, 1));
        }
        Some(Overlay::About) => {
            let area = popup(area, 64, 11);
            frame.render_widget(ratatui::widgets::Clear, area);
            let block = Block::bordered().title("About");
            let inner = block.inner(area);
            frame.render_widget(block, area);
            let lines = vec![
                Line::from(vec![Span::styled(format!("ime {}", env!("CARGO_PKG_VERSION")), Style::default().add_modifier(Modifier::BOLD))]),
                Line::raw(format!("Author: {}", env!("CARGO_PKG_AUTHORS"))),
                Line::raw(format!("License: {}", env!("CARGO_PKG_LICENSE"))),
                Line::raw(format!("Repository: {}", env!("CARGO_PKG_REPOSITORY"))),
                Line::raw(format!("Rust: {}", env!("IME_RUSTC_VERSION"))),
                Line::raw(format!("ratatui: {}", env!("IME_RATATUI_VERSION"))),
            ];
            frame.render_widget(Paragraph::new(lines), inner);
        }
    }
}

fn overlay_editor(frame: &mut Frame, area: Rect, title: &str, input: &tui_input::Input, error: &Option<String>, hints: &[&str]) {
    let height = 4 + hints.len() as u16 + u16::from(error.is_some());
    let area = popup(area, 60, height);
    frame.render_widget(ratatui::widgets::Clear, area);
    let block = Block::bordered().title(title.to_string());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    render_input(frame, input, inner, 0);
    let mut lines: Vec<Line> = hints.iter().map(|hint| Line::styled(hint.to_string(), Style::default().fg(Color::DarkGray))).collect();
    if let Some(error) = error {
        lines.push(Line::styled(error.clone(), Style::default().fg(Color::Red)));
    }
    let height = lines.len() as u16;
    frame.render_widget(Paragraph::new(lines), Rect::new(inner.x, inner.y + 2, inner.width, height));
}

fn render_input(frame: &mut Frame, input: &tui_input::Input, inner: Rect, row: u16) {
    if inner.width < 2 {
        return;
    }
    let width = inner.width.saturating_sub(1) as usize;
    let scroll = input.visual_scroll(width);
    frame.render_widget(Paragraph::new(input.value()).scroll((0, scroll as u16)), Rect::new(inner.x, inner.y + row, inner.width, 1));
    frame.set_cursor_position(((inner.x + (input.visual_cursor().max(scroll) - scroll) as u16), inner.y + row));
}
