use std::{
    collections::{BTreeMap, HashMap},
    f64::consts::{E, PI},
    io::{self, Stdout},
    time::Duration,
};

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
        MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

fn main() -> io::Result<()> {
    let mut terminal = TerminalGuard::new()?;
    let mut app = App::default();
    let result = run_app(terminal.terminal_mut(), &mut app);
    terminal.restore()?;
    result
}

struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    restored: bool,
}

impl TerminalGuard {
    fn new() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self {
            terminal,
            restored: false,
        })
    }

    fn terminal_mut(&mut self) -> &mut Terminal<CrosstermBackend<Stdout>> {
        &mut self.terminal
    }

    fn restore(&mut self) -> io::Result<()> {
        if !self.restored {
            disable_raw_mode()?;
            execute!(
                self.terminal.backend_mut(),
                LeaveAlternateScreen,
                DisableMouseCapture
            )?;
            self.terminal.show_cursor()?;
            self.restored = true;
        }
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

#[derive(Clone, Copy, Debug)]
enum AngleMode {
    Radians,
    Degrees,
}

impl AngleMode {
    fn label(self) -> &'static str {
        match self {
            Self::Radians => "RAD",
            Self::Degrees => "DEG",
        }
    }

    fn toggle(&mut self) {
        *self = match self {
            Self::Radians => Self::Degrees,
            Self::Degrees => Self::Radians,
        };
    }

    fn to_radians(self, value: f64) -> f64 {
        match self {
            Self::Radians => value,
            Self::Degrees => value.to_radians(),
        }
    }
}

struct App {
    input: String,
    cursor: usize,
    ans: f64,
    angle_mode: AngleMode,
    status: String,
    variables: BTreeMap<String, f64>,
    history: Vec<HistoryEntry>,
    history_nav: Option<usize>,
    history_draft: Option<String>,
    show_help: bool,
    should_quit: bool,
    hovered_node: Option<usize>,
    last_render: Option<RenderSnapshot>,
}

#[derive(Clone, Debug)]
struct HistoryEntry {
    input: String,
    math: MathBlock,
    answer: String,
}

impl Default for App {
    fn default() -> Self {
        Self {
            input: String::new(),
            cursor: 0,
            ans: 0.0,
            angle_mode: AngleMode::Degrees,
            status: "".to_owned(),
            variables: BTreeMap::new(),
            history: Vec::new(),
            history_nav: None,
            history_draft: None,
            show_help: false,
            should_quit: false,
            hovered_node: None,
            last_render: None,
        }
    }
}

impl App {
    fn begin_edit(&mut self) {
        self.hovered_node = None;
        self.history_nav = None;
        self.history_draft = None;
    }

    fn insert_char(&mut self, ch: char) {
        match ch {
            '(' | '[' => self.insert_open_delimiter(ch),
            ')' | ']' => self.insert_close_delimiter(ch),
            _ => {
                self.begin_edit();
                let byte_index = char_to_byte_index(&self.input, self.cursor);
                self.input.insert(byte_index, ch);
                self.cursor += 1;
            }
        }
    }

    fn insert_str(&mut self, text: &str) {
        self.begin_edit();
        let byte_index = char_to_byte_index(&self.input, self.cursor);
        self.input.insert_str(byte_index, text);
        self.cursor += text.chars().count();
    }

    fn move_cursor_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn move_cursor_right(&mut self) {
        let max = self.input.chars().count();
        self.cursor = (self.cursor + 1).min(max);
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }

        if let (Some(open), Some(close)) = (self.char_before_cursor(), self.char_at_cursor()) {
            if matching_close(open) == Some(close) {
                let start = char_to_byte_index(&self.input, self.cursor - 1);
                let end = char_to_byte_index(&self.input, self.cursor + 1);
                self.input.replace_range(start..end, "");
                self.cursor -= 1;
                self.begin_edit();
                return;
            }
        }

        self.begin_edit();
        let start = char_to_byte_index(&self.input, self.cursor - 1);
        let end = char_to_byte_index(&self.input, self.cursor);
        self.input.replace_range(start..end, "");
        self.cursor -= 1;
    }

    fn delete(&mut self) {
        let max = self.input.chars().count();
        if self.cursor >= max {
            return;
        }
        self.begin_edit();
        let start = char_to_byte_index(&self.input, self.cursor);
        let end = char_to_byte_index(&self.input, self.cursor + 1);
        self.input.replace_range(start..end, "");
    }

    fn clear_input(&mut self) {
        self.input.clear();
        self.cursor = 0;
        self.hovered_node = None;
    }

    fn insert_open_delimiter(&mut self, open: char) {
        self.begin_edit();
        let close = matching_close(open).expect("opening delimiter must have a closer");
        let byte_index = char_to_byte_index(&self.input, self.cursor);

        if let Some(span_end) = expression_end_from(&self.input, self.cursor) {
            self.input.insert(byte_index, open);
            let close_index = char_to_byte_index(&self.input, self.cursor + span_end + 1);
            self.input.insert(close_index, close);
            self.cursor += 1;
        } else {
            self.input.insert(byte_index, open);
            self.input
                .insert(char_to_byte_index(&self.input, self.cursor + 1), close);
            self.cursor += 1;
        }
    }

    fn insert_close_delimiter(&mut self, close: char) {
        self.begin_edit();
        if self.char_at_cursor() == Some(close) {
            self.cursor += 1;
        } else if let Some(span_start) = expression_start_before(&self.input, self.cursor) {
            let open = matching_open(close).expect("closing delimiter must have an opener");
            let open_index = char_to_byte_index(&self.input, span_start);
            self.input.insert(open_index, open);
            let close_index = char_to_byte_index(&self.input, self.cursor + 1);
            self.input.insert(close_index, close);
            self.cursor += 2;
        } else {
            let byte_index = char_to_byte_index(&self.input, self.cursor);
            self.input.insert(byte_index, close);
            self.cursor += 1;
        }
    }

    fn char_at_cursor(&self) -> Option<char> {
        self.input.chars().nth(self.cursor)
    }

    fn char_before_cursor(&self) -> Option<char> {
        self.cursor
            .checked_sub(1)
            .and_then(|index| self.input.chars().nth(index))
    }

    fn submit(&mut self) {
        let trimmed = self.input.trim();
        if trimmed.is_empty() {
            self.status.clear();
            return;
        }

        if trimmed.eq_ignore_ascii_case("rad") {
            self.angle_mode = AngleMode::Radians;
            self.status = "Angle mode set to RAD.".to_owned();
            self.clear_input();
            return;
        }

        if trimmed.eq_ignore_ascii_case("deg") {
            self.angle_mode = AngleMode::Degrees;
            self.status = "Angle mode set to DEG.".to_owned();
            self.clear_input();
            return;
        }

        match parse_assignment_input(trimmed) {
            Ok(Some((name, expression))) => {
                match evaluate_with_variables(
                    &expression,
                    self.ans,
                    self.angle_mode,
                    &self.variables,
                ) {
                    Ok(value) => {
                        self.variables.insert(name.clone(), value);
                        self.ans = value;
                        self.history.push(HistoryEntry {
                            input: trimmed.to_owned(),
                            math: render_statement_math(trimmed),
                            answer: format!("= {}", format_number(value)),
                        });
                        self.status = format!("{name} = {}", format_number(value));
                        self.clear_input();
                    }
                    Err(err) => self.status = format!("Error: {err}"),
                }
            }
            Ok(None) => {
                match evaluate_with_variables(trimmed, self.ans, self.angle_mode, &self.variables) {
                    Ok(value) => {
                        self.ans = value;
                        self.history.push(HistoryEntry {
                            input: trimmed.to_owned(),
                            math: render_statement_math(trimmed),
                            answer: format!("= {}", format_number(value)),
                        });
                        self.status = format!("= {}", format_number(value));
                        self.clear_input();
                    }
                    Err(err) => {
                        self.status = format!("Error: {err}");
                    }
                }
            }
            Err(err) => self.status = format!("Error: {err}"),
        }
    }

    fn navigate_history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }

        let next_index = match self.history_nav {
            Some(index) if index > 0 => index - 1,
            Some(index) => index,
            None => {
                self.history_draft = Some(self.input.clone());
                self.history.len() - 1
            }
        };
        self.history_nav = Some(next_index);
        self.load_history_entry(next_index);
    }

    fn navigate_history_down(&mut self) {
        let Some(index) = self.history_nav else {
            return;
        };

        if index + 1 < self.history.len() {
            let next_index = index + 1;
            self.history_nav = Some(next_index);
            self.load_history_entry(next_index);
        } else {
            let draft = self.history_draft.take().unwrap_or_default();
            self.history_nav = None;
            self.input = draft;
            self.cursor = self.input.chars().count();
            self.hovered_node = None;
        }
    }

    fn load_history_entry(&mut self, index: usize) {
        self.input = self.history[index].input.clone();
        self.cursor = self.input.chars().count();
        self.hovered_node = None;
    }
}

#[derive(Clone, Debug)]
struct RenderSnapshot {
    input_start_x: u16,
    input_y: u16,
    input_len: usize,
    node_spans: Vec<NodeSpan>,
    node_rects: Vec<AbsoluteNodeRect>,
}

#[derive(Clone, Copy, Debug)]
struct NodeSpan {
    node_id: usize,
    span: SourceSpan,
}

