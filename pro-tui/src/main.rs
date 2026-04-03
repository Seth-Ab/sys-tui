use std::env;
use std::io;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use reqwest::blocking::Client;
use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
struct ProcessInfo {
    pid: u32,
    name: String,
    cpu_percent: f32,
    memory_bytes: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct SystemMetrics {
    cpu_percent: f32,
    memory_used_bytes: u64,
    memory_total_bytes: u64,
    swap_used_bytes: u64,
    swap_total_bytes: u64,
    root_used_bytes: u64,
    root_total_bytes: u64,
    top_processes: Vec<ProcessInfo>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct LlmMetrics {
    ollama_online: bool,
    ollama_ps_url: String,
    running_models: Vec<String>,
    model_count: usize,
    error: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct AgentState {
    seq: u64,
    ts_ms: u128,
    hostname: String,
    watched_dirs: Vec<String>,
    recent_file_events: Vec<String>,
    system: SystemMetrics,
    llm: LlmMetrics,
}

#[derive(Debug)]
struct App {
    endpoint: String,
    token: Option<String>,
    show_help: bool,
    expanded_events: bool,
    should_quit: bool,
    state: Option<AgentState>,
    last_error: Option<String>,
    last_fetch: Option<Instant>,
}

impl App {
    fn new(endpoint: String, token: Option<String>) -> Self {
        Self {
            endpoint,
            token,
            show_help: false,
            expanded_events: false,
            should_quit: false,
            state: None,
            last_error: None,
            last_fetch: None,
        }
    }

    fn on_key(&mut self, key: KeyCode) -> bool {
        match key {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('h') => self.show_help = !self.show_help,
            KeyCode::Char('e') => self.expanded_events = !self.expanded_events,
            KeyCode::Char('r') => return true,
            _ => {}
        }
        false
    }
}

fn main() -> Result<()> {
    let endpoint =
        env::var("AGENT_ENDPOINT").unwrap_or_else(|_| "http://127.0.0.1:8787/state".to_string());
    let token = env::var("AGENT_TOKEN").ok().filter(|v| !v.is_empty());

    run_app(App::new(endpoint, token))
}

fn run_app(mut app: App) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let client = Client::builder()
        .timeout(Duration::from_millis(800))
        .build()?;
    let tick = Duration::from_millis(250);
    let poll_every = Duration::from_secs(1);
    let mut last_poll = Instant::now() - poll_every;

    loop {
        if last_poll.elapsed() >= poll_every {
            poll_state(&client, &mut app);
            last_poll = Instant::now();
        }

        terminal.draw(|frame| ui(frame, &app))?;

        if event::poll(tick)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    let manual_refresh = app.on_key(key.code);
                    if manual_refresh {
                        poll_state(&client, &mut app);
                        last_poll = Instant::now();
                    }
                }
            }
        }

        if app.should_quit {
            break;
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn poll_state(client: &Client, app: &mut App) {
    let mut request = client.get(&app.endpoint);
    if let Some(token) = &app.token {
        request = request.header("x-agent-token", token);
    }

    match request.send() {
        Ok(resp) if resp.status().is_success() => match resp.json::<AgentState>() {
            Ok(state) => {
                app.state = Some(state);
                app.last_error = None;
                app.last_fetch = Some(Instant::now());
            }
            Err(err) => app.last_error = Some(format!("decode failed: {err}")),
        },
        Ok(resp) => app.last_error = Some(format!("agent returned status {}", resp.status())),
        Err(err) => app.last_error = Some(format!("request failed: {err}")),
    }
}

fn ui(frame: &mut ratatui::Frame, app: &App) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(8),
            Constraint::Length(1),
        ])
        .split(frame.area());

    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            " pro-tui ",
            Style::default().fg(Color::Black).bg(Color::Green),
        ),
        Span::raw(" Mac Pro dashboard for Mac mini agent"),
    ]));
    frame.render_widget(title, outer[0]);

    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(outer[1]);

    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(body[0]);

    let bottom = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(body[1]);

    frame.render_widget(
        Paragraph::new(status_lines(app))
            .block(block("Status"))
            .wrap(Wrap { trim: true }),
        top[0],
    );
    frame.render_widget(
        Paragraph::new(system_lines(app))
            .block(block("System"))
            .wrap(Wrap { trim: true }),
        top[1],
    );
    frame.render_widget(
        Paragraph::new(llm_lines(app))
            .block(block("LLM"))
            .wrap(Wrap { trim: true }),
        top[2],
    );
    frame.render_widget(
        Paragraph::new(process_lines(app))
            .block(block("Top Processes"))
            .wrap(Wrap { trim: true }),
        bottom[0],
    );
    frame.render_widget(
        Paragraph::new(event_lines(app))
            .block(block("File Events"))
            .wrap(Wrap { trim: true }),
        bottom[1],
    );

    let footer = Paragraph::new(Line::from(vec![
        Span::styled("q", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" quit  "),
        Span::styled("r", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" refresh  "),
        Span::styled("e", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" expand events  "),
        Span::styled("h", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" help"),
    ]));
    frame.render_widget(footer, outer[2]);

    if app.show_help {
        let popup = centered_rect(70, 65, frame.area());
        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(vec![
                Line::raw("pro-tui help"),
                Line::raw(""),
                Line::raw("Keybinds"),
                Line::raw("q  quit"),
                Line::raw("r  refresh immediately"),
                Line::raw("e  expand/collapse file events"),
                Line::raw("h  show/hide help"),
                Line::raw(""),
                Line::raw("Env"),
                Line::raw("AGENT_ENDPOINT  http://mini:8787/state"),
                Line::raw("AGENT_TOKEN     optional auth token"),
            ])
            .block(block("Help"))
            .wrap(Wrap { trim: true }),
            popup,
        );
    }
}

