use std::collections::{BTreeMap, HashSet, VecDeque};
use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
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
const STATUS_MSG_TTL: Duration = Duration::from_secs(5);

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct AppConfig {
    global: GlobalConfig,
    dashboard: DashboardConfig,
    #[serde(flatten)]
    extra: BTreeMap<String, toml::Value>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            global: GlobalConfig::default(),
            dashboard: DashboardConfig::default(),
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct GlobalConfig {
    theme: String,
    #[serde(flatten)]
    extra: BTreeMap<String, toml::Value>,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            theme: "default".to_string(),
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct DashboardConfig {
    assistant_name: String,
    assistant_color: String,
    user_name: String,
    user_color: String,
    layout_preset: DashboardLayoutPreset,
    show_status: bool,
    show_system: bool,
    show_processes: bool,
    show_events: bool,
    #[serde(flatten)]
    extra: BTreeMap<String, toml::Value>,
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            assistant_name: "assistant".to_string(),
            assistant_color: "cyan".to_string(),
            user_name: "you".to_string(),
            user_color: "green".to_string(),
            layout_preset: DashboardLayoutPreset::ThreeTopTwoBottom,
            show_status: true,
            show_system: true,
            show_processes: true,
            show_events: true,
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum DashboardLayoutPreset {
    ThreeTopTwoBottom,
    LlmColumn,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Dashboard,
    Customize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CustomizeSection {
    Global,
    Dashboard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GlobalOption {
    Theme,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DashboardOption {
    AssistantName,
    AssistantColor,
    UserName,
    UserColor,
    LayoutPreset,
    ShowStatus,
    ShowSystem,
    ShowProcesses,
    ShowEvents,
}

#[derive(Debug)]
enum AppAction {
    None,
    Refresh,
    SendPrompt(String),
    OpenModelSelector,
    SelectModel(String),
    SelectScreen(Screen),
    SaveConfig,
}

#[derive(Debug)]
struct StatusMessage {
    text: String,
    at: Instant,
}

#[derive(Debug)]
struct App {
    endpoint: String,
    token: Option<String>,
    config: AppConfig,
    config_path: PathBuf,
    config_dirty: bool,
    status_msg: Option<StatusMessage>,
    active_screen: Screen,
    show_help: bool,
    show_model_selector: bool,
    show_navigator: bool,
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
    navigator_hover_idx: usize,
    customize_section_idx: usize,
    global_option_idx: usize,
    dashboard_option_idx: usize,
    customize_text_mode: bool,
    customize_text_buffer: String,
    should_quit: bool,
    state: Option<AgentState>,
    last_error: Option<String>,
    last_fetch: Option<Instant>,
}

impl App {
    fn new(
        endpoint: String,
        token: Option<String>,
        config_path: PathBuf,
        config: AppConfig,
    ) -> Self {
        Self {
            endpoint,
            token,
            config,
            config_path,
            config_dirty: false,
            status_msg: None,
            active_screen: Screen::Dashboard,
            show_help: false,
            show_model_selector: false,
            show_navigator: false,
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
            navigator_hover_idx: 0,
            customize_section_idx: 0,
            global_option_idx: 0,
            dashboard_option_idx: 0,
            customize_text_mode: false,
            customize_text_buffer: String::new(),
            should_quit: false,
            state: None,
            last_error: None,
            last_fetch: None,
        }
    }

    fn on_key(&mut self, key: KeyEvent) -> AppAction {
        if matches!(key.code, KeyCode::Char('q')) {
            self.should_quit = true;
            return AppAction::None;
        }

        // Global screen navigator is always available.
        if matches!(key.code, KeyCode::Tab) {
            self.show_help = false;
            self.show_model_selector = false;
            self.show_navigator = true;
            self.input_mode = false;
            self.customize_text_mode = false;
            self.navigator_hover_idx = Self::screen_index(self.active_screen);
            return AppAction::None;
        }

        if matches!(key.code, KeyCode::Esc) {
            if self.show_help {
                self.show_help = false;
                return AppAction::None;
            }
            if self.show_model_selector {
                self.show_model_selector = false;
                return AppAction::None;
            }
            if self.show_navigator {
                self.show_navigator = false;
                return AppAction::None;
            }
            if self.input_mode {
                self.input_mode = false;
                return AppAction::None;
            }
            if self.customize_text_mode {
                self.customize_text_mode = false;
                self.customize_text_buffer.clear();
                return AppAction::None;
            }
        }

        if self.show_help {
            return AppAction::None;
        }

        if self.show_navigator {
            return self.on_key_navigator(key);
        }

        if self.show_model_selector {
            return self.on_key_model_selector(key);
        }

        if self.input_mode {
            return self.on_key_input_mode(key);
        }

        if self.customize_text_mode {
            return self.on_key_customize_text_mode(key);
        }

        if matches!(key.code, KeyCode::Char('h')) {
            self.show_help = true;
            return AppAction::None;
        }

        if self.active_screen == Screen::Customize {
            return self.on_key_customize(key);
        }

        match key.code {
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

    fn on_key_navigator(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Esc => self.show_navigator = false,
            KeyCode::Up => {
                if self.navigator_hover_idx > 0 {
                    self.navigator_hover_idx -= 1;
                }
            }
            KeyCode::Down => {
                if self.navigator_hover_idx + 1 < Self::screens().len() {
                    self.navigator_hover_idx += 1;
                }
            }
            KeyCode::Enter => {
                self.show_navigator = false;
                return AppAction::SelectScreen(Self::screen_from_index(self.navigator_hover_idx));
            }
            _ => {}
        }

        AppAction::None
    }

    fn on_key_customize(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Char('[') => self.prev_customize_section(),
            KeyCode::Char(']') => self.next_customize_section(),
            KeyCode::Up => self.move_customize_option(-1),
            KeyCode::Down => self.move_customize_option(1),
            KeyCode::Left => self.adjust_selected_customize_value(-1),
            KeyCode::Right => self.adjust_selected_customize_value(1),
            KeyCode::Char('s') => return AppAction::SaveConfig,
            KeyCode::Char('r') => self.reset_selected_customize_option(),
            KeyCode::Enter => self.enter_or_apply_selected_customize_option(),
            _ => {}
        }

        AppAction::None
    }

    fn on_key_customize_text_mode(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Esc => {
                self.customize_text_mode = false;
                self.customize_text_buffer.clear();
            }
            KeyCode::Backspace => {
                self.customize_text_buffer.pop();
            }
            KeyCode::Enter => {
                let value = self.customize_text_buffer.trim().to_string();
                self.apply_selected_text_value(value);
                self.customize_text_mode = false;
                self.customize_text_buffer.clear();
            }
            KeyCode::Char(c) => {
                if !key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.customize_text_buffer.push(c);
                }
            }
            _ => {}
        }

        AppAction::None
    }

    fn prev_customize_section(&mut self) {
        if self.customize_section_idx > 0 {
            self.customize_section_idx -= 1;
        }
    }

    fn next_customize_section(&mut self) {
        if self.customize_section_idx + 1 < Self::customize_sections().len() {
            self.customize_section_idx += 1;
        }
    }

    fn move_customize_option(&mut self, delta: i32) {
        match self.active_customize_section() {
            CustomizeSection::Global => {
                let max = Self::global_options().len().saturating_sub(1);
                self.global_option_idx = step_index(self.global_option_idx, max, delta);
            }
            CustomizeSection::Dashboard => {
                let max = Self::dashboard_options().len().saturating_sub(1);
                self.dashboard_option_idx = step_index(self.dashboard_option_idx, max, delta);
            }
        }
    }

    fn adjust_selected_customize_value(&mut self, delta: i32) {
        match self.active_customize_section() {
            CustomizeSection::Global => {
                if self.active_global_option() == GlobalOption::Theme {
                    self.config.global.theme = rotate_theme(&self.config.global.theme, delta);
                    self.config_dirty = true;
                }
            }
            CustomizeSection::Dashboard => match self.active_dashboard_option() {
                DashboardOption::AssistantColor => {
                    self.config.dashboard.assistant_color =
                        rotate_color_name(&self.config.dashboard.assistant_color, delta);
                    self.config_dirty = true;
                }
                DashboardOption::UserColor => {
                    self.config.dashboard.user_color =
                        rotate_color_name(&self.config.dashboard.user_color, delta);
                    self.config_dirty = true;
                }
                DashboardOption::LayoutPreset => {
                    self.config.dashboard.layout_preset =
                        rotate_layout(self.config.dashboard.layout_preset, delta);
                    self.config_dirty = true;
                }
                DashboardOption::ShowStatus => {
                    self.config.dashboard.show_status = !self.config.dashboard.show_status;
                    self.config_dirty = true;
                }
                DashboardOption::ShowSystem => {
                    self.config.dashboard.show_system = !self.config.dashboard.show_system;
                    self.config_dirty = true;
                }
                DashboardOption::ShowProcesses => {
                    self.config.dashboard.show_processes = !self.config.dashboard.show_processes;
                    self.config_dirty = true;
                }
                DashboardOption::ShowEvents => {
                    self.config.dashboard.show_events = !self.config.dashboard.show_events;
                    self.config_dirty = true;
                }
                DashboardOption::AssistantName | DashboardOption::UserName => {}
            },
        }
    }

    fn reset_selected_customize_option(&mut self) {
        let defaults = AppConfig::default();

        match self.active_customize_section() {
            CustomizeSection::Global => {
                if self.active_global_option() == GlobalOption::Theme {
                    self.config.global.theme = defaults.global.theme;
                    self.config_dirty = true;
                }
            }
            CustomizeSection::Dashboard => match self.active_dashboard_option() {
                DashboardOption::AssistantName => {
                    self.config.dashboard.assistant_name = defaults.dashboard.assistant_name
                }
                DashboardOption::AssistantColor => {
                    self.config.dashboard.assistant_color = defaults.dashboard.assistant_color
                }
                DashboardOption::UserName => {
                    self.config.dashboard.user_name = defaults.dashboard.user_name
                }
                DashboardOption::UserColor => {
                    self.config.dashboard.user_color = defaults.dashboard.user_color
                }
                DashboardOption::LayoutPreset => {
                    self.config.dashboard.layout_preset = defaults.dashboard.layout_preset
                }
                DashboardOption::ShowStatus => {
                    self.config.dashboard.show_status = defaults.dashboard.show_status
                }
                DashboardOption::ShowSystem => {
                    self.config.dashboard.show_system = defaults.dashboard.show_system
                }
                DashboardOption::ShowProcesses => {
                    self.config.dashboard.show_processes = defaults.dashboard.show_processes
                }
                DashboardOption::ShowEvents => {
                    self.config.dashboard.show_events = defaults.dashboard.show_events
                }
            },
        }

        self.config_dirty = true;
    }

    fn enter_or_apply_selected_customize_option(&mut self) {
        match self.active_customize_section() {
            CustomizeSection::Global => {
                if self.active_global_option() == GlobalOption::Theme {
                    self.config.global.theme = rotate_theme(&self.config.global.theme, 1);
                    self.config_dirty = true;
                }
            }
            CustomizeSection::Dashboard => match self.active_dashboard_option() {
                DashboardOption::AssistantName => {
                    self.customize_text_mode = true;
                    self.customize_text_buffer = self.config.dashboard.assistant_name.clone();
                }
                DashboardOption::UserName => {
                    self.customize_text_mode = true;
                    self.customize_text_buffer = self.config.dashboard.user_name.clone();
                }
                _ => self.adjust_selected_customize_value(1),
            },
        }
    }

    fn apply_selected_text_value(&mut self, value: String) {
        if value.is_empty() {
            return;
        }

        if self.active_customize_section() == CustomizeSection::Dashboard {
            match self.active_dashboard_option() {
                DashboardOption::AssistantName => {
                    self.config.dashboard.assistant_name = value;
                    self.config_dirty = true;
                }
                DashboardOption::UserName => {
                    self.config.dashboard.user_name = value;
                    self.config_dirty = true;
                }
                _ => {}
            }
        }
    }

    fn active_customize_section(&self) -> CustomizeSection {
        Self::customize_sections()[self
            .customize_section_idx
            .min(Self::customize_sections().len().saturating_sub(1))]
    }

    fn active_global_option(&self) -> GlobalOption {
        Self::global_options()[self
            .global_option_idx
            .min(Self::global_options().len().saturating_sub(1))]
    }

    fn active_dashboard_option(&self) -> DashboardOption {
        Self::dashboard_options()[self
            .dashboard_option_idx
            .min(Self::dashboard_options().len().saturating_sub(1))]
    }

    fn screen_index(screen: Screen) -> usize {
        match screen {
            Screen::Dashboard => 0,
            Screen::Customize => 1,
        }
    }

    fn screen_from_index(idx: usize) -> Screen {
        match idx {
            1 => Screen::Customize,
            _ => Screen::Dashboard,
        }
    }

    fn screen_name(screen: Screen) -> &'static str {
        match screen {
            Screen::Dashboard => "Dashboard",
            Screen::Customize => "Customize",
        }
    }

    fn screens() -> [Screen; 2] {
        [Screen::Dashboard, Screen::Customize]
    }

    fn customize_sections() -> [CustomizeSection; 2] {
        [CustomizeSection::Global, CustomizeSection::Dashboard]
    }

    fn global_options() -> [GlobalOption; 1] {
        [GlobalOption::Theme]
    }

    fn dashboard_options() -> [DashboardOption; 9] {
        [
            DashboardOption::AssistantName,
            DashboardOption::AssistantColor,
            DashboardOption::UserName,
            DashboardOption::UserColor,
            DashboardOption::LayoutPreset,
            DashboardOption::ShowStatus,
            DashboardOption::ShowSystem,
            DashboardOption::ShowProcesses,
            DashboardOption::ShowEvents,
        ]
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

    fn set_status_message(&mut self, text: impl Into<String>) {
        self.status_msg = Some(StatusMessage {
            text: text.into(),
            at: Instant::now(),
        });
    }

    fn visible_status_message(&self) -> Option<&str> {
        self.status_msg
            .as_ref()
            .filter(|m| m.at.elapsed() <= STATUS_MSG_TTL)
            .map(|m| m.text.as_str())
    }
}

fn main() -> Result<()> {
    let endpoint =
        env::var("AGENT_ENDPOINT").unwrap_or_else(|_| "http://127.0.0.1:8787/state".to_string());
    let token = env::var("AGENT_TOKEN").ok().filter(|v| !v.is_empty());

    let config_path = resolve_config_path()?;
    let (config, load_note) = load_config(&config_path);
    let mut app = App::new(endpoint, token, config_path, config);

    if let Some(note) = load_note {
        app.set_status_message(note);
    }

    run_app(app)
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
                        AppAction::SelectScreen(screen) => {
                            app.active_screen = screen;
                            app.show_help = false;
                            app.show_model_selector = false;
                            app.show_navigator = false;
                            app.input_mode = false;
                            app.customize_text_mode = false;
                            app.customize_text_buffer.clear();
                        }
                        AppAction::SaveConfig => {
                            let (normalized, warnings) = normalize_config(app.config.clone());
                            app.config = normalized;

                            match save_config(&app.config_path, &app.config) {
                                Ok(()) => {
                                    app.config_dirty = false;
                                    if warnings.is_empty() {
                                        app.set_status_message(format!(
                                            "saved config: {}",
                                            app.config_path.display()
                                        ));
                                    } else {
                                        app.set_status_message(format!(
                                            "saved with normalization: {}",
                                            warnings.join("; ")
                                        ));
                                    }
                                }
                                Err(err) => {
                                    app.set_status_message(format!("save failed: {err}"));
                                }
                            }
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

fn resolve_config_path() -> Result<PathBuf> {
    if let Ok(path) = env::var("PRO_TUI_CONFIG") {
        return Ok(PathBuf::from(path));
    }

    let home = env::var("HOME").context("HOME is not set; cannot resolve config path")?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("pro-tui")
        .join("config.toml"))
}

// Config loading is forgiving: invalid/missing files fall back to defaults.
fn load_config(path: &PathBuf) -> (AppConfig, Option<String>) {
    let raw = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return (AppConfig::default(), None),
        Err(err) => {
            return (
                AppConfig::default(),
                Some(format!("config read failed, using defaults: {err}")),
            );
        }
    };

    let parsed = match toml::from_str::<AppConfig>(&raw) {
        Ok(cfg) => cfg,
        Err(err) => {
            return (
                AppConfig::default(),
                Some(format!("config parse failed, using defaults: {err}")),
            );
        }
    };

    let (cfg, warnings) = normalize_config(parsed);
    if warnings.is_empty() {
        (cfg, None)
    } else {
        (
            cfg,
            Some(format!("config normalized: {}", warnings.join("; "))),
        )
    }
}

// Save uses a temp file then rename to avoid partially written config files.
fn save_config(path: &PathBuf, cfg: &AppConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating config dir {}", parent.display()))?;
    }

    let body = toml::to_string_pretty(cfg)?;
    let tmp_path = path.with_extension("toml.tmp");
    fs::write(&tmp_path, body)
        .with_context(|| format!("writing temp config {}", tmp_path.display()))?;
    fs::rename(&tmp_path, path)
        .with_context(|| format!("renaming temp config to {}", path.display()))?;
    Ok(())
}

fn normalize_config(mut cfg: AppConfig) -> (AppConfig, Vec<String>) {
    let mut warnings = Vec::new();

    if !theme_names().contains(&cfg.global.theme.as_str()) {
        warnings.push(format!("theme '{}' invalid -> default", cfg.global.theme));
        cfg.global.theme = GlobalConfig::default().theme;
    }

    if color_from_name(&cfg.dashboard.assistant_color).is_none() {
        warnings.push(format!(
            "assistant_color '{}' invalid -> default",
            cfg.dashboard.assistant_color
        ));
        cfg.dashboard.assistant_color = DashboardConfig::default().assistant_color;
    }

    if color_from_name(&cfg.dashboard.user_color).is_none() {
        warnings.push(format!(
            "user_color '{}' invalid -> default",
            cfg.dashboard.user_color
        ));
        cfg.dashboard.user_color = DashboardConfig::default().user_color;
    }

    if cfg.dashboard.assistant_name.trim().is_empty() {
        warnings.push("assistant_name empty -> default".to_string());
        cfg.dashboard.assistant_name = DashboardConfig::default().assistant_name;
    }

    if cfg.dashboard.user_name.trim().is_empty() {
        warnings.push("user_name empty -> default".to_string());
        cfg.dashboard.user_name = DashboardConfig::default().user_name;
    }

    (cfg, warnings)
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

    let badge_bg = theme_badge_bg(&app.config.global.theme);
    let title = Paragraph::new(Line::from(vec![
        Span::styled(" pro-tui ", Style::default().fg(Color::Black).bg(badge_bg)),
        Span::raw(format!(
            " Mac Pro dashboard for Mac mini agent  |  Screen: {}",
            App::screen_name(app.active_screen)
        )),
    ]));
    frame.render_widget(title, outer[0]);

    match app.active_screen {
        Screen::Dashboard => render_dashboard_screen(frame, app, outer[1]),
        Screen::Customize => render_customize_screen(frame, app, outer[1]),
    }

    let mut footer_spans = vec![
        Span::styled("q", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" quit  "),
        Span::styled("Tab", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" screens  "),
        Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" close/modal back  "),
        Span::styled("h", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" help"),
    ];

    if app.active_screen == Screen::Dashboard {
        footer_spans.extend([
            Span::raw("  "),
            Span::styled("r", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" refresh  "),
            Span::styled("e", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" expand events  "),
            Span::styled("i", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" chat input  "),
            Span::styled("m", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" models  "),
            Span::styled("↑/↓", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" chat scroll"),
        ]);
    } else {
        footer_spans.extend([
            Span::raw("  "),
            Span::styled("↑/↓", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" option  "),
            Span::styled("←/→", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" change value  "),
            Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" text edit/apply  "),
            Span::styled("s", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" save all  "),
            Span::styled("r", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" reset selected  "),
            Span::styled("[ / ]", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" section"),
        ]);
    }

    let footer = Paragraph::new(Line::from(footer_spans));
    frame.render_widget(footer, outer[2]);

    if app.show_help {
        render_help_popup(frame, app);
    }

    if app.show_model_selector {
        render_model_selector_popup(frame, app);
    }

    if app.show_navigator {
        render_navigator_popup(frame, app);
    }
}

fn render_dashboard_screen(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    match app.config.dashboard.layout_preset {
        DashboardLayoutPreset::ThreeTopTwoBottom => {
            render_dashboard_three_top_two_bottom(frame, app, area)
        }
        DashboardLayoutPreset::LlmColumn => render_dashboard_llm_column(frame, app, area),
    }
}

fn render_dashboard_three_top_two_bottom(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    let top_constraints = if app.config.dashboard.show_processes {
        vec![Constraint::Percentage(75), Constraint::Percentage(25)]
    } else {
        vec![Constraint::Percentage(100)]
    };
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(top_constraints)
        .split(body[0]);

    let top_left = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(top[0]);

    if app.config.dashboard.show_status {
        frame.render_widget(
            Paragraph::new(status_lines(app))
                .block(block("Status"))
                .wrap(Wrap { trim: true }),
            top_left[0],
        );
    } else {
        frame.render_widget(
            Paragraph::new("hidden (toggle in Customize)").block(block("Status")),
            top_left[0],
        );
    }

    if app.config.dashboard.show_system {
        frame.render_widget(
            Paragraph::new(system_lines(app))
                .block(block("System"))
                .wrap(Wrap { trim: true }),
            top_left[1],
        );
    } else {
        frame.render_widget(
            Paragraph::new("hidden (toggle in Customize)").block(block("System")),
            top_left[1],
        );
    }

    if app.config.dashboard.show_processes && top.len() > 1 {
        frame.render_widget(
            Paragraph::new(process_lines(app))
                .block(block("Top Processes"))
                .wrap(Wrap { trim: true }),
            top[1],
        );
    }

    let bottom = if app.config.dashboard.show_events {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
            .split(body[1])
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(100)])
            .split(body[1])
    };

    render_llm_panel(frame, app, bottom[0]);

    if app.config.dashboard.show_events && bottom.len() > 1 {
        frame.render_widget(
            Paragraph::new(event_lines(app))
                .block(block("File Events"))
                .wrap(Wrap { trim: true }),
            bottom[1],
        );
    }
}