#[derive(Clone, Copy, Debug)]
struct AbsoluteNodeRect {
    node_id: usize,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> io::Result<()> {
    loop {
        let mut snapshot = None;
        terminal.draw(|frame| {
            snapshot = Some(render(frame, app));
        })?;
        app.last_render = snapshot;

        if !event::poll(Duration::from_millis(50))? {
            continue;
        }

        match event::read()? {
            Event::Key(key) => {
                handle_key(app, key);
                if app.should_quit {
                    return Ok(());
                }
            }
            Event::Paste(text) => app.insert_str(&text),
            Event::Mouse(mouse) => handle_mouse(app, mouse),
            Event::Resize(_, _) => {}
            _ => {}
        }
    }
}

fn should_quit(key: KeyEvent) -> bool {
    matches!(
        (key.code, key.modifiers),
        (KeyCode::Char('c'), KeyModifiers::CONTROL) | (KeyCode::Char('d'), KeyModifiers::CONTROL)
    )
}

fn handle_key(app: &mut App, key: KeyEvent) {
    if should_quit(key) {
        app.should_quit = true;
        return;
    }

    if app.show_help {
        match key.code {
            KeyCode::Esc | KeyCode::F(1) => app.show_help = false,
            _ => {}
        }
        return;
    }

    match key.code {
        KeyCode::Esc => app.should_quit = true,
        KeyCode::Enter => app.submit(),
        KeyCode::Backspace => app.backspace(),
        KeyCode::Delete => app.delete(),
        KeyCode::Left => app.move_cursor_left(),
        KeyCode::Right => app.move_cursor_right(),
        KeyCode::Up => app.navigate_history_up(),
        KeyCode::Down => app.navigate_history_down(),
        KeyCode::Home => app.cursor = 0,
        KeyCode::End => app.cursor = app.input.chars().count(),
        KeyCode::F(1) => app.show_help = true,
        KeyCode::F(2) => {
            app.angle_mode.toggle();
            app.status = format!("Angle mode set to {}.", app.angle_mode.label());
        }
        KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.clear_input();
            app.status = "Cleared input.".to_owned();
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.clear_input();
        }
        KeyCode::Char(ch)
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT) =>
        {
            app.insert_char(ch);
            app.hovered_node = None;
        }
        _ => {}
    }
}

fn handle_mouse(app: &mut App, mouse: MouseEvent) {
    match mouse.kind {
        MouseEventKind::Moved
        | MouseEventKind::Down(_)
        | MouseEventKind::Drag(_)
        | MouseEventKind::ScrollUp
        | MouseEventKind::ScrollDown
        | MouseEventKind::ScrollLeft
        | MouseEventKind::ScrollRight => {}
        _ => return,
    }

    let Some(snapshot) = &app.last_render else {
        app.hovered_node = None;
        return;
    };

    let source_hit = pick_hovered_from_input(snapshot, mouse.column, mouse.row);
    let render_hit = pick_hovered_from_render(snapshot, mouse.column, mouse.row);
    app.hovered_node = source_hit.or(render_hit);
}

fn pick_hovered_from_input(snapshot: &RenderSnapshot, x: u16, y: u16) -> Option<usize> {
    if y != snapshot.input_y || x < snapshot.input_start_x {
        return None;
    }
    let char_index = usize::from(x - snapshot.input_start_x);
    if char_index >= snapshot.input_len {
        return None;
    }

    snapshot
        .node_spans
        .iter()
        .filter(|node| node.span.contains(char_index))
        .min_by_key(|node| node.span.len())
        .map(|node| node.node_id)
}

fn pick_hovered_from_render(snapshot: &RenderSnapshot, x: u16, y: u16) -> Option<usize> {
    snapshot
        .node_rects
        .iter()
        .filter(|node| {
            x >= node.x && x < node.x + node.width && y >= node.y && y < node.y + node.height
        })
        .min_by_key(|node| u32::from(node.width) * u32::from(node.height))
        .map(|node| node.node_id)
}

fn pick_hovered_from_cursor(spans: &[NodeSpan], cursor: usize, input_len: usize) -> Option<usize> {
    if input_len == 0 {
        return None;
    }

    let primary = cursor.min(input_len.saturating_sub(1));
    let secondary = cursor.saturating_sub(1);

    [primary, secondary].into_iter().find_map(|index| {
        spans
            .iter()
            .filter(|node| node.span.contains(index))
            .min_by_key(|node| node.span.len())
            .map(|node| node.node_id)
    })
}

fn render(frame: &mut Frame, app: &App) -> RenderSnapshot {
    let parsed_assignment = if app.input.trim().is_empty() {
        Ok(None)
    } else {
        parse_assignment_input(&app.input)
    };

    let (math, node_spans) = match parsed_assignment.as_ref() {
        Ok(Some((name, expression))) => {
            if let Ok(expr) = parse_expression_input(expression) {
                let prefix = format!("{name} = ");
                let offset = assignment_expression_offset(&app.input).unwrap_or(0);
                let mut spans = Vec::new();
                collect_node_spans(&expr, &mut spans);
                for span in &mut spans {
                    span.span.start += offset;
                    span.span.end += offset;
                }
                (
                    join_blocks(&[MathBlock::from_text(prefix), render_expression(&expr)]),
                    spans,
                )
            } else {
                (render_statement_math(&app.input), Vec::new())
            }
        }
        Ok(None) => match parse_expression_input(&app.input) {
            Ok(expr) => {
                let mut spans = Vec::new();
                collect_node_spans(&expr, &mut spans);
                (render_expression(&expr), spans)
            }
            Err(_) => (render_input_math(&app.input), Vec::new()),
        },
        Err(_) => (render_statement_math(&app.input), Vec::new()),
    };

    let preview = if app.input.trim().is_empty() {
        format!("= {}", format_number(app.ans))
    } else {
        match preview_statement(&app.input, app.ans, app.angle_mode, &app.variables) {
            Ok(value) => format!("= {}", format_number(value)),
            Err(err) => format!("preview: {err}"),
        }
    };

    let active_hover = pick_hovered_from_cursor(&node_spans, app.cursor, app.input.chars().count())
        .or(app.hovered_node);

    let math_height = math.height().max(1) as u16;
    let current_height = math_height + 5;
    let history_entries = visible_history_entries(&app.history, 5);
    let history_height = history_block_height(history_entries);
    let variable_height = variable_block_height(&app.variables);
    let column = centered_column(
        frame.area(),
        history_height + 1 + current_height + 1 + variable_height,
    );

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(history_height),
            Constraint::Length(1),
            Constraint::Length(current_height),
            Constraint::Length(1),
            Constraint::Length(variable_height),
        ])
        .split(column);

    let history_area = sections[0];
    let current_area = sections[2];
    let variables_area = sections[4];

    render_history_block(frame, history_area, history_entries);
    render_variables_block(frame, variables_area, &app.variables);

    let current_bg = current_panel_bg();
    let inner = render_shaded_panel(frame, current_area, "TaoUI", current_bg);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(math_height),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);

    let header = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(7), Constraint::Fill(1)])
        .split(rows[0]);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(" {} ", app.angle_mode.label()),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))),
        header[0],
    );

    frame.render_widget(
        Paragraph::new(preview)
            .alignment(Alignment::Right)
            .style(panel_text_style(current_bg).fg(Color::Green)),
        header[1],
    );

    frame.render_widget(
        Paragraph::new(styled_math_lines(&math, active_hover)).style(panel_text_style(current_bg)),
        rows[1],
    );

    frame.render_widget(
        Paragraph::new(build_input_line(
            app.input.as_str(),
            active_hover.and_then(|node_id| {
                node_spans
                    .iter()
                    .find(|span| span.node_id == node_id)
                    .map(|span| span.span)
            }),
        ))
        .style(panel_text_style(current_bg)),
        rows[2],
    );

    frame.render_widget(
        Paragraph::new(app.status.as_str()).style(panel_text_style(current_bg).fg(Color::Gray)),
        rows[3],
    );

    let cursor_x = rows[2].x + 2 + app.cursor as u16;
    let cursor_y = rows[2].y;
    let max_x = rows[2].x + rows[2].width.saturating_sub(1);
    frame.set_cursor_position((cursor_x.min(max_x), cursor_y));

    let node_rects = math
        .rects
        .iter()
        .map(|rect| AbsoluteNodeRect {
            node_id: rect.node_id,
            x: rows[1].x + rect.x as u16,
            y: rows[1].y + rect.y as u16,
            width: rect.width as u16,
            height: rect.height as u16,
        })
        .collect();

    if app.show_help {
        render_help_popup(frame, frame.area());
    }

    RenderSnapshot {
        input_start_x: rows[2].x + 2,
        input_y: rows[2].y,
        input_len: app.input.chars().count(),
        node_spans,
        node_rects,
    }
}

fn centered_column(area: Rect, desired_height: u16) -> Rect {
    let max_height = area.height.saturating_sub(2).max(7);
    let height = desired_height.min(max_height).max(7);
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(height),
            Constraint::Fill(1),
        ])
        .split(area);

    let width = area.width.saturating_sub(4).clamp(60, 120);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(width),
            Constraint::Fill(1),
        ])
        .split(vertical[1]);

    horizontal[1]
}

fn render_shaded_panel(frame: &mut Frame, area: Rect, title: &str, bg: Color) -> Rect {
    frame.render_widget(Paragraph::new("").style(Style::default().bg(bg)), area);

    if area.width > 4 && area.height > 0 {
        let title_area = Rect {
            x: area.x + 2,
            y: area.y,
            width: area.width.saturating_sub(4),
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                title.to_owned(),
                panel_text_style(bg)
                    .fg(Color::Rgb(182, 190, 197))
                    .add_modifier(Modifier::BOLD),
            ))),
            title_area,
        );
    }

    Rect {
        x: area.x + 2,
        y: area.y + 1,
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(2),
    }
}

fn panel_text_style(bg: Color) -> Style {
    Style::default().fg(Color::White).bg(bg)
}

fn current_panel_bg() -> Color {
    Color::Rgb(28, 30, 34)
}

fn history_panel_bg() -> Color {
    Color::Rgb(23, 25, 29)
}

fn variables_panel_bg() -> Color {
    Color::Rgb(23, 25, 29)
}

fn help_panel_bg() -> Color {
    Color::Rgb(18, 20, 24)
}

fn history_block_height(entries: &[HistoryEntry]) -> u16 {
    let content_height = if entries.is_empty() {
        1
    } else {
        entries
            .iter()
            .map(|entry| entry.math.height().max(1) as u16)
            .sum::<u16>()
    };
    content_height + 2
}

fn variable_block_height(variables: &BTreeMap<String, f64>) -> u16 {
    let content_height = variables.len().max(1).min(5) as u16;
    content_height + 2
}

