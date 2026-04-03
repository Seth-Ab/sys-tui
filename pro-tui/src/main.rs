use std::collections::{HashSet, VecDeque};
use std::env;
use std::io;
use std::sync::mpsc::{self, Receiver, Sender};
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
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    response: String,
    model: String,
    error: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct ModelsResponse {
    selected_model: String,
    running_models: Vec<String>,
    installed_models: Vec<String>,
    error: Option<String>,
}

#[derive(Debug, Clone)]
struct ChatMessage {
    id: Option<u64>,
    role: String,
    content: String,
}

#[derive(Debug)]
struct ChatJobResult {
    placeholder_id: u64,
    role: String,
    content: String,
}

#[derive(Debug)]
enum AppAction {
    None,
    Refresh,
    SendPrompt(String),
    OpenModelSelector,
    SelectModel(String),
}

#[derive(Debug)]
struct App {
    endpoint: String,
    token: Option<String>,
    show_help: bool,
    show_model_selector: bool,
    expanded_events: bool,
    input_mode: bool,
    chat_input: String,
    chat_history: VecDeque<ChatMessage>,
    chat_scroll: usize,
    next_chat_id: u64,
    selected_model: Option<String>,
    model_list: Vec<String>,
    running_models: Vec<String>,
    model_hover_idx: usize,
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
            show_model_selector: false,
            expanded_events: false,
            input_mode: false,
            chat_input: String::new(),
            chat_history: VecDeque::new(),
            chat_scroll: 0,
            next_chat_id: 1,
            selected_model: None,
            model_list: Vec::new(),
            running_models: Vec::new(),
            model_hover_idx: 0,
            should_quit: false,
            state: None,
            last_error: None,
            last_fetch: None,
        }
    }

    fn on_key(&mut self, key: KeyEvent) -> AppAction {
        if self.show_help {
            if matches!(key.code, KeyCode::Esc) {
                self.show_help = false;
            }
            return AppAction::None;
        }

        if self.show_model_selector {
            return self.on_key_model_selector(key);
        }

        if self.input_mode {
            return self.on_key_input_mode(key);
        }

        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('h') => self.show_help = true,
            KeyCode::Char('e') => self.expanded_events = !self.expanded_events,
            KeyCode::Char('i') => self.input_mode = true,
            KeyCode::Char('m') => return AppAction::OpenModelSelector,
            KeyCode::Char('r') => return AppAction::Refresh,
            KeyCode::Up => self.chat_scroll = self.chat_scroll.saturating_sub(1),
            KeyCode::Down => self.chat_scroll = self.chat_scroll.saturating_add(1),
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

    fn on_key_model_selector(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Esc => self.show_model_selector = false,
            KeyCode::Up => {
                if self.model_hover_idx > 0 {
                    self.model_hover_idx -= 1;
                }
            }
            KeyCode::Down => {
                if self.model_hover_idx + 1 < self.model_list.len() {
                    self.model_hover_idx += 1;
                }
            }
            KeyCode::Enter => {
                if let Some(model) = self.model_list.get(self.model_hover_idx).cloned() {
                    self.show_model_selector = false;
                    return AppAction::SelectModel(model);
                }
            }
            _ => {}
        }

        AppAction::None
    }

    fn push_chat(&mut self, role: impl Into<String>, content: impl Into<String>) {
        self.push_chat_with_id(None, role, content);
    }

    fn push_chat_with_id(
        &mut self,
        id: Option<u64>,
        role: impl Into<String>,
        content: impl Into<String>,
    ) {
        self.chat_history.push_back(ChatMessage {
            id,
            role: role.into(),
            content: content.into(),
        });
        while self.chat_history.len() > CHAT_HISTORY_MAX {
            self.chat_history.pop_front();
        }
    }

    fn replace_chat_by_id(&mut self, id: u64, role: impl Into<String>, content: impl Into<String>) {
        let role = role.into();
        let content = content.into();
        if let Some(msg) = self
            .chat_history
            .iter_mut()
            .rev()
            .find(|m| m.id == Some(id))
        {
            msg.id = None;
            msg.role = role;
            msg.content = content;
            return;
        }

        self.push_chat(role, content);
    }

    fn chat_endpoint(&self) -> String {
        if let Some(prefix) = self.endpoint.strip_suffix("/state") {
            format!("{prefix}/chat")
        } else {
            format!("{}/chat", self.endpoint.trim_end_matches('/'))
        }
    }

    fn models_endpoint(&self) -> String {
        if let Some(prefix) = self.endpoint.strip_suffix("/state") {
            format!("{prefix}/models")
        } else {
            format!("{}/models", self.endpoint.trim_end_matches('/'))
        }
    }

    fn effective_model(&self) -> Option<String> {
        if let Some(model) = &self.selected_model {
            return Some(model.clone());
        }
        self.state
            .as_ref()
            .and_then(|s| s.llm.running_models.first().cloned())
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
    let (chat_tx, chat_rx) = mpsc::channel::<ChatJobResult>();

    let tick = Duration::from_millis(250);
    let poll_every = Duration::from_secs(1);
    let mut last_poll = Instant::now() - poll_every;

    loop {
        drain_chat_results(&mut app, &chat_rx);

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
                            send_prompt(&client, &mut app, &chat_tx, prompt);
                        }
                        AppAction::OpenModelSelector => {
                            open_model_selector(&client, &mut app);
                        }
                        AppAction::SelectModel(model) => {
                            app.selected_model = Some(model.clone());
                            app.push_chat("system", format!("selected model: {model}"));
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

fn drain_chat_results(app: &mut App, rx: &Receiver<ChatJobResult>) {
    while let Ok(result) = rx.try_recv() {
        app.replace_chat_by_id(result.placeholder_id, result.role, result.content);
    }
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

fn open_model_selector(client: &Client, app: &mut App) {
    let mut request = client.get(app.models_endpoint());
    if let Some(token) = &app.token {
        request = request.header("x-agent-token", token);
    }

    match request.send() {
        Ok(resp) if resp.status().is_success() => match resp.json::<ModelsResponse>() {
            Ok(models) => {
                let mut ordered = Vec::new();

                for model in &models.running_models {
                    if !ordered.contains(model) {
                        ordered.push(model.clone());
                    }
                }
                for model in &models.installed_models {
                    if !ordered.contains(model) {
                        ordered.push(model.clone());
                    }
                }

                if ordered.is_empty() {
                    app.push_chat("error", "no models available from mini-agent /models");
                    return;
                }

                app.running_models = models.running_models;
                app.model_list = ordered;
                app.show_model_selector = true;

                let selected = app
                    .selected_model
                    .clone()
                    .or_else(|| {
                        (!models.selected_model.is_empty()).then_some(models.selected_model)
                    })
                    .or_else(|| app.effective_model());

                app.model_hover_idx = selected
                    .and_then(|sel| app.model_list.iter().position(|m| m == &sel))
                    .unwrap_or(0);

                if let Some(err) = models.error {
                    app.push_chat("system", format!("models warning: {err}"));
                }
            }
            Err(err) => app.push_chat("error", format!("models decode failed: {err}")),
        },
        Ok(resp) => app.push_chat("error", format!("models status {}", resp.status())),
        Err(err) => app.push_chat("error", format!("models request failed: {err}")),
    }
}

fn send_prompt(client: &Client, app: &mut App, tx: &Sender<ChatJobResult>, prompt: String) {
    app.push_chat("you", prompt.clone());

    let placeholder_id = app.next_chat_id;
    app.next_chat_id = app.next_chat_id.saturating_add(1);
    app.push_chat_with_id(Some(placeholder_id), "assistant", "*working*");

    let mut request = client.post(app.chat_endpoint()).json(&ChatRequest {
        prompt,
        model: app.selected_model.clone(),
    });
    if let Some(token) = &app.token {
        request = request.header("x-agent-token", token);
    }

    let tx = tx.clone();
    std::thread::spawn(move || {
        let result = match request.send() {
            Ok(resp) if resp.status().is_success() => match resp.json::<ChatResponse>() {
                Ok(chat) => {
                    if let Some(err) = chat.error {
                        ChatJobResult {
                            placeholder_id,
                            role: "error".to_string(),
                            content: err,
                        }
                    } else if chat.response.trim().is_empty() {
                        ChatJobResult {
                            placeholder_id,
                            role: "assistant".to_string(),
                            content: format!("({} returned empty response)", chat.model),
                        }
                    } else {
                        ChatJobResult {
                            placeholder_id,
                            role: "assistant".to_string(),
                            content: chat.response,
                        }
                    }
                }
                Err(err) => ChatJobResult {
                    placeholder_id,
                    role: "error".to_string(),
                    content: format!("decode failed: {err}"),
                },
            },
            Ok(resp) => ChatJobResult {
                placeholder_id,
                role: "error".to_string(),
                content: format!("chat status {}", resp.status()),
            },
            Err(err) => ChatJobResult {
                placeholder_id,
                role: "error".to_string(),
                content: format!("chat request failed: {err}"),
            },
        };

        let _ = tx.send(result);
    });
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
        Span::styled("m", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" models  "),
        Span::styled("h", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" help"),
    ]));
    frame.render_widget(footer, outer[2]);

    if app.show_help {
        render_help_popup(frame);
    }

    if app.show_model_selector {
        render_model_selector_popup(frame, app);
    }
}