fn render_dashboard_llm_column(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(33), Constraint::Percentage(67)])
        .split(area);

    render_llm_panel(frame, app, columns[0]);

    let right = if app.config.dashboard.show_events {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
            .split(columns[1])
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(100)])
            .split(columns[1])
    };

    let top_constraints = if app.config.dashboard.show_processes {
        vec![Constraint::Percentage(70), Constraint::Percentage(30)]
    } else {
        vec![Constraint::Percentage(100)]
    };
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(top_constraints)
        .split(right[0]);

    let top_left = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(top[0]);

    if app.config.dashboard.show_status {
        frame.render_widget(
            Paragraph::new(status_lines(app))
                .block(block("Status"))
                .wrap(Wrap { trim: true }),
            top_left[0],
        );
    } else {
        frame.render_widget(
            Paragraph::new("hidden (toggle in Customize)").block(block("Status")),
            top_left[0],
        );
    }

    if app.config.dashboard.show_system {
        frame.render_widget(
            Paragraph::new(system_lines(app))
                .block(block("System"))
                .wrap(Wrap { trim: true }),
            top_left[1],
        );
    } else {
        frame.render_widget(
            Paragraph::new("hidden (toggle in Customize)").block(block("System")),
            top_left[1],
        );
    }

    if app.config.dashboard.show_processes && top.len() > 1 {
        frame.render_widget(
            Paragraph::new(process_lines(app))
                .block(block("Top Processes"))
                .wrap(Wrap { trim: true }),
            top[1],
        );
    }

    if app.config.dashboard.show_events && right.len() > 1 {
        frame.render_widget(
            Paragraph::new(event_lines(app))
                .block(block("File Events"))
                .wrap(Wrap { trim: true }),
            right[1],
        );
    }
}