fn visible_history_entries(history: &[HistoryEntry], max_entries: usize) -> &[HistoryEntry] {
    let start = history.len().saturating_sub(max_entries);
    &history[start..]
}

fn render_history_block(frame: &mut Frame, area: Rect, entries: &[HistoryEntry]) {
    let bg = history_panel_bg();
    let inner = render_shaded_panel(frame, area, "History", bg);
    let width = inner.width.saturating_sub(1) as usize;
    frame.render_widget(
        Paragraph::new(build_history_lines(entries, width)).style(panel_text_style(bg)),
        inner,
    );
}

fn build_history_lines(entries: &[HistoryEntry], width: usize) -> Vec<Line<'static>> {
    if entries.is_empty() {
        return vec![Line::from(Span::styled(
            "No history yet",
            Style::default().fg(Color::DarkGray),
        ))];
    }

    let mut lines = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        lines.extend(history_entry_lines(entry, width, index));
    }
    lines
}

fn history_entry_lines(entry: &HistoryEntry, width: usize, index: usize) -> Vec<Line<'static>> {
    let answer_width = entry.answer.chars().count();
    let bg = history_entry_bg(index);
    let mut lines = Vec::new();

    for (row, math_line) in entry.math.lines.iter().enumerate() {
        let expr = math_line.trim_end().to_owned();
        let expr_width = expr.chars().count();
        let content_width = width.saturating_sub(2);
        let mut spans = vec![Span::styled(" ".to_owned(), Style::default().bg(bg))];

        if row == entry.math.baseline && content_width > expr_width + answer_width {
            let gap = content_width - expr_width - answer_width;
            spans.push(Span::styled(expr, Style::default().fg(Color::White).bg(bg)));
            spans.push(Span::styled(" ".repeat(gap), Style::default().bg(bg)));
            spans.push(Span::styled(
                entry.answer.clone(),
                Style::default().fg(Color::Green).bg(bg),
            ));
        } else {
            let padding = content_width.saturating_sub(expr_width);
            spans.push(Span::styled(expr, Style::default().fg(Color::White).bg(bg)));
            spans.push(Span::styled(" ".repeat(padding), Style::default().bg(bg)));
        }

        spans.push(Span::styled(" ".to_owned(), Style::default().bg(bg)));
        lines.push(Line::from(spans));
    }

    lines
}

fn history_entry_bg(index: usize) -> Color {
    match index % 2 {
        0 => Color::Rgb(30, 33, 38),
        _ => Color::Rgb(35, 38, 44),
    }
}

fn render_variables_block(frame: &mut Frame, area: Rect, variables: &BTreeMap<String, f64>) {
    let bg = variables_panel_bg();
    let inner = render_shaded_panel(frame, area, "Variables", bg);
    frame.render_widget(
        Paragraph::new(build_variable_lines(variables)).style(panel_text_style(bg)),
        inner,
    );
}

fn build_variable_lines(variables: &BTreeMap<String, f64>) -> Vec<Line<'static>> {
    if variables.is_empty() {
        return vec![Line::from(Span::styled(
            "No variables",
            Style::default().fg(Color::DarkGray),
        ))];
    }

    variables
        .iter()
        .map(|(name, value)| {
            Line::from(vec![
                Span::styled(name.clone(), Style::default().fg(Color::Cyan)),
                Span::raw(" = "),
                Span::styled(format_number(*value), Style::default().fg(Color::Green)),
            ])
        })
        .collect()
}

fn render_help_popup(frame: &mut Frame, area: Rect) {
    let popup = centered_popup(area, 64, 13);
    frame.render_widget(Clear, popup);
    let bg = help_panel_bg();
    let inner = render_shaded_panel(frame, popup, "Help", bg);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from("F1: open/close help"),
            Line::from("Enter: evaluate"),
            Line::from("Up/Down: recall history"),
            Line::from("F2: toggle RAD/DEG"),
            Line::from("Ctrl+L: clear input"),
            Line::from("Esc: quit"),
            Line::from(""),
            Line::from("Assignments: name = expression"),
            Line::from("Variables: multi-letter, CaseSensitive, underscore_ok"),
            Line::from("Implicit multiply: 2x, 2(3+4), 3sin(3)"),
            Line::from("Functions: sin cos tan sqrt root(n,x) abs frac pow"),
        ])
        .style(panel_text_style(bg)),
        inner,
    );
}

fn centered_popup(area: Rect, width: u16, height: u16) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(height.min(area.height.saturating_sub(2))),
            Constraint::Fill(1),
        ])
        .split(area);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(width.min(area.width.saturating_sub(2))),
            Constraint::Fill(1),
        ])
        .split(vertical[1]);

    horizontal[1]
}

fn char_to_byte_index(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(index, _)| index)
}

fn matching_close(open: char) -> Option<char> {
    match open {
        '(' => Some(')'),
        '[' => Some(']'),
        _ => None,
    }
}

fn matching_open(close: char) -> Option<char> {
    match close {
        ')' => Some('('),
        ']' => Some('['),
        _ => None,
    }
}

fn expression_end_from(input: &str, cursor: usize) -> Option<usize> {
    let suffix: String = input.chars().skip(cursor).collect();
    if suffix.is_empty() {
        return None;
    }

    let tokens = tokenize(&suffix).ok()?;
    if tokens.is_empty() || !token_starts_expression(&tokens[0]) {
        return None;
    }

    let mut parser = Parser::new(tokens);
    let expr = parser.parse_expression().ok()?;
    Some(expr.span.end)
}

fn expression_start_before(input: &str, cursor: usize) -> Option<usize> {
    if cursor == 0 {
        return None;
    }

    let chars: Vec<char> = input.chars().collect();
    for start in 0..cursor {
        let candidate: String = chars[start..cursor].iter().collect();
        if candidate.trim().is_empty() {
            continue;
        }

        if parse_expression_input(&candidate).is_ok() {
            return Some(start);
        }
    }

    None
}

fn token_starts_expression(token: &Token) -> bool {
    matches!(
        token.kind,
        TokenKindValue::Number { .. }
            | TokenKindValue::Ident(_)
            | TokenKindValue::LParen
            | TokenKindValue::Minus
            | TokenKindValue::Plus
            | TokenKindValue::Pipe
    )
}

fn token_starts_implicit_product(token: &Token) -> bool {
    matches!(
        token.kind,
        TokenKindValue::Number { .. } | TokenKindValue::Ident(_) | TokenKindValue::LParen
    )
}

fn supported_function_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "sin" | "cos" | "tan" | "sqrt" | "abs" | "sq" | "pow" | "frac" | "root" | "nroot"
    )
}

#[derive(Clone, Debug)]
struct MathBlock {
    lines: Vec<String>,
    width: usize,
    baseline: usize,
    rects: Vec<NodeRect>,
}

#[derive(Clone, Copy, Debug)]
struct NodeRect {
    node_id: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

impl MathBlock {
    fn from_text(text: impl Into<String>) -> Self {
        Self::from_lines(vec![text.into()], 0)
    }

    fn from_lines(lines: Vec<String>, baseline: usize) -> Self {
        Self::with_rects(lines, baseline, Vec::new())
    }

    fn with_rects(lines: Vec<String>, baseline: usize, rects: Vec<NodeRect>) -> Self {
        let width = lines
            .iter()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0);
        let normalized = lines
            .into_iter()
            .map(|line| pad_to_width(line, width))
            .collect();
        Self {
            lines: normalized,
            width,
            baseline,
            rects,
        }
    }

    fn height(&self) -> usize {
        self.lines.len()
    }
}

fn pad_to_width(mut text: String, width: usize) -> String {
    let current = text.chars().count();
    if current < width {
        text.push_str(&" ".repeat(width - current));
    }
    text
}

fn centered_line(text: &str, width: usize) -> String {
    let content_width = text.chars().count();
    if content_width >= width {
        return text.to_owned();
    }

    let left = (width - content_width) / 2;
    let right = width - content_width - left;
    format!("{}{}{}", " ".repeat(left), text, " ".repeat(right))
}

fn join_blocks(parts: &[MathBlock]) -> MathBlock {
    let baseline = parts.iter().map(|block| block.baseline).max().unwrap_or(0);
    let below = parts
        .iter()
        .map(|block| block.height().saturating_sub(block.baseline + 1))
        .max()
        .unwrap_or(0);
    let height = baseline + below + 1;
    let mut lines = vec![String::new(); height];
    let mut rects = Vec::new();
    let mut x_offset = 0;

    for block in parts {
        let top = baseline.saturating_sub(block.baseline);
        for (row, line) in lines.iter_mut().enumerate() {
            if row >= top && row < top + block.height() {
                line.push_str(&block.lines[row - top]);
            } else {
                line.push_str(&" ".repeat(block.width));
            }
        }
        rects.extend(block.rects.iter().map(|rect| NodeRect {
            node_id: rect.node_id,
            x: rect.x + x_offset,
            y: rect.y + top,
            width: rect.width,
            height: rect.height,
        }));
        x_offset += block.width;
    }

    MathBlock::with_rects(lines, baseline, rects)
}

fn delimiter_block(
    height: usize,
    baseline: usize,
    top: char,
    middle: char,
    bottom: char,
) -> MathBlock {
    if height <= 1 {
        return MathBlock::from_lines(
            vec![middle.to_string()],
            baseline.min(height.saturating_sub(1)),
        );
    }

    let mut lines = Vec::with_capacity(height);
    for row in 0..height {
        let ch = if row == 0 {
            top
        } else if row + 1 == height {
            bottom
        } else {
            middle
        };
        lines.push(ch.to_string());
    }
    MathBlock::from_lines(lines, baseline)
}

fn wrap_parentheses(block: MathBlock) -> MathBlock {
    if block.height() <= 1 {
        return MathBlock::from_text(format!("({})", block.lines[0].trim_end()));
    }

    let height = block.height();
    let baseline = block.baseline;
    join_blocks(&[
        delimiter_block(height, baseline, '⎛', '⎜', '⎝'),
        block,
        delimiter_block(height, baseline, '⎞', '⎟', '⎠'),
    ])
}