fn render_help_popup(frame: &mut ratatui::Frame) {
    let popup = centered_rect(75, 74, frame.area());
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(vec![
            Line::raw("pro-tui help"),
            Line::raw(""),
            Line::raw("General"),
            Line::raw("q  quit"),
            Line::raw("r  refresh immediately"),
            Line::raw("e  expand/collapse file events"),
            Line::raw("h  open help"),
            Line::raw("Esc close help"),
            Line::raw(""),
            Line::raw("LLM Chat"),
            Line::raw("i        focus chat input"),
            Line::raw("Enter    send prompt (when input focused)"),
            Line::raw("Esc      exit input focus"),
            Line::raw("Backspace edit input"),
            Line::raw(""),
            Line::raw("Model Selector"),
            Line::raw("m        open model selector"),
            Line::raw("Up/Down  move selection"),
            Line::raw("Enter    apply model"),
            Line::raw("Esc      close selector"),
            Line::raw(""),
            Line::raw("Chat Scroll (normal mode)"),
            Line::raw("Up       scroll chat up"),
            Line::raw("Down     scroll chat down"),
        ])
        .block(block("Help"))
        .wrap(Wrap { trim: true }),
        popup,
    );
}

fn render_model_selector_popup(frame: &mut ratatui::Frame, app: &App) {
    let popup = centered_rect(65, 70, frame.area());
    frame.render_widget(Clear, popup);

    let running: HashSet<&str> = app.running_models.iter().map(String::as_str).collect();
    let effective = app.selected_model.as_deref();

    let mut lines = vec![
        Line::raw("Select model (Enter apply, Esc close)"),
        Line::raw(""),
    ];

    for (idx, model) in app.model_list.iter().enumerate() {
        let prefix = if idx == app.model_hover_idx { ">" } else { " " };
        let run_mark = if running.contains(model.as_str()) {
            "[running]"
        } else {
            "         "
        };
        let sel_mark = if effective == Some(model.as_str()) {
            "*"
        } else {
            " "
        };

        lines.push(Line::from(vec![
            Span::raw(format!("{prefix}{sel_mark} {run_mark} ")),
            if idx == app.model_hover_idx {
                Span::styled(
                    model.clone(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::raw(model.clone())
            },
        ]));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(block("Models"))
            .wrap(Wrap { trim: true }),
        popup,
    );
}

fn render_llm_panel(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let outer = block("LLM").inner(area);
    frame.render_widget(block("LLM"), area);

    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .split(outer);

    let llm_summary = Paragraph::new(llm_lines(app)).wrap(Wrap { trim: true });
    frame.render_widget(llm_summary, parts[0]);

    let chat_window = Paragraph::new(chat_lines(app))
        .block(block("Chat"))
        .scroll((app.chat_scroll.min(u16::MAX as usize) as u16, 0))
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
    let selected = app
        .selected_model
        .clone()
        .or_else(|| state.llm.running_models.first().cloned())
        .unwrap_or_else(|| "(none)".to_string());

    let mut lines = vec![
        Line::raw(format!("ollama endpoint: {}", llm.ollama_ps_url)),
        Line::raw(format!("online: {}", llm.ollama_online)),
        Line::raw(format!("running models: {}", llm.model_count)),
        Line::raw(format!("selected chat model: {selected}")),
    ];

    if let Some(err) = &llm.error {
        lines.push(Line::raw(format!("error: {err}")));
    }

    lines
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChatLineKind {
    Plain,
    Code,
    CodeLang,
}

#[derive(Debug, Clone)]
struct ChatLine {
    text: String,
    kind: ChatLineKind,
}

fn chat_lines(app: &App) -> Vec<Line<'static>> {
    if app.chat_history.is_empty() {
        return vec![Line::raw(
            "No chat messages yet. Press i to type, m to choose model.".to_string(),
        )];
    }

    let mut out = Vec::new();
    for msg in app.chat_history.iter().rev().take(20).rev() {
        let formatted = format_markdown_for_chat(&msg.content);
        if formatted.is_empty() {
            out.push(Line::raw(format!("{}:", msg.role)));
            continue;
        }

        for (idx, line) in formatted.iter().enumerate() {
            let prefix = if idx == 0 {
                format!("{}: ", msg.role)
            } else {
                "    ".to_string()
            };

            let styled = match line.kind {
                ChatLineKind::Plain => Line::raw(format!("{}{}", prefix, line.text)),
                ChatLineKind::Code => Line::from(vec![
                    Span::raw(prefix),
                    Span::styled(line.text.clone(), Style::default().fg(Color::Cyan)),
                ]),
                ChatLineKind::CodeLang => Line::from(vec![
                    Span::raw(prefix),
                    Span::styled(
                        line.text.clone(),
                        Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
            };
            out.push(styled);
        }
    }

    out
}

fn format_markdown_for_chat(text: &str) -> Vec<ChatLine> {
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut in_code = false;

    while i < text.len() {
        let rem = &text[i..];
        if let Some(rel) = rem.find("```") {
            let pos = i + rel;
            let seg = &text[i..pos];
            append_segment(seg, in_code, &mut out);

            i = pos + 3;
            if in_code {
                in_code = false;
                out.push(ChatLine {
                    text: String::new(),
                    kind: ChatLineKind::Plain,
                });
            } else {
                let (lang, consumed) = parse_fence_lang(&text[i..]);
                i += consumed;
                in_code = true;
                out.push(ChatLine {
                    text: String::new(),
                    kind: ChatLineKind::Plain,
                });
                if let Some(lang) = lang {
                    out.push(ChatLine {
                        text: format!("[{}]", lang),
                        kind: ChatLineKind::CodeLang,
                    });
                }
            }
        } else {
            let seg = &text[i..];
            append_segment(seg, in_code, &mut out);
            break;
        }
    }

    while out
        .first()
        .is_some_and(|l| l.text.is_empty() && l.kind == ChatLineKind::Plain)
    {
        out.remove(0);
    }
    while out
        .last()
        .is_some_and(|l| l.text.is_empty() && l.kind == ChatLineKind::Plain)
    {
        out.pop();
    }

    out
}

fn parse_fence_lang(input: &str) -> (Option<String>, usize) {
    if input.is_empty() {
        return (None, 0);
    }

    let mut lang = String::new();
    let mut consumed = 0usize;

    for ch in input.chars() {
        if ch == '\n' {
            consumed += ch.len_utf8();
            break;
        }
        if ch.is_whitespace() {
            consumed += ch.len_utf8();
            break;
        }
        lang.push(ch);
        consumed += ch.len_utf8();
    }

    let lang = if lang.is_empty() { None } else { Some(lang) };
    (lang, consumed)
}

fn append_segment(seg: &str, in_code: bool, out: &mut Vec<ChatLine>) {
    if seg.is_empty() {
        return;
    }

    for line in seg.split('\n') {
        if in_code {
            out.push(ChatLine {
                text: format!("  | {}", line),
                kind: ChatLineKind::Code,
            });
        } else {
            out.push(ChatLine {
                text: line.to_string(),
                kind: ChatLineKind::Plain,
            });
        }
    }
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