fn render_customize_screen(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25), Constraint::Percentage(75)])
        .split(area);

    let mut section_lines = vec![Line::raw("Sections"), Line::raw("")];
    for (idx, section) in App::customize_sections().iter().enumerate() {
        let prefix = if idx == app.customize_section_idx {
            ">"
        } else {
            " "
        };
        let label = match section {
            CustomizeSection::Global => "Global",
            CustomizeSection::Dashboard => "Dashboard",
        };
        section_lines.push(if idx == app.customize_section_idx {
            Line::from(vec![
                Span::raw(format!("{prefix} ")),
                Span::styled(
                    label,
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            ])
        } else {
            Line::raw(format!("{prefix} {label}"))
        });
    }

    frame.render_widget(
        Paragraph::new(section_lines)
            .block(block("Customize"))
            .wrap(Wrap { trim: true }),
        layout[0],
    );

    let mut option_lines: Vec<Line<'static>> = vec![
        Line::raw("Use [ and ] to switch section".to_string()),
        Line::raw("Up/Down select option, Left/Right change value".to_string()),
        Line::raw("Enter on assistant_name/user_name: start inline edit".to_string()),
        Line::raw("Enter while editing: apply text, Esc: cancel inline edit".to_string()),
        Line::raw("s saves all changes, r resets selected option to default".to_string()),
        Line::raw("".to_string()),
    ];

    match app.active_customize_section() {
        CustomizeSection::Global => {
            for (idx, option) in App::global_options().iter().enumerate() {
                let selected = idx == app.global_option_idx;
                let value = match option {
                    GlobalOption::Theme => app.config.global.theme.clone(),
                };
                option_lines.push(customize_option_line(selected, false, "theme", &value));
            }
        }
        CustomizeSection::Dashboard => {
            for (idx, option) in App::dashboard_options().iter().enumerate() {
                let selected = idx == app.dashboard_option_idx;
                let (name, value) = match option {
                    DashboardOption::AssistantName => (
                        "assistant_name",
                        app.config.dashboard.assistant_name.clone(),
                    ),
                    DashboardOption::AssistantColor => (
                        "assistant_color",
                        app.config.dashboard.assistant_color.clone(),
                    ),
                    DashboardOption::UserName => {
                        ("user_name", app.config.dashboard.user_name.clone())
                    }
                    DashboardOption::UserColor => {
                        ("user_color", app.config.dashboard.user_color.clone())
                    }
                    DashboardOption::LayoutPreset => (
                        "layout_preset",
                        layout_preset_label(app.config.dashboard.layout_preset).to_string(),
                    ),
                    DashboardOption::ShowStatus => (
                        "show_status",
                        bool_label(app.config.dashboard.show_status).to_string(),
                    ),
                    DashboardOption::ShowSystem => (
                        "show_system",
                        bool_label(app.config.dashboard.show_system).to_string(),
                    ),
                    DashboardOption::ShowProcesses => (
                        "show_processes",
                        bool_label(app.config.dashboard.show_processes).to_string(),
                    ),
                    DashboardOption::ShowEvents => (
                        "show_events",
                        bool_label(app.config.dashboard.show_events).to_string(),
                    ),
                };
                let editing = app.customize_text_mode
                    && selected
                    && matches!(
                        option,
                        DashboardOption::AssistantName | DashboardOption::UserName
                    );
                let display_value = if editing {
                    format!("{}_", app.customize_text_buffer)
                } else {
                    value
                };

                option_lines.push(customize_option_line(
                    selected,
                    editing,
                    name,
                    &display_value,
                ));
            }
        }
    }

    option_lines.push(Line::raw("".to_string()));
    option_lines.push(Line::raw(format!(
        "Config path: {}",
        app.config_path.display()
    )));
    option_lines.push(Line::raw(format!(
        "Dirty: {}",
        bool_label(app.config_dirty)
    )));

    if let Some(msg) = app.visible_status_message() {
        option_lines.push(Line::from(vec![
            Span::styled("Status: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(msg.to_string()),
        ]));
    }

    frame.render_widget(
        Paragraph::new(option_lines)
            .block(block("Options"))
            .wrap(Wrap { trim: true }),
        layout[1],
    );
}