fn wrap_absolute(block: MathBlock) -> MathBlock {
    if block.height() <= 1 {
        return MathBlock::from_text(format!("|{}|", block.lines[0].trim_end()));
    }

    let height = block.height();
    let baseline = block.baseline;
    join_blocks(&[
        delimiter_block(height, baseline, '│', '│', '│'),
        block,
        delimiter_block(height, baseline, '│', '│', '│'),
    ])
}

fn render_fraction_block(numerator: MathBlock, denominator: MathBlock) -> MathBlock {
    let width = numerator.width.max(denominator.width).max(1) + 2;
    let mut lines = Vec::with_capacity(numerator.height() + denominator.height() + 1);
    let numerator_x = (width - numerator.width) / 2;
    let denominator_x = (width - denominator.width) / 2;

    for line in &numerator.lines {
        lines.push(centered_line(line.trim_end(), width));
    }
    lines.push("─".repeat(width));
    for line in &denominator.lines {
        lines.push(centered_line(line.trim_end(), width));
    }

    let mut rects = Vec::new();
    rects.extend(numerator.rects.iter().map(|rect| NodeRect {
        node_id: rect.node_id,
        x: rect.x + numerator_x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }));
    rects.extend(denominator.rects.iter().map(|rect| NodeRect {
        node_id: rect.node_id,
        x: rect.x + denominator_x,
        y: rect.y + numerator.height() + 1,
        width: rect.width,
        height: rect.height,
    }));

    MathBlock::with_rects(lines, numerator.height(), rects)
}

fn render_power_block(base: MathBlock, exponent: MathBlock) -> MathBlock {
    let height = exponent.height() + base.height();
    let width = base.width + exponent.width;
    let baseline = exponent.height() + base.baseline;
    let mut lines = vec![" ".repeat(width); height];

    for (row, line) in exponent.lines.iter().enumerate() {
        overwrite_segment(&mut lines[row], base.width, line);
    }

    for (row, line) in base.lines.iter().enumerate() {
        overwrite_segment(&mut lines[exponent.height() + row], 0, line);
    }

    let mut rects = Vec::new();
    rects.extend(base.rects.iter().copied());
    rects.extend(exponent.rects.iter().map(|rect| NodeRect {
        node_id: rect.node_id,
        x: rect.x + base.width,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }));

    MathBlock::with_rects(lines, baseline, rects)
}

fn render_root_block(index: Option<MathBlock>, radicand: MathBlock) -> MathBlock {
    let index = index.unwrap_or_else(|| MathBlock::from_text(""));
    if radicand.height() <= 1 && index.height() <= 1 {
        return render_inline_root_block(index, radicand);
    }

    let body_top = if index.width == 0 {
        0
    } else {
        index.height().saturating_sub(1)
    };
    let height = (body_top + radicand.height() + 1).max(index.height());
    let width = index.width + 1 + radicand.width;
    let baseline = body_top + 1 + radicand.baseline;
    let mut lines = vec![" ".repeat(width); height];

    for (row, line) in index.lines.iter().enumerate() {
        overwrite_segment(&mut lines[row], 0, line);
    }

    overwrite_segment(
        &mut lines[body_top],
        index.width,
        &format!(" {}", "─".repeat(radicand.width)),
    );

    for (row, line) in radicand.lines.iter().enumerate() {
        let prefix = if row == 0 { "√" } else { " " };
        overwrite_segment(&mut lines[body_top + 1 + row], index.width, prefix);
        overwrite_segment(&mut lines[body_top + 1 + row], index.width + 1, line);
    }

    let mut rects = Vec::new();
    rects.extend(index.rects.iter().copied());
    rects.extend(radicand.rects.iter().map(|rect| NodeRect {
        node_id: rect.node_id,
        x: rect.x + index.width + 1,
        y: rect.y + body_top + 1,
        width: rect.width,
        height: rect.height,
    }));

    MathBlock::with_rects(lines, baseline, rects)
}

fn render_inline_root_block(index: MathBlock, radicand: MathBlock) -> MathBlock {
    let radicand_text = radicand
        .lines
        .first()
        .map(|line| line.trim_end().to_owned())
        .unwrap_or_default();
    let index_text = index
        .lines
        .first()
        .map(|line| line.trim_end().to_owned())
        .unwrap_or_default();
    let root_text = format!("{index_text}√{radicand_text}");
    let root_offset = index_text.chars().count() + 1;

    let mut rects = Vec::new();
    rects.extend(index.rects.iter().map(|rect| NodeRect {
        node_id: rect.node_id,
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }));
    rects.extend(radicand.rects.iter().map(|rect| NodeRect {
        node_id: rect.node_id,
        x: rect.x + root_offset,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }));

    MathBlock::with_rects(vec![root_text], 0, rects)
}

fn overwrite_segment(target: &mut String, start: usize, segment: &str) {
    let mut chars: Vec<char> = target.chars().collect();
    for (offset, ch) in segment.chars().enumerate() {
        if start + offset < chars.len() {
            chars[start + offset] = ch;
        }
    }
    *target = chars.into_iter().collect();
}

fn format_number(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value.is_infinite() {
        return if value.is_sign_positive() {
            "inf".to_owned()
        } else {
            "-inf".to_owned()
        };
    }

    if value.abs() >= 1e12 {
        return format_scientific(value);
    }

    if nearly_integer(value) {
        let rounded = value.round();
        return if rounded == 0.0 {
            "0".to_owned()
        } else {
            format!("{rounded:.0}")
        };
    }

    if let Some((numerator, denominator)) = approximate_fraction(value, 1_000_000) {
        return format_fraction(numerator, denominator);
    }

    let mut plain = format!("{value:.12}");
    while plain.contains('.') && plain.ends_with('0') {
        plain.pop();
    }
    if plain.ends_with('.') {
        plain.pop();
    }
    if plain == "-0" {
        plain = "0".to_owned();
    }

    plain.push_str("...");
    plain
}

fn format_scientific(value: f64) -> String {
    let formatted = format!("{value:.10e}");
    let (mantissa, exponent) = formatted
        .split_once('e')
        .expect("scientific notation should contain exponent");
    let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
    let exponent_value = exponent.parse::<i32>().unwrap_or(0);
    format!("{mantissa}e{exponent_value}")
}

fn nearly_integer(value: f64) -> bool {
    (value - value.round()).abs() < 1e-12
}

fn approximate_fraction(value: f64, max_denominator: i128) -> Option<(i128, i128)> {
    if !value.is_finite() {
        return None;
    }

    let sign = if value.is_sign_negative() {
        -1_i128
    } else {
        1_i128
    };
    let target = value.abs();
    if target == 0.0 {
        return Some((0, 1));
    }

    let tolerance = 1e-15 * target.max(1.0);

    let mut x = target;
    let mut h_prev2 = 0_i128;
    let mut h_prev1 = 1_i128;
    let mut k_prev2 = 1_i128;
    let mut k_prev1 = 0_i128;

    for _ in 0..32 {
        let a = x.floor();
        if !a.is_finite() || a > i128::MAX as f64 {
            return None;
        }
        let a = a as i128;

        let numerator = a.checked_mul(h_prev1)?.checked_add(h_prev2)?;
        let denominator = a.checked_mul(k_prev1)?.checked_add(k_prev2)?;
        if denominator > max_denominator || denominator == 0 {
            break;
        }

        let approximation = numerator as f64 / denominator as f64;
        if (approximation - target).abs() <= tolerance {
            return Some((sign * numerator, denominator));
        }

        let fractional = x - a as f64;
        if fractional.abs() < 1e-15 {
            return Some((sign * numerator, denominator));
        }

        h_prev2 = h_prev1;
        h_prev1 = numerator;
        k_prev2 = k_prev1;
        k_prev1 = denominator;
        x = 1.0 / fractional;
    }

    None
}

fn format_fraction(numerator: i128, denominator: i128) -> String {
    if denominator == 0 {
        return "NaN".to_owned();
    }

    let negative = numerator < 0;
    let numerator = numerator.abs();
    let denominator = denominator.abs();
    let integer_part = numerator / denominator;
    let mut remainder = numerator % denominator;

    if remainder == 0 {
        let text = integer_part.to_string();
        return if negative { format!("-{text}") } else { text };
    }

    let mut seen = HashMap::new();
    let mut digits = Vec::new();
    let mut repeat_start = None;

    while remainder != 0 {
        if let Some(&start) = seen.get(&remainder) {
            repeat_start = Some(start);
            break;
        }

        if digits.len() >= 48 {
            break;
        }

        seen.insert(remainder, digits.len());
        remainder *= 10;
        digits.push(((remainder / denominator) as u8 + b'0') as char);
        remainder %= denominator;
    }

    let mut result = String::new();
    if negative {
        result.push('-');
    }
    result.push_str(&integer_part.to_string());
    result.push('.');

    match repeat_start {
        Some(start) => {
            let non_repeating: String = digits[..start].iter().collect();
            let repeating: String = digits[start..].iter().collect();
            result.push_str(&non_repeating);
            if repeating.len() <= 12 {
                result.push_str(&overline_digits(&repeating));
            } else {
                result.push_str(&repeating);
                result.push_str("...");
            }
        }
        None if remainder == 0 => {
            let decimal: String = digits.iter().collect();
            result.push_str(decimal.trim_end_matches('0'));
        }
        None => {
            let decimal: String = digits.iter().collect();
            result.push_str(decimal.trim_end_matches('0'));
            result.push_str("...");
        }
    }

    result
}

fn overline_digits(digits: &str) -> String {
    let mut output = String::with_capacity(digits.len() * 3);
    for ch in digits.chars() {
        output.push(ch);
        output.push('\u{0305}');
    }
    output
}

fn evaluate_with_variables(
    input: &str,
    ans: f64,
    angle_mode: AngleMode,
    variables: &BTreeMap<String, f64>,
) -> Result<f64, String> {
    let expr = parse_expression_input(input)?;
    let value = evaluate_expression(
        &expr,
        EvalContext {
            ans,
            angle_mode,
            variables,
        },
    )?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err("result is not finite".to_owned())
    }
}

