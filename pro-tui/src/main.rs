use std::collections::VecDeque;
use std::env;
use std::io;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
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
use serde::{Deserialize, Serialize};

const CHAT_HISTORY_MAX: usize = 100;

#[derive(Debug, Clone, Default, Deserialize)]
struct ProcessInfo {
    pid: u32,
    name: String,
    cpu_percent: f32,
    #[serde(default)]
    current_cpu_percent: f32,
    memory_bytes: u64,
    #[serde(default)]
    samples_5m: u32,
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

#[derive(Debug, Serialize)]
struct ChatRequest {
    prompt: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    response: String,
    model: String,
    error: Option<String>,
}

#[derive(Debug, Clone)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug)]
enum AppAction {
    None,
    Refresh,
    SendPrompt(String),
}

#[derive(Debug)]
struct App {
    endpoint: String,
    token: Option<String>,
    show_help: bool,
    expanded_events: bool,
    input_mode: bool,
    chat_input: String,
    chat_history: VecDeque<ChatMessage>,
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
            input_mode: false,
            chat_input: String::new(),
            chat_history: VecDeque::new(),
            should_quit: false,
            state: None,
            last_error: None,
            last_fetch: None,
        }
    }

    fn on_key(&mut self, key: KeyEvent) -> AppAction {
        if self.input_mode {
            return self.on_key_input_mode(key);
        }

        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('h') => self.show_help = !self.show_help,
            KeyCode::Char('e') => self.expanded_events = !self.expanded_events,
            KeyCode::Char('i') => self.input_mode = true,
            KeyCode::Char('r') => return AppAction::Refresh,
            _ => {}
        }

        AppAction::None
    }

    fn on_key_input_mode(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Esc => self.input_mode = false,
            KeyCode::Enter => {
                let prompt = self.chat_input.trim().to_string();
                self.chat_input.clear();
                if !prompt.is_empty() {
                    return AppAction::SendPrompt(prompt);
                }
            }
            KeyCode::Backspace => {
                self.chat_input.pop();
            }
            KeyCode::Char(c) => {
                if !key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.chat_input.push(c);
                }
            }
            _ => {}
        }

        AppAction::None
    }

    fn push_chat(&mut self, role: impl Into<String>, content: impl Into<String>) {
        self.chat_history.push_back(ChatMessage {
            role: role.into(),
            content: content.into(),
        });
        while self.chat_history.len() > CHAT_HISTORY_MAX {
            self.chat_history.pop_front();
        }
    }

    fn chat_endpoint(&self) -> String {
        if let Some(prefix) = self.endpoint.strip_suffix("/state") {
            format!("{prefix}/chat")
        } else {
            format!("{}/chat", self.endpoint.trim_end_matches('/'))
        }
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

    let client = Client::builder().timeout(Duration::from_secs(20)).build()?;
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
                    match app.on_key(key) {
                        AppAction::None => {}
                        AppAction::Refresh => {
                            poll_state(&client, &mut app);
                            last_poll = Instant::now();
                        }
                        AppAction::SendPrompt(prompt) => {
                            send_prompt(&client, &mut app, prompt);
                            poll_state(&client, &mut app);
                            last_poll = Instant::now();
                        }
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

fn send_prompt(client: &Client, app: &mut App, prompt: String) {
    app.push_chat("you", prompt.clone());

    let mut request = client
        .post(app.chat_endpoint())
        .json(&ChatRequest { prompt });
    if let Some(token) = &app.token {
        request = request.header("x-agent-token", token);
    }

    match request.send() {
        Ok(resp) if resp.status().is_success() => match resp.json::<ChatResponse>() {
            Ok(chat) => {
                if let Some(err) = chat.error {
                    app.push_chat("error", err);
                } else if chat.response.trim().is_empty() {
                    app.push_chat(
                        "assistant",
                        format!("({} returned empty response)", chat.model),
                    );
                } else {
                    app.push_chat("assistant", chat.response);
                }
            }
            Err(err) => app.push_chat("error", format!("decode failed: {err}")),
        },
        Ok(resp) => app.push_chat("error", format!("chat status {}", resp.status())),
        Err(err) => app.push_chat("error", format!("chat request failed: {err}")),
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
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(outer[1]);

    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(75), Constraint::Percentage(25)])
        .split(body[0]);

    let top_left = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(top[0]);

    let bottom = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(body[1]);

    frame.render_widget(
        Paragraph::new(status_lines(app))
            .block(block("Status"))
            .wrap(Wrap { trim: true }),
        top_left[0],
    );
    frame.render_widget(
        Paragraph::new(system_lines(app))
            .block(block("System"))
            .wrap(Wrap { trim: true }),
        top_left[1],
    );
    frame.render_widget(
        Paragraph::new(process_lines(app))
            .block(block("Top Processes"))
            .wrap(Wrap { trim: true }),
        top[1],
    );
    render_llm_panel(frame, app, bottom[0]);
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
        Span::styled("i", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" chat input  "),
        Span::styled("h", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" help"),
    ]));
    frame.render_widget(footer, outer[2]);

    if app.show_help {
        let popup = centered_rect(75, 72, frame.area());
        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(vec![
                Line::raw("pro-tui help"),
                Line::raw(""),
                Line::raw("General"),
                Line::raw("q  quit"),
                Line::raw("r  refresh immediately"),
                Line::raw("e  expand/collapse file events"),
                Line::raw("h  show/hide help"),
                Line::raw(""),
                Line::raw("LLM Chat"),
                Line::raw("i        focus chat input"),
                Line::raw("Enter    send prompt (when input focused)"),
                Line::raw("Esc      exit input focus"),
                Line::raw("Backspace edit input"),
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

fn render_llm_panel(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let outer = block("LLM").inner(area);
    frame.render_widget(block("LLM"), area);

    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .split(outer);

    let llm_summary = Paragraph::new(llm_lines(app)).wrap(Wrap { trim: true });
    frame.render_widget(llm_summary, parts[0]);

    let chat_window = Paragraph::new(chat_lines(app))
        .block(block("Chat"))
        .wrap(Wrap { trim: true });
    frame.render_widget(chat_window, parts[1]);

    let input_title = if app.input_mode {
        "Input (focused: Enter send, Esc stop)"
    } else {
        "Input (press i to focus)"
    };
    let input_style = if app.input_mode {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    let input = Paragraph::new(format!("> {}", app.chat_input))
        .block(Block::default().borders(Borders::ALL).title(input_title))
        .style(input_style)
        .wrap(Wrap { trim: true });
    frame.render_widget(input, parts[2]);
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

    if let Some(err) = &llm.error {
        lines.push(Line::raw(format!("error: {err}")));
    }

    lines
}

fn chat_lines(app: &App) -> Vec<Line<'static>> {
    if app.chat_history.is_empty() {
        return vec![Line::raw(
            "No chat messages yet. Press i to start typing.".to_string(),
        )];
    }

    app.chat_history
        .iter()
        .rev()
        .take(20)
        .rev()
        .map(|m| Line::raw(format!("{}: {}", m.role, m.content)))
        .collect()
}

fn process_lines(app: &App) -> Vec<Line<'static>> {
    let Some(state) = &app.state else {
        return vec![Line::raw("waiting for first snapshot...".to_string())];
    };

    if state.system.top_processes.is_empty() {
        return vec![Line::raw("no process data".to_string())];
    }

    let mut lines = vec![Line::raw("ranked by avg CPU over last 5m".to_string())];
    lines.extend(state.system.top_processes.iter().map(|p| {
        Line::raw(format!(
            "pid={} avg5m={:.1}% now={:.1}% n={} mem={} {}",
            p.pid,
            p.cpu_percent,
            p.current_cpu_percent,
            p.samples_5m,
            bytes_human(p.memory_bytes),
            p.name
        ))
    }));
    lines
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