fn render_navigator_popup(frame: &mut ratatui::Frame, app: &App) {
    let popup = centered_rect(45, 45, frame.area());
    frame.render_widget(Clear, popup);

    let mut lines = vec![
        Line::raw("Navigator (Enter select, Esc close)"),
        Line::raw(""),
    ];

    for (idx, screen) in App::screens().iter().enumerate() {
        let hover = if idx == app.navigator_hover_idx {
            ">"
        } else {
            " "
        };
        let active = if *screen == app.active_screen {
            "*"
        } else {
            " "
        };
        let label = App::screen_name(*screen);
        lines.push(Line::from(vec![
            Span::raw(format!("{}{} ", hover, active)),
            if idx == app.navigator_hover_idx {
                Span::styled(
                    label,
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::raw(label)
            },
        ]));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(block("Screens"))
            .wrap(Wrap { trim: true }),
        popup,
    );
}

fn render_help_popup(frame: &mut ratatui::Frame, app: &App) {
    let popup = centered_rect(78, 78, frame.area());
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(vec![
            Line::raw("pro-tui help"),
            Line::raw(""),
            Line::raw("General (all screens)"),
            Line::raw("q    quit"),
            Line::raw("Tab  open screen navigator"),
            Line::raw("h    open help"),
            Line::raw("Esc  close active modal/input"),
            Line::raw(""),
            Line::raw(format!(
                "Current screen: {}",
                App::screen_name(app.active_screen)
            )),
            Line::raw(""),
            Line::raw("Dashboard-only"),
            Line::raw("r        refresh immediately"),
            Line::raw("e        expand/collapse file events"),
            Line::raw("i        focus chat input"),
            Line::raw("m        open model selector"),
            Line::raw("Up/Down  scroll chat"),
            Line::raw(""),
            Line::raw("Model Selector"),
            Line::raw("Up/Down  move selection"),
            Line::raw("Enter    apply model"),
            Line::raw("Esc      close selector"),
            Line::raw(""),
            Line::raw("Customize screen"),
            Line::raw("[ / ]    switch section"),
            Line::raw("Up/Down  move option"),
            Line::raw("Left/Right change value"),
            Line::raw("Enter    inline edit/apply text fields"),
            Line::raw("s        save all Customize changes to config"),
            Line::raw("r        reset selected option to default"),
            Line::raw("Esc      cancel inline text edit"),
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
        let (label, prefix_style) = role_display(app, &msg.role);

        if formatted.is_empty() {
            out.push(Line::from(vec![Span::styled(
                format!("{}:", label),
                prefix_style,
            )]));
            continue;
        }

        for (idx, line) in formatted.iter().enumerate() {
            let first_prefix = format!("{}: ", label);
            let cont_prefix = " ".repeat(label.len() + 2);
            let prefix = if idx == 0 {
                first_prefix.as_str()
            } else {
                cont_prefix.as_str()
            };

            let styled = match line.kind {
                ChatLineKind::Plain => Line::from(vec![
                    Span::styled(prefix.to_string(), prefix_style),
                    Span::raw(line.text.clone()),
                ]),
                ChatLineKind::Code => Line::from(vec![
                    Span::styled(prefix.to_string(), prefix_style),
                    Span::styled(line.text.clone(), Style::default().fg(Color::Cyan)),
                ]),
                ChatLineKind::CodeLang => Line::from(vec![
                    Span::styled(prefix.to_string(), prefix_style),
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

// Tiny markdown-ish formatter for terminal chat readability.
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

fn role_display(app: &App, role: &str) -> (String, Style) {
    match role {
        "you" => (
            app.config.dashboard.user_name.clone(),
            Style::default()
                .fg(color_from_name(&app.config.dashboard.user_color).unwrap_or(Color::Green)),
        ),
        "assistant" => (
            app.config.dashboard.assistant_name.clone(),
            Style::default()
                .fg(color_from_name(&app.config.dashboard.assistant_color).unwrap_or(Color::Cyan)),
        ),
        "system" => (
            "system".to_string(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        "error" => (
            "error".to_string(),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        other => (other.to_string(), Style::default()),
    }
}

fn theme_badge_bg(theme: &str) -> Color {
    match theme {
        "amber" => Color::Yellow,
        "ocean" => Color::Blue,
        "mono" => Color::Gray,
        _ => Color::Green,
    }
}

fn theme_names() -> &'static [&'static str] {
    &["default", "amber", "ocean", "mono"]
}

fn rotate_theme(current: &str, delta: i32) -> String {
    rotate_name(theme_names(), current, delta).to_string()
}

fn color_names() -> &'static [&'static str] {
    &[
        "white",
        "gray",
        "red",
        "green",
        "yellow",
        "blue",
        "magenta",
        "cyan",
        "light_red",
        "light_green",
        "light_yellow",
        "light_blue",
        "light_magenta",
        "light_cyan",
    ]
}

fn rotate_color_name(current: &str, delta: i32) -> String {
    rotate_name(color_names(), current, delta).to_string()
}

fn rotate_name<'a>(items: &'a [&'a str], current: &str, delta: i32) -> &'a str {
    if items.is_empty() {
        return "";
    }

    let len = items.len() as i32;
    let idx = items
        .iter()
        .position(|c| c.eq_ignore_ascii_case(current))
        .unwrap_or(0) as i32;
    let next = (idx + delta).rem_euclid(len) as usize;
    items[next]
}

fn color_from_name(name: &str) -> Option<Color> {
    match name {
        "white" => Some(Color::White),
        "gray" => Some(Color::Gray),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "light_red" => Some(Color::LightRed),
        "light_green" => Some(Color::LightGreen),
        "light_yellow" => Some(Color::LightYellow),
        "light_blue" => Some(Color::LightBlue),
        "light_magenta" => Some(Color::LightMagenta),
        "light_cyan" => Some(Color::LightCyan),
        _ => None,
    }
}

fn rotate_layout(current: DashboardLayoutPreset, delta: i32) -> DashboardLayoutPreset {
    let layouts = [
        DashboardLayoutPreset::ThreeTopTwoBottom,
        DashboardLayoutPreset::LlmColumn,
    ];
    let idx = layouts.iter().position(|l| *l == current).unwrap_or(0);
    let next = step_index(idx, layouts.len() - 1, delta);
    layouts[next]
}

fn layout_preset_label(layout: DashboardLayoutPreset) -> &'static str {
    match layout {
        DashboardLayoutPreset::ThreeTopTwoBottom => "ThreeTopTwoBottom",
        DashboardLayoutPreset::LlmColumn => "LlmColumn",
    }
}

fn customize_option_line(selected: bool, editing: bool, name: &str, value: &str) -> Line<'static> {
    if selected {
        let mut style = Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD);
        if editing {
            style = style.fg(Color::LightCyan);
        }
        Line::from(vec![
            Span::raw("> ".to_string()),
            Span::styled(format!("{name}: {value}"), style),
        ])
    } else {
        Line::raw(format!("  {name}: {value}"))
    }
}

fn bool_label(v: bool) -> &'static str {
    if v { "true" } else { "false" }
}

fn step_index(current: usize, max: usize, delta: i32) -> usize {
    let max_i = max as i32;
    let cur_i = current.min(max) as i32;
    let next = (cur_i + delta).clamp(0, max_i);
    next as usize
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