fn preview_statement(
    input: &str,
    ans: f64,
    angle_mode: AngleMode,
    variables: &BTreeMap<String, f64>,
) -> Result<f64, String> {
    match parse_assignment_input(input)? {
        Some((_name, expression)) => {
            evaluate_with_variables(&expression, ans, angle_mode, variables)
        }
        None => evaluate_with_variables(input, ans, angle_mode, variables),
    }
}

fn parse_assignment_input(input: &str) -> Result<Option<(String, String)>, String> {
    let Some(index) = find_top_level_assignment(input) else {
        return Ok(None);
    };

    let (left, right_with_equals) = input.split_at(index);
    let right = &right_with_equals[1..];
    let name = left.trim();
    let expression = right.trim();

    if !is_valid_identifier(name) {
        return Err("invalid variable name".to_owned());
    }
    if expression.is_empty() {
        return Err("missing expression in assignment".to_owned());
    }
    if is_reserved_identifier(name) {
        return Err(format!("`{name}` is reserved"));
    }

    Ok(Some((name.to_owned(), expression.to_owned())))
}

fn assignment_expression_offset(input: &str) -> Option<usize> {
    let index = find_top_level_assignment(input)?;
    let rhs = &input[index + 1..];
    let trimmed = rhs.trim_start();
    let skipped = rhs.chars().count().saturating_sub(trimmed.chars().count());
    Some(input[..index + 1].chars().count() + skipped)
}

fn find_top_level_assignment(input: &str) -> Option<usize> {
    let mut depth = 0_i32;
    let mut in_abs = false;
    for (index, ch) in input.char_indices() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            '|' => in_abs = !in_abs,
            '=' if depth == 0 && !in_abs => return Some(index),
            _ => {}
        }
    }
    None
}

fn is_valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(ch) if ch.is_ascii_alphabetic() || ch == '_' => {}
        _ => return false,
    }

    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn is_reserved_identifier(name: &str) -> bool {
    supported_function_name(name) || matches!(name, "pi" | "e" | "ans" | "rad" | "deg")
}

fn render_statement_math(input: &str) -> MathBlock {
    match parse_assignment_input(input) {
        Ok(Some((name, expression))) => {
            if let Ok(expr) = parse_expression_input(&expression) {
                join_blocks(&[
                    MathBlock::from_text(format!("{name} = ")),
                    render_expression(&expr),
                ])
            } else {
                MathBlock::from_text(render_inline_fallback(input))
            }
        }
        _ => render_input_math(input),
    }
}

fn parse_expression_input(input: &str) -> Result<Expr, String> {
    let tokens = tokenize(input)?;
    if tokens.is_empty() {
        return Err("empty expression".to_owned());
    }
    let mut parser = Parser::new(tokens);
    let expr = parser.parse_expression()?;
    parser.expect_end()?;
    Ok(expr)
}

#[derive(Clone, Copy)]
struct EvalContext<'a> {
    ans: f64,
    angle_mode: AngleMode,
    variables: &'a BTreeMap<String, f64>,
}

#[derive(Clone, Copy, Debug)]
struct SourceSpan {
    start: usize,
    end: usize,
}

impl SourceSpan {
    fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    fn contains(self, index: usize) -> bool {
        index >= self.start && index < self.end
    }
}

#[derive(Clone, Debug)]
struct Expr {
    id: usize,
    span: SourceSpan,
    kind: ExprKind,
}

#[derive(Clone, Debug)]
enum ExprKind {
    Number(String),
    Identifier(String),
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Function {
        name: String,
        args: Vec<Expr>,
    },
    Group(Box<Expr>),
    Abs(Box<Expr>),
    Percent(Box<Expr>),
}

impl Expr {
    fn precedence(&self) -> u8 {
        match &self.kind {
            ExprKind::Number(_)
            | ExprKind::Identifier(_)
            | ExprKind::Function { .. }
            | ExprKind::Group(_)
            | ExprKind::Abs(_) => 5,
            ExprKind::Percent(_) | ExprKind::Unary { .. } => 4,
            ExprKind::Binary {
                op: BinaryOp::Power,
                ..
            } => 3,
            ExprKind::Binary {
                op: BinaryOp::Multiply(_) | BinaryOp::Divide,
                ..
            } => 2,
            ExprKind::Binary {
                op: BinaryOp::Add | BinaryOp::Subtract,
                ..
            } => 1,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum UnaryOp {
    Negate,
}

#[derive(Clone, Copy, Debug)]
enum BinaryOp {
    Add,
    Subtract,
    Multiply(MultiplyStyle),
    Divide,
    Power,
}

#[derive(Clone, Copy, Debug)]
enum MultiplyStyle {
    Explicit,
    Implicit,
}

#[derive(Clone, Debug)]
struct Token {
    kind: TokenKindValue,
    span: SourceSpan,
}

#[derive(Clone, Debug)]
enum TokenKindValue {
    Number { raw: String },
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    Percent,
    Comma,
    LParen,
    RParen,
    Pipe,
}

fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut index = 0;

    while index < chars.len() {
        let ch = chars[index];
        if ch.is_whitespace() {
            index += 1;
            continue;
        }

        if ch.is_ascii_digit()
            || (ch == '.' && chars.get(index + 1).is_some_and(char::is_ascii_digit))
        {
            let start = index;
            let mut seen_dot = ch == '.';
            index += 1;
            while index < chars.len() {
                let next = chars[index];
                if next == '.' {
                    if seen_dot {
                        return Err("invalid number literal".to_owned());
                    }
                    seen_dot = true;
                    index += 1;
                } else if next.is_ascii_digit() {
                    index += 1;
                } else {
                    break;
                }
            }

            let literal: String = chars[start..index].iter().collect();
            literal
                .parse::<f64>()
                .map_err(|_| format!("invalid number `{literal}`"))?;
            tokens.push(Token {
                kind: TokenKindValue::Number { raw: literal },
                span: SourceSpan::new(start, index),
            });
            continue;
        }

        if ch.is_ascii_alphabetic() || ch == '_' {
            let start = index;
            index += 1;
            while index < chars.len()
                && (chars[index].is_ascii_alphanumeric() || chars[index] == '_')
            {
                index += 1;
            }
            let ident: String = chars[start..index].iter().collect();
            tokens.push(Token {
                kind: TokenKindValue::Ident(ident),
                span: SourceSpan::new(start, index),
            });
            continue;
        }

        let kind = match ch {
            '+' => TokenKindValue::Plus,
            '-' => TokenKindValue::Minus,
            '*' => TokenKindValue::Star,
            '/' => TokenKindValue::Slash,
            '^' => TokenKindValue::Caret,
            '%' => TokenKindValue::Percent,
            ',' => TokenKindValue::Comma,
            '(' | '[' => TokenKindValue::LParen,
            ')' | ']' => TokenKindValue::RParen,
            '|' => TokenKindValue::Pipe,
            _ => return Err(format!("unexpected character `{ch}`")),
        };
        tokens.push(Token {
            kind,
            span: SourceSpan::new(index, index + 1),
        });
        index += 1;
    }

    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    position: usize,
    next_id: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            position: 0,
            next_id: 0,
        }
    }

    fn make_expr(&mut self, span: SourceSpan, kind: ExprKind) -> Expr {
        let id = self.next_id;
        self.next_id += 1;
        Expr { id, span, kind }
    }

    fn parse_expression(&mut self) -> Result<Expr, String> {
        self.parse_sum()
    }