fn block(title: &str) -> Block<'_> {
    Block::default().borders(Borders::ALL).title(Span::styled(
        format!(" {} ", title),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    ))
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1]);

    horizontal[1]
}

fn status_lines(app: &App) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::raw(format!("endpoint: {}", app.endpoint)),
        Line::raw(format!(
            "last fetch: {}",
            app.last_fetch
                .map(|t| format!("{} ms ago", t.elapsed().as_millis()))
                .unwrap_or_else(|| "never".to_string())
        )),
    ];

    if let Some(state) = &app.state {
        lines.push(Line::raw(format!("host: {}", state.hostname)));
        lines.push(Line::raw(format!("seq: {}", state.seq)));
        lines.push(Line::raw(format!("ts_ms: {}", state.ts_ms)));
    } else {
        lines.push(Line::raw("host: (no data)".to_string()));
    }

    match &app.last_error {
        Some(err) => lines.push(Line::raw(format!("error: {err}"))),
        None => lines.push(Line::raw("error: none".to_string())),
    }

    lines
}

fn system_lines(app: &App) -> Vec<Line<'static>> {
    let Some(state) = &app.state else {
        return vec![Line::raw("waiting for first snapshot...".to_string())];
    };

    let s = &state.system;
    vec![
        Line::raw(format!("cpu: {:.1}%", s.cpu_percent)),
        Line::raw(format!(
            "mem: {} / {}",
            bytes_human(s.memory_used_bytes),
            bytes_human(s.memory_total_bytes)
        )),
        Line::raw(format!(
            "swap: {} / {}",
            bytes_human(s.swap_used_bytes),
            bytes_human(s.swap_total_bytes)
        )),
        Line::raw(format!(
            "root: {} / {}",
            bytes_human(s.root_used_bytes),
            bytes_human(s.root_total_bytes)
        )),
    ]
}

fn llm_lines(app: &App) -> Vec<Line<'static>> {
    let Some(state) = &app.state else {
        return vec![Line::raw("waiting for first snapshot...".to_string())];
    };

    let llm = &state.llm;
    let mut lines = vec![
        Line::raw(format!("ollama endpoint: {}", llm.ollama_ps_url)),
        Line::raw(format!("online: {}", llm.ollama_online)),
        Line::raw(format!("running models: {}", llm.model_count)),
    ];

    if llm.running_models.is_empty() {
        lines.push(Line::raw("models: (none)".to_string()));
    } else {
        lines.extend(
            llm.running_models
                .iter()
                .take(6)
                .map(|m| Line::raw(format!("- {m}"))),
        );
    }

    if let Some(err) = &llm.error {
        lines.push(Line::raw(format!("error: {err}")));
    }

    lines
}

fn process_lines(app: &App) -> Vec<Line<'static>> {
    let Some(state) = &app.state else {
        return vec![Line::raw("waiting for first snapshot...".to_string())];
    };

    if state.system.top_processes.is_empty() {
        return vec![Line::raw("no process data".to_string())];
    }

    state
        .system
        .top_processes
        .iter()
        .map(|p| {
            Line::raw(format!(
                "pid={} cpu={:.1}% mem={} {}",
                p.pid,
                p.cpu_percent,
                bytes_human(p.memory_bytes),
                p.name
            ))
        })
        .collect()
}

fn event_lines(app: &App) -> Vec<Line<'static>> {
    let Some(state) = &app.state else {
        return vec![Line::raw("waiting for first snapshot...".to_string())];
    };

    let mut lines = vec![];
    lines.push(Line::raw(format!(
        "watching: {}",
        if state.watched_dirs.is_empty() {
            "(none)".to_string()
        } else {
            state.watched_dirs.join(", ")
        }
    )));

    let limit = if app.expanded_events { 20 } else { 8 };
    if state.recent_file_events.is_empty() {
        lines.push(Line::raw("no recent file events".to_string()));
    } else {
        lines.extend(
            state
                .recent_file_events
                .iter()
                .take(limit)
                .map(|e| Line::raw(e.clone())),
        );
    }

    lines
}

fn bytes_human(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;

    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{} B", bytes)
    }
}