    fn parse_sum(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_product()?;
        loop {
            if self.matches(TokenKind::Plus) {
                let right = self.parse_product()?;
                let span = SourceSpan::new(expr.span.start, right.span.end);
                expr = self.make_expr(
                    span,
                    ExprKind::Binary {
                        op: BinaryOp::Add,
                        left: Box::new(expr),
                        right: Box::new(right),
                    },
                );
            } else if self.matches(TokenKind::Minus) {
                let right = self.parse_product()?;
                let span = SourceSpan::new(expr.span.start, right.span.end);
                expr = self.make_expr(
                    span,
                    ExprKind::Binary {
                        op: BinaryOp::Subtract,
                        left: Box::new(expr),
                        right: Box::new(right),
                    },
                );
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_product(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_power()?;
        loop {
            if self.matches(TokenKind::Star) {
                let right = self.parse_power()?;
                let span = SourceSpan::new(expr.span.start, right.span.end);
                expr = self.make_expr(
                    span,
                    ExprKind::Binary {
                        op: BinaryOp::Multiply(MultiplyStyle::Explicit),
                        left: Box::new(expr),
                        right: Box::new(right),
                    },
                );
            } else if self.matches(TokenKind::Slash) {
                let right = self.parse_power()?;
                let span = SourceSpan::new(expr.span.start, right.span.end);
                expr = self.make_expr(
                    span,
                    ExprKind::Binary {
                        op: BinaryOp::Divide,
                        left: Box::new(expr),
                        right: Box::new(right),
                    },
                );
            } else if self.next_starts_implicit_product() {
                let right = self.parse_power()?;
                let span = SourceSpan::new(expr.span.start, right.span.end);
                expr = self.make_expr(
                    span,
                    ExprKind::Binary {
                        op: BinaryOp::Multiply(MultiplyStyle::Implicit),
                        left: Box::new(expr),
                        right: Box::new(right),
                    },
                );
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_power(&mut self) -> Result<Expr, String> {
        let expr = self.parse_unary()?;
        if self.matches(TokenKind::Caret) {
            let right = self.parse_power()?;
            let span = SourceSpan::new(expr.span.start, right.span.end);
            Ok(self.make_expr(
                span,
                ExprKind::Binary {
                    op: BinaryOp::Power,
                    left: Box::new(expr),
                    right: Box::new(right),
                },
            ))
        } else {
            Ok(expr)
        }
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        if self.matches(TokenKind::Plus) {
            return self.parse_unary();
        }
        if let Some(token) = self.take(TokenKind::Minus) {
            let expr = self.parse_unary()?;
            let span = SourceSpan::new(token.span.start, expr.span.end);
            return Ok(self.make_expr(
                span,
                ExprKind::Unary {
                    op: UnaryOp::Negate,
                    expr: Box::new(expr),
                },
            ));
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_primary()?;
        while let Some(token) = self.take(TokenKind::Percent) {
            let span = SourceSpan::new(expr.span.start, token.span.end);
            expr = self.make_expr(span, ExprKind::Percent(Box::new(expr)));
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        match self.peek().cloned() {
            Some(Token {
                kind: TokenKindValue::Number { raw },
                span,
            }) => {
                self.position += 1;
                Ok(self.make_expr(span, ExprKind::Number(raw)))
            }
            Some(Token {
                kind: TokenKindValue::Ident(name),
                span,
            }) => {
                self.position += 1;
                if supported_function_name(&name) {
                    let exponent = if self.matches(TokenKind::Caret) {
                        Some(self.parse_unary()?)
                    } else {
                        None
                    };

                    if self.matches(TokenKind::LParen) {
                        let args = self.parse_arguments()?;
                        let closing = self.previous_span();
                        let function = self.make_expr(
                            SourceSpan::new(span.start, closing.end),
                            ExprKind::Function { name, args },
                        );

                        if let Some(exponent) = exponent {
                            Ok(self.make_expr(
                                SourceSpan::new(span.start, closing.end),
                                ExprKind::Binary {
                                    op: BinaryOp::Power,
                                    left: Box::new(function),
                                    right: Box::new(exponent),
                                },
                            ))
                        } else {
                            Ok(function)
                        }
                    } else if exponent.is_some() {
                        Err("expected `(` after function exponent".to_owned())
                    } else {
                        Ok(self.make_expr(span, ExprKind::Identifier(name)))
                    }
                } else {
                    Ok(self.make_expr(span, ExprKind::Identifier(name)))
                }
            }
            Some(Token {
                kind: TokenKindValue::LParen,
                span,
            }) => {
                self.position += 1;
                let expr = self.parse_expression()?;
                let closing = self.expect(TokenKind::RParen, "expected `)`")?;
                Ok(self.make_expr(
                    SourceSpan::new(span.start, closing.end),
                    ExprKind::Group(Box::new(expr)),
                ))
            }
            Some(Token {
                kind: TokenKindValue::Pipe,
                span,
            }) => {
                self.position += 1;
                let expr = self.parse_expression()?;
                let closing = self.expect(TokenKind::Pipe, "expected closing `|`")?;
                Ok(self.make_expr(
                    SourceSpan::new(span.start, closing.end),
                    ExprKind::Abs(Box::new(expr)),
                ))
            }
            _ => Err("expected a number, identifier, or group".to_owned()),
        }
    }

    fn parse_arguments(&mut self) -> Result<Vec<Expr>, String> {
        let mut args = Vec::new();
        if self.matches(TokenKind::RParen) {
            return Ok(args);
        }

        loop {
            args.push(self.parse_expression()?);
            if self.matches(TokenKind::Comma) {
                continue;
            }
            self.expect(TokenKind::RParen, "expected `)` after arguments")?;
            break;
        }
        Ok(args)
    }

    fn expect_end(&self) -> Result<(), String> {
        if self.position == self.tokens.len() {
            Ok(())
        } else {
            Err("unexpected trailing input".to_owned())
        }
    }

    fn expect(&mut self, kind: TokenKind, message: &str) -> Result<SourceSpan, String> {
        self.take(kind)
            .map(|token| token.span)
            .ok_or_else(|| message.to_owned())
    }

    fn matches(&mut self, kind: TokenKind) -> bool {
        self.take(kind).is_some()
    }

    fn take(&mut self, kind: TokenKind) -> Option<Token> {
        if self.peek().is_some_and(|token| token_kind(token) == kind) {
            let token = self.tokens.get(self.position).cloned();
            self.position += 1;
            token
        } else {
            None
        }
    }

    fn previous_span(&self) -> SourceSpan {
        self.tokens[self.position - 1].span
    }

    fn next_starts_implicit_product(&self) -> bool {
        self.peek().is_some_and(token_starts_implicit_product)
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TokenKind {
    Number,
    Ident,
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    Percent,
    Comma,
    LParen,
    RParen,
    Pipe,
}

fn token_kind(token: &Token) -> TokenKind {
    match &token.kind {
        TokenKindValue::Number { .. } => TokenKind::Number,
        TokenKindValue::Ident(_) => TokenKind::Ident,
        TokenKindValue::Plus => TokenKind::Plus,
        TokenKindValue::Minus => TokenKind::Minus,
        TokenKindValue::Star => TokenKind::Star,
        TokenKindValue::Slash => TokenKind::Slash,
        TokenKindValue::Caret => TokenKind::Caret,
        TokenKindValue::Percent => TokenKind::Percent,
        TokenKindValue::Comma => TokenKind::Comma,
        TokenKindValue::LParen => TokenKind::LParen,
        TokenKindValue::RParen => TokenKind::RParen,
        TokenKindValue::Pipe => TokenKind::Pipe,
    }
}

fn evaluate_expression(expr: &Expr, context: EvalContext<'_>) -> Result<f64, String> {
    match &expr.kind {
        ExprKind::Number(raw) => raw
            .parse::<f64>()
            .map_err(|_| format!("invalid number `{raw}`")),
        ExprKind::Identifier(name) => resolve_identifier(name, context),
        ExprKind::Unary {
            op: UnaryOp::Negate,
            expr,
        } => Ok(-evaluate_expression(expr, context)?),
        ExprKind::Binary { op, left, right } => {
            let left_value = evaluate_expression(left, context)?;
            let right_value = evaluate_expression(right, context)?;
            match op {
                BinaryOp::Add => Ok(left_value + right_value),
                BinaryOp::Subtract => Ok(left_value - right_value),
                BinaryOp::Multiply(_) => Ok(left_value * right_value),
                BinaryOp::Divide => {
                    if right_value == 0.0 {
                        Err("division by zero".to_owned())
                    } else {
                        Ok(left_value / right_value)
                    }
                }
                BinaryOp::Power => Ok(left_value.powf(right_value)),
            }
        }
        ExprKind::Function { name, args } => {
            let evaluated_args = args
                .iter()
                .map(|arg| evaluate_expression(arg, context))
                .collect::<Result<Vec<_>, _>>()?;
            apply_function(name, &evaluated_args, context)
        }
        ExprKind::Group(expr) => evaluate_expression(expr, context),
        ExprKind::Abs(expr) => Ok(evaluate_expression(expr, context)?.abs()),
        ExprKind::Percent(expr) => Ok(evaluate_expression(expr, context)? / 100.0),
    }
}

fn resolve_identifier(name: &str, context: EvalContext<'_>) -> Result<f64, String> {
    if let Some(value) = context.variables.get(name) {
        Ok(*value)
    } else {
        match name {
            "pi" => Ok(PI),
            "e" => Ok(E),
            "ans" => Ok(context.ans),
            _ => decompose_identifier_product(name, context)
                .ok_or_else(|| format!("unknown identifier `{name}`")),
        }
    }
}

fn decompose_identifier_product(name: &str, context: EvalContext<'_>) -> Option<f64> {
    if name.is_empty() {
        return None;
    }

    fn dfs(
        name: &str,
        index: usize,
        context: EvalContext<'_>,
        memo: &mut HashMap<usize, Option<f64>>,
    ) -> Option<f64> {
        if let Some(cached) = memo.get(&index) {
            return *cached;
        }
        if index == name.len() {
            return Some(1.0);
        }

        let suffix = &name[index..];
        let mut candidates: Vec<(usize, f64)> = Vec::new();

        if suffix.as_bytes()[0].is_ascii_digit() {
            let digit_len = suffix
                .bytes()
                .take_while(|byte| byte.is_ascii_digit())
                .count();
            if digit_len > 0 {
                let value = suffix[..digit_len].parse::<f64>().ok()?;
                candidates.push((digit_len, value));
            }
        }

        for key in context.variables.keys() {
            if suffix.starts_with(key) {
                if let Some(value) = context.variables.get(key) {
                    candidates.push((key.len(), *value));
                }
            }
        }

        for (constant, value) in [("pi", PI), ("e", E), ("ans", context.ans)] {
            if suffix.starts_with(constant) {
                candidates.push((constant.len(), value));
            }
        }

        candidates.sort_by(|a, b| b.0.cmp(&a.0));
        let result = candidates.into_iter().find_map(|(length, value)| {
            dfs(name, index + length, context, memo).map(|rest| value * rest)
        });
        memo.insert(index, result);
        result
    }

    let mut memo = HashMap::new();
    dfs(name, 0, context, &mut memo)
}

fn apply_function(name: &str, args: &[f64], context: EvalContext<'_>) -> Result<f64, String> {
    match name.to_ascii_lowercase().as_str() {
        "sin" => unary_function(name, args, |value| {
            Ok(context.angle_mode.to_radians(value).sin())
        }),
        "cos" => unary_function(name, args, |value| {
            Ok(context.angle_mode.to_radians(value).cos())
        }),
        "tan" => unary_function(name, args, |value| {
            Ok(context.angle_mode.to_radians(value).tan())
        }),
        "sqrt" => unary_function(name, args, |value| {
            if value < 0.0 {
                Err("sqrt domain error".to_owned())
            } else {
                Ok(value.sqrt())
            }
        }),
        "abs" => unary_function(name, args, |value| Ok(value.abs())),
        "sq" => unary_function(name, args, |value| Ok(value * value)),
        "pow" => binary_function(name, args, |a, b| Ok(a.powf(b))),
        "frac" => binary_function(name, args, |a, b| {
            if b == 0.0 {
                Err("division by zero".to_owned())
            } else {
                Ok(a / b)
            }
        }),
        "root" | "nroot" => binary_function(name, args, nth_root),
        _ => Err(format!("unknown function `{name}`")),
    }
}

fn unary_function<F>(name: &str, args: &[f64], func: F) -> Result<f64, String>
where
    F: FnOnce(f64) -> Result<f64, String>,
{
    if args.len() != 1 {
        return Err(format!("`{name}` expects 1 argument"));
    }
    func(args[0])
}

fn binary_function<F>(name: &str, args: &[f64], func: F) -> Result<f64, String>
where
    F: FnOnce(f64, f64) -> Result<f64, String>,
{
    if args.len() != 2 {
        return Err(format!("`{name}` expects 2 arguments"));
    }
    func(args[0], args[1])
}

fn nth_root(degree: f64, value: f64) -> Result<f64, String> {
    if degree == 0.0 {
        return Err("root degree cannot be zero".to_owned());
    }
    if value < 0.0 && is_odd_integer(degree) {
        Ok(-(-value).powf(1.0 / degree.abs()))
    } else if value < 0.0 {
        Err("root domain error".to_owned())
    } else {
        Ok(value.powf(1.0 / degree))
    }
}

fn is_odd_integer(value: f64) -> bool {
    let rounded = value.round();
    (value - rounded).abs() < 1e-9 && rounded.rem_euclid(2.0) != 0.0
}

fn render_expression(expr: &Expr) -> MathBlock {
    render_expression_with_parent(expr, 0)
}

fn render_expression_with_parent(expr: &Expr, parent_precedence: u8) -> MathBlock {
    let mut block = match &expr.kind {
        ExprKind::Number(raw) => MathBlock::from_text(raw.clone()),
        ExprKind::Identifier(name) => MathBlock::from_text(render_identifier(name)),
        ExprKind::Unary {
            op: UnaryOp::Negate,
            expr,
        } => join_blocks(&[
            MathBlock::from_text("−"),
            render_expression_with_parent(expr, 4),
        ]),
        ExprKind::Binary { op, left, right } => match op {
            BinaryOp::Add => join_blocks(&[
                render_expression_with_parent(left, 1),
                MathBlock::from_text(" + "),
                render_expression_with_parent(right, 1),
            ]),
            BinaryOp::Subtract => join_blocks(&[
                render_expression_with_parent(left, 1),
                MathBlock::from_text(" − "),
                render_expression_with_parent(right, 2),
            ]),
            BinaryOp::Multiply(style) => join_blocks(&[
                render_expression_with_parent(left, 2),
                MathBlock::from_text(match style {
                    MultiplyStyle::Explicit => " × ",
                    MultiplyStyle::Implicit => " ",
                }),
                render_expression_with_parent(right, 2),
            ]),
            BinaryOp::Divide => render_fraction_block(
                render_expression_with_parent(left, 0),
                render_expression_with_parent(right, 0),
            ),
            BinaryOp::Power => render_power_block(
                render_expression_with_parent(left, 4),
                render_expression_with_parent(right, 4),
            ),
        },
        ExprKind::Function { name, args } => render_function_expression(name, args),
        ExprKind::Group(expr) => wrap_parentheses(render_expression_with_parent(expr, 0)),
        ExprKind::Abs(expr) => wrap_absolute(render_expression_with_parent(expr, 0)),
        ExprKind::Percent(expr) => join_blocks(&[
            render_expression_with_parent(expr, 4),
            MathBlock::from_text("%"),
        ]),
    };

    if !matches!(expr.kind, ExprKind::Group(_)) && expr.precedence() < parent_precedence {
        block = wrap_parentheses(block);
    }

    block.rects.push(NodeRect {
        node_id: expr.id,
        x: 0,
        y: 0,
        width: block.width,
        height: block.height(),
    });
    block
}

fn render_function_expression(name: &str, args: &[Expr]) -> MathBlock {
    match (name, args) {
        ("sqrt", [value]) => render_root_block(None, render_expression_with_parent(value, 0)),
        ("root" | "nroot", [index, value]) => render_root_block(
            Some(render_expression_with_parent(index, 4)),
            render_expression_with_parent(value, 0),
        ),
        ("frac", [numerator, denominator]) => render_fraction_block(
            render_expression_with_parent(numerator, 0),
            render_expression_with_parent(denominator, 0),
        ),
        ("pow", [base, exponent]) => render_power_block(
            render_expression_with_parent(base, 4),
            render_expression_with_parent(exponent, 4),
        ),
        ("sq", [value]) => render_power_block(
            render_expression_with_parent(value, 4),
            MathBlock::from_text("2"),
        ),
        _ => render_generic_function(name, args),
    }
}

fn render_generic_function(name: &str, args: &[Expr]) -> MathBlock {
    let mut parts = Vec::new();
    for (index, arg) in args.iter().enumerate() {
        if index > 0 {
            parts.push(MathBlock::from_text(", "));
        }
        parts.push(render_expression_with_parent(arg, 0));
    }

    let arguments = if parts.is_empty() {
        MathBlock::from_text("()")
    } else {
        wrap_parentheses(join_blocks(&parts))
    };

    join_blocks(&[MathBlock::from_text(name.to_owned()), arguments])
}

fn render_identifier(name: &str) -> String {
    match name {
        "pi" => "π".to_owned(),
        "e" => "ℯ".to_owned(),
        other => other.to_owned(),
    }
}

fn render_input_math(input: &str) -> MathBlock {
    if input.trim().is_empty() {
        MathBlock::from_text("")
    } else if let Ok(expr) = parse_expression_input(input) {
        render_expression(&expr)
    } else {
        MathBlock::from_text(render_inline_fallback(input))
    }
}

fn render_inline_fallback(input: &str) -> String {
    input
        .replace("pi", "π")
        .replace('*', "×")
        .replace(" e", " ℯ")
}

fn collect_node_spans(expr: &Expr, output: &mut Vec<NodeSpan>) {
    output.push(NodeSpan {
        node_id: expr.id,
        span: expr.span,
    });

    match &expr.kind {
        ExprKind::Unary { expr, .. }
        | ExprKind::Group(expr)
        | ExprKind::Abs(expr)
        | ExprKind::Percent(expr) => collect_node_spans(expr, output),
        ExprKind::Binary { left, right, .. } => {
            collect_node_spans(left, output);
            collect_node_spans(right, output);
        }
        ExprKind::Function { args, .. } => {
            for arg in args {
                collect_node_spans(arg, output);
            }
        }
        ExprKind::Number(_) | ExprKind::Identifier(_) => {}
    }
}

fn styled_math_lines(math: &MathBlock, hovered_node: Option<usize>) -> Vec<Line<'static>> {
    let highlight = hovered_node.and_then(|node_id| {
        math.rects
            .iter()
            .filter(|rect| rect.node_id == node_id)
            .min_by_key(|rect| rect.width * rect.height)
            .copied()
    });

    math.lines
        .iter()
        .enumerate()
        .map(|(row, line)| style_math_line(line, row, highlight))
        .collect()
}

fn style_math_line(line: &str, row: usize, highlight: Option<NodeRect>) -> Line<'static> {
    let chars: Vec<char> = line.chars().collect();
    let mut spans = Vec::new();
    let mut buffer = String::new();
    let mut active = None;

    for (column, ch) in chars.iter().enumerate() {
        let is_highlighted = highlight.is_some_and(|rect| {
            row >= rect.y
                && row < rect.y + rect.height
                && column >= rect.x
                && column < rect.x + rect.width
        });

        if active != Some(is_highlighted) && !buffer.is_empty() {
            spans.push(Span::styled(
                buffer.clone(),
                math_style(active.unwrap_or(false)),
            ));
            buffer.clear();
        }
        active = Some(is_highlighted);
        buffer.push(*ch);
    }

    if !buffer.is_empty() {
        spans.push(Span::styled(buffer, math_style(active.unwrap_or(false))));
    }

    Line::from(spans)
}

fn build_input_line(input: &str, highlight: Option<SourceSpan>) -> Line<'static> {
    let chars: Vec<char> = input.chars().collect();
    let mut spans = vec![Span::styled(
        "> ".to_owned(),
        Style::default().fg(Color::Yellow),
    )];
    let mut buffer = String::new();
    let mut active = None;

    for (index, ch) in chars.iter().enumerate() {
        let is_highlighted = highlight.is_some_and(|span| span.contains(index));
        if active != Some(is_highlighted) && !buffer.is_empty() {
            spans.push(Span::styled(
                buffer.clone(),
                input_style(active.unwrap_or(false)),
            ));
            buffer.clear();
        }
        active = Some(is_highlighted);
        buffer.push(*ch);
    }

    if !buffer.is_empty() {
        spans.push(Span::styled(buffer, input_style(active.unwrap_or(false))));
    }

    if chars.is_empty() {
        spans.push(Span::raw(String::new()));
    }

    Line::from(spans)
}

fn math_style(highlighted: bool) -> Style {
    if highlighted {
        Style::default().fg(Color::Black).bg(Color::Yellow)
    } else {
        Style::default().fg(Color::White)
    }
}

fn input_style(highlighted: bool) -> Style {
    if highlighted {
        Style::default().fg(Color::Black).bg(Color::Yellow)
    } else {
        Style::default().fg(Color::White)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AngleMode, App, evaluate_with_variables, format_number, handle_key, parse_expression_input,
        render_expression,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::collections::BTreeMap;
    use std::f64::consts::E;

    fn approx_eq(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "expected {expected}, got {actual}"
        );
    }

    fn evaluate(input: &str, ans: f64, angle_mode: AngleMode) -> Result<f64, String> {
        evaluate_with_variables(input, ans, angle_mode, &BTreeMap::new())
    }

    fn rendered_lines(input: &str) -> Vec<String> {
        render_expression(&parse_expression_input(input).unwrap())
            .lines
            .into_iter()
            .map(|line| line.trim_end().to_owned())
            .collect()
    }

    #[test]
    fn evaluates_trig_in_degree_mode() {
        let value = evaluate("sin(30)", 0.0, AngleMode::Degrees).unwrap();
        approx_eq(value, 0.5);
    }

    #[test]
    fn evaluates_trig_in_radian_mode() {
        let value = evaluate("sin(pi / 2)", 0.0, AngleMode::Radians).unwrap();
        approx_eq(value, 1.0);
    }

    #[test]
    fn evaluates_e_constant() {
        let value = evaluate("e", 0.0, AngleMode::Radians).unwrap();
        approx_eq(value, E);
    }

    #[test]
    fn square_brackets_parse_like_parentheses() {
        let value = evaluate("[1 + 2] * 3", 0.0, AngleMode::Radians).unwrap();
        approx_eq(value, 9.0);
    }

    #[test]
    fn supports_implicit_multiplication_with_parentheses() {
        let value = evaluate("2(3 + 4)", 0.0, AngleMode::Radians).unwrap();
        approx_eq(value, 14.0);
    }

    #[test]
    fn supports_implicit_multiplication_between_grouped_expressions() {
        let value = evaluate("(1 + 2)(3 + 4)", 0.0, AngleMode::Radians).unwrap();
        approx_eq(value, 21.0);
    }

    #[test]
    fn supports_implicit_multiplication_with_constants_and_functions() {
        let constant = evaluate("pi(2)", 0.0, AngleMode::Radians).unwrap();
        approx_eq(constant, 2.0 * std::f64::consts::PI);

        let function = evaluate("2sin(30)", 0.0, AngleMode::Degrees).unwrap();
        approx_eq(function, 1.0);
    }

    #[test]
    fn supports_function_power_shorthand() {
        let value = evaluate("sin^2(30)", 0.0, AngleMode::Degrees).unwrap();
        approx_eq(value, 0.25);

        let radians = evaluate("sin^2(pi / 2)", 0.0, AngleMode::Radians).unwrap();
        approx_eq(radians, 1.0);
    }

    #[test]
    fn supports_absolute_value_and_percent() {
        let value = evaluate("|-25| + 50%", 0.0, AngleMode::Degrees).unwrap();
        approx_eq(value, 25.5);
    }

    #[test]
    fn supports_ans_and_nth_root() {
        let value = evaluate("root(3, ans)", 27.0, AngleMode::Degrees).unwrap();
        approx_eq(value, 3.0);
    }

    #[test]
    fn keeps_power_right_associative() {
        let value = evaluate("2^3^2", 0.0, AngleMode::Degrees).unwrap();
        approx_eq(value, 512.0);
    }

    #[test]
    fn formats_terminating_decimals_without_trailing_zeroes() {
        assert_eq!(format_number(12.340000000000), "12.34");
        assert_eq!(format_number(2.0), "2");
    }

    #[test]
    fn formats_huge_numbers_in_scientific_notation() {
        assert_eq!(format_number(1_000_000_000_000.0), "1e12");
        assert_eq!(format_number(1_234_567_890_123.0), "1.2345678901e12");
    }

    #[test]
    fn formats_repeating_decimals_with_an_overline() {
        assert_eq!(format_number(1.0 / 3.0), "0.3\u{0305}");
        assert_eq!(
            format_number(2.0 / 7.0),
            "0.2\u{0305}8\u{0305}5\u{0305}7\u{0305}1\u{0305}4\u{0305}"
        );
    }

    #[test]
    fn formats_non_terminating_non_repeating_values_with_ellipsis() {
        let rendered = format_number(2.0_f64.sqrt());
        assert!(rendered.starts_with("1.414213562"));
        assert!(rendered.ends_with("..."));
        assert!(!rendered.contains('e'));
    }

    #[test]
    fn renders_division_as_a_fraction() {
        assert_eq!(rendered_lines("1/2"), vec![" 1", "───", " 2"]);
    }

    #[test]
    fn renders_powers_above_the_baseline() {
        assert_eq!(rendered_lines("2^3"), vec![" 3", "2"]);
    }

    #[test]
    fn renders_function_power_shorthand_as_a_power() {
        let rendered = rendered_lines("sin^2(3)");
        let joined = rendered.join("\n");
        assert!(joined.contains('2'));
        assert!(joined.contains("sin"));
        assert!(joined.contains('3'));
    }

    #[test]
    fn renders_implicit_function_multiplication_without_a_times_symbol() {
        let rendered = rendered_lines("3sin(3)");
        let joined = rendered.join("\n");
        assert!(joined.contains("3 sin"));
        assert!(!joined.contains('×'));
    }

    #[test]
    fn renders_explicit_function_multiplication_with_a_times_symbol() {
        let rendered = rendered_lines("3*sin(3)");
        let joined = rendered.join("\n");
        assert!(joined.contains('×'));
    }

    #[test]
    fn renders_roots_and_math_symbols() {
        let rendered = rendered_lines("sqrt(pi*e)");
        let joined = rendered.join("\n");
        assert!(joined.contains('√'));
        assert!(joined.contains('π'));
        assert!(joined.contains('ℯ'));
        assert!(joined.contains('×'));
    }

    #[test]
    fn renders_single_line_square_roots_inline() {
        let rendered = rendered_lines("sqrt(3)+2");
        assert_eq!(rendered.len(), 1);
        assert_eq!(rendered[0], "√3 + 2");
    }

    #[test]
    fn opening_paren_autocreates_a_pair() {
        let mut app = App::default();
        app.insert_char('(');
        assert_eq!(app.input, "()");
        assert_eq!(app.cursor, 1);
    }

    #[test]
    fn opening_bracket_wraps_the_expression_to_the_right() {
        let mut app = App::default();
        app.input = "1+2".to_owned();
        app.cursor = 2;
        app.insert_char('[');
        assert_eq!(app.input, "1+[2]");
        assert_eq!(app.cursor, 3);
    }

    #[test]
    fn opening_paren_wraps_a_full_expression_suffix() {
        let mut app = App::default();
        app.input = "1+2*3".to_owned();
        app.cursor = 0;
        app.insert_char('(');
        assert_eq!(app.input, "(1+2*3)");
        assert_eq!(app.cursor, 1);
    }

    #[test]
    fn backspace_deletes_autopaired_delimiters_together() {
        let mut app = App::default();
        app.input = "()".to_owned();
        app.cursor = 1;
        app.backspace();
        assert_eq!(app.input, "");
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn typing_closer_over_existing_closer_advances_cursor() {
        let mut app = App::default();
        app.input = "()".to_owned();
        app.cursor = 1;
        app.insert_char(')');
        assert_eq!(app.input, "()");
        assert_eq!(app.cursor, 2);
    }

    #[test]
    fn closing_paren_wraps_the_expression_to_the_left() {
        let mut app = App::default();
        app.input = "1+2*3".to_owned();
        app.cursor = app.input.chars().count();
        app.insert_char(')');
        assert_eq!(app.input, "(1+2*3)");
        assert_eq!(app.cursor, app.input.chars().count());
    }

    #[test]
    fn closing_bracket_wraps_the_expression_to_the_left() {
        let mut app = App::default();
        app.input = "1+2".to_owned();
        app.cursor = app.input.chars().count();
        app.insert_char(']');
        assert_eq!(app.input, "[1+2]");
        assert_eq!(app.cursor, app.input.chars().count());
    }

    #[test]
    fn assignments_create_variables_and_history_entries() {
        let mut app = App::default();
        app.input = "x = 5".to_owned();
        app.cursor = app.input.chars().count();
        app.submit();

        assert_eq!(app.variables.get("x"), Some(&5.0));
        assert_eq!(app.ans, 5.0);
        assert_eq!(app.history.len(), 1);
        assert_eq!(app.history[0].input, "x = 5");
    }

    #[test]
    fn variables_support_case_and_underscores_with_implicit_multiplication() {
        let mut app = App::default();
        app.input = "Foo_bar = 7".to_owned();
        app.cursor = app.input.chars().count();
        app.submit();

        app.input = "2Foo_bar".to_owned();
        app.cursor = app.input.chars().count();
        app.submit();

        assert_eq!(app.ans, 14.0);
    }

    #[test]
    fn adjacent_variables_decompose_as_implicit_products() {
        let mut app = App::default();
        app.variables.insert("x".to_owned(), 2.0);
        app.variables.insert("y".to_owned(), 3.0);

        app.input = "xy".to_owned();
        app.cursor = app.input.chars().count();
        app.submit();
        assert_eq!(app.ans, 6.0);

        app.input = "xxx".to_owned();
        app.cursor = app.input.chars().count();
        app.submit();
        assert_eq!(app.ans, 8.0);
    }

    #[test]
    fn variable_followed_by_digits_decomposes_as_multiplication() {
        let mut app = App::default();
        app.variables.insert("x".to_owned(), 4.0);

        app.input = "x5".to_owned();
        app.cursor = app.input.chars().count();
        app.submit();
        assert_eq!(app.ans, 20.0);
    }

    #[test]
    fn exact_variable_name_wins_over_decomposition() {
        let mut app = App::default();
        app.variables.insert("x".to_owned(), 2.0);
        app.variables.insert("y".to_owned(), 3.0);
        app.variables.insert("xy".to_owned(), 11.0);

        app.input = "xy".to_owned();
        app.cursor = app.input.chars().count();
        app.submit();
        assert_eq!(app.ans, 11.0);
    }

    #[test]
    fn history_navigation_restores_previous_inputs() {
        let mut app = App::default();
        app.input = "1+1".to_owned();
        app.cursor = app.input.chars().count();
        app.submit();

        app.input = "2+2".to_owned();
        app.cursor = app.input.chars().count();
        app.submit();

        app.input = "draft".to_owned();
        app.cursor = app.input.chars().count();
        app.navigate_history_up();
        assert_eq!(app.input, "2+2");

        app.navigate_history_up();
        assert_eq!(app.input, "1+1");

        app.navigate_history_down();
        assert_eq!(app.input, "2+2");

        app.navigate_history_down();
        assert_eq!(app.input, "draft");
    }

    #[test]
    fn help_is_opened_by_keybind_not_by_command() {
        let mut app = App::default();
        handle_key(&mut app, KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
        assert!(app.show_help);

        handle_key(&mut app, KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
        assert!(!app.show_help);

        app.input = "help".to_owned();
        app.cursor = app.input.chars().count();
        app.submit();
        assert!(!app.show_help);
        assert!(app.status.contains("Error"));
    }
}
