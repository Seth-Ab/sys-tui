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

mod config;
mod net;
mod ui;

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
    modules: DashboardModulesConfig,
    #[serde(default)]
    themes: BTreeMap<String, ThemePaletteConfig>,
    #[serde(flatten)]
    extra: BTreeMap<String, toml::Value>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            global: GlobalConfig::default(),
            dashboard: DashboardConfig::default(),
            modules: DashboardModulesConfig::default(),
            themes: BTreeMap::new(),
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
    show_flow: bool,
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
            show_flow: true,
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct DashboardModulesConfig {
    flow_map: FlowMapModuleConfig,
    system: SystemModuleConfig,
    #[serde(flatten)]
    extra: BTreeMap<String, toml::Value>,
}

impl Default for DashboardModulesConfig {
    fn default() -> Self {
        Self {
            flow_map: FlowMapModuleConfig::default(),
            system: SystemModuleConfig::default(),
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct FlowMapModuleConfig {
    colorize: bool,
    active_color: String,
    run_color: String,
    wait_color: String,
    ok_color: String,
    err_color: String,
    #[serde(flatten)]
    extra: BTreeMap<String, toml::Value>,
}

impl Default for FlowMapModuleConfig {
    fn default() -> Self {
        Self {
            colorize: true,
            active_color: "light_yellow".to_string(),
            run_color: "yellow".to_string(),
            wait_color: "yellow".to_string(),
            ok_color: "light_green".to_string(),
            err_color: "light_red".to_string(),
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct SystemModuleConfig {
    colorize: bool,
    memory_warn_percent: u8,
    memory_crit_percent: u8,
    warn_color: String,
    crit_color: String,
    #[serde(flatten)]
    extra: BTreeMap<String, toml::Value>,
}

impl Default for SystemModuleConfig {
    fn default() -> Self {
        Self {
            colorize: true,
            memory_warn_percent: 75,
            memory_crit_percent: 90,
            warn_color: "yellow".to_string(),
            crit_color: "light_red".to_string(),
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
struct ThemeColor {
    r: u8,
    g: u8,
    b: u8,
}

impl ThemeColor {
    fn from_u8(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    fn to_display_color(self) -> Color {
        Color::Indexed(nearest_ansi256_index(self))
    }
}

impl Default for ThemeColor {
    fn default() -> Self {
        Self {
            r: 255,
            g: 255,
            b: 255,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct ThemePaletteConfig {
    main_text_color: ThemeColor,
    secondary_text_color: ThemeColor,
    border_color: ThemeColor,
    section_title_color: ThemeColor,
    focus_color: ThemeColor,
    title_color: ThemeColor,
    #[serde(flatten)]
    extra: BTreeMap<String, toml::Value>,
}

impl Default for ThemePaletteConfig {
    fn default() -> Self {
        Self {
            main_text_color: ThemeColor::from_u8(255, 255, 255),
            secondary_text_color: ThemeColor::from_u8(150, 150, 150),
            border_color: ThemeColor::from_u8(0, 170, 130),
            section_title_color: ThemeColor::from_u8(100, 255, 220),
            focus_color: ThemeColor::from_u8(100, 255, 220),
            title_color: ThemeColor::from_u8(0, 166, 128),
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
    Themes,
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
    ShowFlow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThemeColorField {
    MainText,
    SecondaryText,
    Border,
    SectionTitle,
    Focus,
    Title,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThemesOption {
    ColorPick(ThemeColorField),
    NewThemeName,
    SaveNewTheme,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColorPickerRow {
    BasePrimary,
    BaseExtended,
    Shade,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CustomizeTextTarget {
    DashboardAssistantName,
    DashboardUserName,
    ThemeNewName,
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
    themes_option_idx: usize,
    theme_draft: ThemePaletteConfig,
    theme_new_name: String,
    theme_draft_live_preview: bool,
    customize_text_mode: bool,
    customize_text_buffer: String,
    customize_text_target: Option<CustomizeTextTarget>,
    show_color_picker: bool,
    color_picker_field: Option<ThemeColorField>,
    color_picker_row: ColorPickerRow,
    color_picker_idx: usize,
    color_picker_primary_idx: usize,
    color_picker_extended_idx: usize,
    color_picker_shade_idx: usize,
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
        let current_theme = config.global.theme.clone();
        let theme_draft = resolve_theme_palette_config(&config, &current_theme);
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
            themes_option_idx: 0,
            theme_draft,
            theme_new_name: String::new(),
            theme_draft_live_preview: false,
            customize_text_mode: false,
            customize_text_buffer: String::new(),
            customize_text_target: None,
            show_color_picker: false,
            color_picker_field: None,
            color_picker_row: ColorPickerRow::BasePrimary,
            color_picker_idx: 0,
            color_picker_primary_idx: 0,
            color_picker_extended_idx: 16,
            color_picker_shade_idx: 8,
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
            self.customize_text_target = None;
            self.show_color_picker = false;
            self.color_picker_field = None;
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
            if self.show_color_picker {
                self.show_color_picker = false;
                self.color_picker_field = None;
                return AppAction::None;
            }
            if self.customize_text_mode {
                self.customize_text_mode = false;
                self.customize_text_buffer.clear();
                self.customize_text_target = None;
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

        if self.show_color_picker {
            return self.on_key_color_picker(key);
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

    fn on_key_color_picker(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Esc => {
                self.show_color_picker = false;
                self.color_picker_field = None;
            }
            KeyCode::Up => {
                self.color_picker_row = match self.color_picker_row {
                    ColorPickerRow::Shade => ColorPickerRow::BaseExtended,
                    ColorPickerRow::BaseExtended => ColorPickerRow::BasePrimary,
                    ColorPickerRow::BasePrimary => ColorPickerRow::BasePrimary,
                };
            }
            KeyCode::Down => {
                self.color_picker_row = match self.color_picker_row {
                    ColorPickerRow::BasePrimary => ColorPickerRow::BaseExtended,
                    ColorPickerRow::BaseExtended => ColorPickerRow::Shade,
                    ColorPickerRow::Shade => ColorPickerRow::Shade,
                };
            }
            KeyCode::Left => match self.color_picker_row {
                ColorPickerRow::BasePrimary => {
                    if self.color_picker_primary_idx > 0 {
                        self.color_picker_primary_idx -= 1;
                        self.color_picker_idx = self.color_picker_primary_idx;
                    }
                }
                ColorPickerRow::BaseExtended => {
                    if self.color_picker_extended_idx > 16 {
                        self.color_picker_extended_idx -= 1;
                        self.color_picker_idx = self.color_picker_extended_idx;
                    }
                }
                ColorPickerRow::Shade => {
                    if self.color_picker_shade_idx > 0 {
                        self.color_picker_shade_idx -= 1;
                    }
                }
            },
            KeyCode::Right => match self.color_picker_row {
                ColorPickerRow::BasePrimary => {
                    if self.color_picker_primary_idx < 15 {
                        self.color_picker_primary_idx += 1;
                        self.color_picker_idx = self.color_picker_primary_idx;
                    }
                }
                ColorPickerRow::BaseExtended => {
                    if self.color_picker_extended_idx + 1 < base_palette_len() {
                        self.color_picker_extended_idx += 1;
                        self.color_picker_idx = self.color_picker_extended_idx;
                    }
                }
                ColorPickerRow::Shade => {
                    if self.color_picker_shade_idx < 15 {
                        self.color_picker_shade_idx += 1;
                    }
                }
            },
            KeyCode::Enter => {
                if let Some(field) = self.color_picker_field {
                    let picked = self.color_picker_current_color();
                    if let Some(dst) = self.theme_color_for_editor_mut(field) {
                        *dst = picked;
                        self.config_dirty = true;
                    }
                }
                self.show_color_picker = false;
                self.color_picker_field = None;
            }
            _ => {}
        }
        AppAction::None
    }

    fn open_color_picker_for_field(&mut self, field: ThemeColorField) {
        let current = self.theme_color_for_editor(field).unwrap_or_default();
        let base_idx = nearest_base_palette_index(current);
        self.color_picker_primary_idx = base_idx.min(15);
        self.color_picker_extended_idx = base_idx.max(16).min(base_palette_len().saturating_sub(1));
        self.show_color_picker = true;
        self.color_picker_field = Some(field);
        self.color_picker_row = if base_idx < 16 {
            ColorPickerRow::BasePrimary
        } else {
            ColorPickerRow::BaseExtended
        };
        self.color_picker_idx = base_idx;
        self.color_picker_shade_idx = nearest_shade_index(base_idx, current);
    }

    fn color_picker_current_color(&self) -> ThemeColor {
        match self.color_picker_row {
            ColorPickerRow::BasePrimary => base_palette_color_at(self.color_picker_primary_idx),
            ColorPickerRow::BaseExtended => base_palette_color_at(self.color_picker_extended_idx),
            ColorPickerRow::Shade => {
                let shades = shade_gradient_for_base(self.color_picker_idx);
                shades[self.color_picker_shade_idx.min(15)]
            }
        }
    }

    fn on_key_customize(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Char('[') => self.prev_customize_section(),
            KeyCode::Char(']') => self.next_customize_section(),
            KeyCode::Up => self.move_customize_option(-1),
            KeyCode::Down => self.move_customize_option(1),
            KeyCode::Left => self.adjust_selected_customize_value(-1),
            KeyCode::Right => self.adjust_selected_customize_value(1),
            KeyCode::Char('t') => {
                if self.active_customize_section() == CustomizeSection::Themes {
                    self.theme_draft_live_preview = !self.theme_draft_live_preview;
                    self.set_status_message(if self.theme_draft_live_preview {
                        "new theme live preview: on"
                    } else {
                        "new theme live preview: off"
                    });
                }
            }
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
                self.customize_text_target = None;
            }
            KeyCode::Backspace => {
                self.customize_text_buffer.pop();
            }
            KeyCode::Enter => {
                let value = self.customize_text_buffer.trim().to_string();
                self.apply_selected_text_value(value);
                self.customize_text_mode = false;
                self.customize_text_buffer.clear();
                self.customize_text_target = None;
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
            CustomizeSection::Themes => {
                let max = self.themes_options().len().saturating_sub(1);
                self.themes_option_idx = step_index(self.themes_option_idx, max, delta);
            }
        }
    }

    fn adjust_selected_customize_value(&mut self, delta: i32) {
        match self.active_customize_section() {
            CustomizeSection::Global => {
                if self.active_global_option() == GlobalOption::Theme {
                    let names = self.available_theme_names();
                    self.config.global.theme =
                        rotate_name_owned(&names, &self.config.global.theme, delta);
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
                DashboardOption::ShowFlow => {
                    self.config.dashboard.show_flow = !self.config.dashboard.show_flow;
                    self.config_dirty = true;
                }
                DashboardOption::AssistantName | DashboardOption::UserName => {}
            },
            CustomizeSection::Themes => self.adjust_theme_option(delta),
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
                DashboardOption::ShowFlow => {
                    self.config.dashboard.show_flow = defaults.dashboard.show_flow
                }
            },
            CustomizeSection::Themes => self.reset_theme_option(),
        }

        self.config_dirty = true;
    }

    fn enter_or_apply_selected_customize_option(&mut self) {
        match self.active_customize_section() {
            CustomizeSection::Global => {
                if self.active_global_option() == GlobalOption::Theme {
                    let names = self.available_theme_names();
                    self.config.global.theme =
                        rotate_name_owned(&names, &self.config.global.theme, 1);
                    self.config_dirty = true;
                }
            }
            CustomizeSection::Dashboard => match self.active_dashboard_option() {
                DashboardOption::AssistantName => {
                    self.customize_text_mode = true;
                    self.customize_text_target = Some(CustomizeTextTarget::DashboardAssistantName);
                    self.customize_text_buffer = self.config.dashboard.assistant_name.clone();
                }
                DashboardOption::UserName => {
                    self.customize_text_mode = true;
                    self.customize_text_target = Some(CustomizeTextTarget::DashboardUserName);
                    self.customize_text_buffer = self.config.dashboard.user_name.clone();
                }
                _ => self.adjust_selected_customize_value(1),
            },
            CustomizeSection::Themes => self.enter_theme_option(),
        }
    }

    fn apply_selected_text_value(&mut self, value: String) {
        if value.is_empty() {
            return;
        }

        match self.customize_text_target {
            Some(CustomizeTextTarget::DashboardAssistantName) => {
                self.config.dashboard.assistant_name = value;
                self.config_dirty = true;
            }
            Some(CustomizeTextTarget::DashboardUserName) => {
                self.config.dashboard.user_name = value;
                self.config_dirty = true;
            }
            Some(CustomizeTextTarget::ThemeNewName) => {
                self.theme_new_name = value;
            }
            None => {}
        }
    }

    fn available_theme_names(&self) -> Vec<String> {
        let mut out = builtin_theme_names();
        for k in self.config.themes.keys() {
            if !out.iter().any(|n| n == k) {
                out.push(k.clone());
            }
        }
        out
    }

    fn themes_options(&self) -> Vec<ThemesOption> {
        let mut out = vec![];
        for field in [
            ThemeColorField::MainText,
            ThemeColorField::SecondaryText,
            ThemeColorField::Border,
            ThemeColorField::SectionTitle,
            ThemeColorField::Focus,
            ThemeColorField::Title,
        ] {
            out.push(ThemesOption::ColorPick(field));
        }
        out.push(ThemesOption::NewThemeName);
        out.push(ThemesOption::SaveNewTheme);
        out
    }

    fn active_themes_option(&self) -> ThemesOption {
        let opts = self.themes_options();
        opts[self.themes_option_idx.min(opts.len().saturating_sub(1))]
    }

    fn adjust_theme_option(&mut self, delta: i32) {
        let _ = delta;
    }

    fn reset_theme_option(&mut self) {
        match self.active_themes_option() {
            ThemesOption::NewThemeName => self.theme_new_name.clear(),
            ThemesOption::SaveNewTheme => {}
            ThemesOption::ColorPick(field) => {
                let default_cfg = ThemePaletteConfig::default();
                if let Some(dst) = self.theme_color_for_editor_mut(field) {
                    *dst = theme_color_from_field(&default_cfg, field);
                }
            }
        }
    }

    fn enter_theme_option(&mut self) {
        match self.active_themes_option() {
            ThemesOption::NewThemeName => {
                self.customize_text_mode = true;
                self.customize_text_target = Some(CustomizeTextTarget::ThemeNewName);
                self.customize_text_buffer = self.theme_new_name.clone();
            }
            ThemesOption::SaveNewTheme => {
                let name = self.theme_new_name.trim().to_string();
                if name.is_empty() {
                    self.set_status_message("set new theme name first");
                    return;
                }

                let existed = self.config.themes.contains_key(&name);
                self.config.themes.insert(name.clone(), self.theme_draft.clone());
                self.config_dirty = true;
                if existed {
                    self.set_status_message("updated theme");
                } else {
                    self.set_status_message("created theme");
                }
            }
            ThemesOption::ColorPick(field) => self.open_color_picker_for_field(field),
        }
    }

    fn theme_color_for_editor(&self, field: ThemeColorField) -> Option<ThemeColor> {
        Some(theme_color_from_field(&self.theme_draft, field))
    }

    fn theme_color_for_editor_mut(&mut self, field: ThemeColorField) -> Option<&mut ThemeColor> {
        Some(theme_color_from_field_mut(&mut self.theme_draft, field))
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

    fn customize_sections() -> [CustomizeSection; 3] {
        [
            CustomizeSection::Global,
            CustomizeSection::Dashboard,
            CustomizeSection::Themes,
        ]
    }

    fn global_options() -> [GlobalOption; 1] {
        [GlobalOption::Theme]
    }

    fn dashboard_options() -> [DashboardOption; 10] {
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
            DashboardOption::ShowFlow,
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

pub fn run() -> Result<()> {
    let endpoint =
        env::var("AGENT_ENDPOINT").unwrap_or_else(|_| "http://127.0.0.1:8787/state".to_string());
    let token = env::var("AGENT_TOKEN").ok().filter(|v| !v.is_empty());

    let config_path = config::resolve_config_path()?;
    let (config, load_note) = config::load_config(&config_path);
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
            net::poll_state(&client, &mut app);
            last_poll = Instant::now();
        }

        terminal.draw(|frame| ui::ui(frame, &app))?;

        if event::poll(tick)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match app.on_key(key) {
                        AppAction::None => {}
                        AppAction::Refresh => {
                            net::poll_state(&client, &mut app);
                            last_poll = Instant::now();
                        }
                        AppAction::SendPrompt(prompt) => {
                            net::send_prompt(&client, &mut app, &chat_tx, prompt);
                        }
                        AppAction::OpenModelSelector => {
                            net::open_model_selector(&client, &mut app);
                        }
                        AppAction::SelectModel(model) => {
                            app.selected_model = Some(model.clone());
                            app.push_chat("system", format!("selected model: {model}"));
                        }
                        AppAction::SelectScreen(screen) => {
                            app.active_screen = screen;
                            if screen != Screen::Customize {
                                app.theme_draft_live_preview = false;
                            }
                            app.show_help = false;
                            app.show_model_selector = false;
                            app.show_navigator = false;
                            app.input_mode = false;
                            app.customize_text_mode = false;
                            app.customize_text_buffer.clear();
                            app.customize_text_target = None;
                            app.show_color_picker = false;
                            app.color_picker_field = None;
                        }
                        AppAction::SaveConfig => {
                            let (normalized, warnings) =
                                config::normalize_config(app.config.clone());
                            app.config = normalized;

                            match config::save_config(&app.config_path, &app.config) {
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

fn drain_chat_results(app: &mut App, rx: &Receiver<ChatJobResult>) {
    while let Ok(result) = rx.try_recv() {
        app.replace_chat_by_id(result.placeholder_id, result.role, result.content);
    }
}

fn active_theme_palette(app: &App) -> ThemePalette {
    if app.theme_draft_live_preview && app.active_screen == Screen::Customize {
        return theme_palette_from_cfg(app.theme_draft.clone(), None);
    }
    theme_palette(&app.config, &app.config.global.theme, None)
}

#[derive(Debug, Clone, Copy)]
struct ThemePalette {
    main_text_color: Color,
    secondary_text_color: Color,
    border_color: Color,
    section_title_color: Color,
    focus_color: Color,
    title_color: Color,
    title_text_color: Color,
}

fn theme_palette(
    config: &AppConfig,
    theme: &str,
    preview: Option<(ThemeColorField, ThemeColor)>,
) -> ThemePalette {
    let cfg = resolve_theme_palette_config(config, theme);
    theme_palette_from_cfg(cfg, preview)
}

fn theme_palette_from_cfg(
    mut cfg: ThemePaletteConfig,
    preview: Option<(ThemeColorField, ThemeColor)>,
) -> ThemePalette {
    if let Some((field, color)) = preview {
        *theme_color_from_field_mut(&mut cfg, field) = color;
    }

    ThemePalette {
        main_text_color: cfg.main_text_color.to_display_color(),
        secondary_text_color: cfg.secondary_text_color.to_display_color(),
        border_color: cfg.border_color.to_display_color(),
        section_title_color: cfg.section_title_color.to_display_color(),
        focus_color: cfg.focus_color.to_display_color(),
        title_color: cfg.title_color.to_display_color(),
        title_text_color: theme_title_text_color(cfg.title_color),
    }
}

fn theme_title_text_color(c: ThemeColor) -> Color {
    let luminance = 0.2126 * f64::from(c.r) + 0.7152 * f64::from(c.g) + 0.0722 * f64::from(c.b);
    if luminance > 140.0 {
        Color::Black
    } else {
        Color::White
    }
}

const BASE_PICKER_INDICES: [u8; 32] = [
    196, 202, 208, 214, 220, 226, 190, 154, 118, 82, 46, 47, 84, 85, 86, 87, 45, 39, 33, 27, 21,
    57, 93, 129, 165, 201, 200, 199, 171, 177, 183, 189,
];

fn base_palette_len() -> usize {
    BASE_PICKER_INDICES.len()
}

fn base_palette_color_at(idx: usize) -> ThemeColor {
    let safe = idx.min(BASE_PICKER_INDICES.len().saturating_sub(1));
    ansi256_rgb(BASE_PICKER_INDICES[safe])
}

fn base_palette_name(idx: usize) -> String {
    let safe = idx.min(BASE_PICKER_INDICES.len().saturating_sub(1));
    format!("ansi-{}", BASE_PICKER_INDICES[safe])
}

fn base_palette_short(idx: usize) -> String {
    let safe = idx.min(BASE_PICKER_INDICES.len().saturating_sub(1));
    format!("{:03}", BASE_PICKER_INDICES[safe])
}

fn nearest_base_palette_index(c: ThemeColor) -> usize {
    let mut best_idx = 0usize;
    let mut best_dist = i64::MAX;
    for (idx, color_idx) in BASE_PICKER_INDICES.iter().enumerate() {
        let sw = ansi256_rgb(*color_idx);
        let dr = i64::from(c.r) - i64::from(sw.r);
        let dg = i64::from(c.g) - i64::from(sw.g);
        let db = i64::from(c.b) - i64::from(sw.b);
        let dist = dr * dr + dg * dg + db * db;
        if dist < best_dist {
            best_dist = dist;
            best_idx = idx;
        }
    }
    best_idx
}

fn blend_theme_color(a: ThemeColor, b: ThemeColor, t: f32) -> ThemeColor {
    fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
        let af = f32::from(a);
        let bf = f32::from(b);
        let v = af + (bf - af) * t.clamp(0.0, 1.0);
        v.round().clamp(0.0, 255.0) as u8
    }

    ThemeColor {
        r: lerp_u8(a.r, b.r, t),
        g: lerp_u8(a.g, b.g, t),
        b: lerp_u8(a.b, b.b, t),
    }
}

fn shade_gradient_for_base(base_idx: usize) -> [ThemeColor; 16] {
    let base = base_palette_color_at(base_idx);
    let mut out = [base; 16];

    // 0..6: progressively darker (0 is darkest), 7: original color.
    for idx in 0..7 {
        let amt_to_black = (7 - idx) as f32 / 8.0;
        out[idx] = blend_theme_color(base, ThemeColor::from_u8(0, 0, 0), amt_to_black);
    }
    out[7] = base;

    // 8..15: progressively lighter (15 is lightest).
    for idx in 8..16 {
        let amt_to_white = (idx - 7) as f32 / 8.0;
        out[idx] = blend_theme_color(base, ThemeColor::from_u8(255, 255, 255), amt_to_white);
    }

    out
}

fn nearest_shade_index(base_idx: usize, current: ThemeColor) -> usize {
    let shades = shade_gradient_for_base(base_idx);
    let mut best = 0usize;
    let mut best_dist = i64::MAX;
    for (idx, c) in shades.iter().enumerate() {
        let dr = i64::from(current.r) - i64::from(c.r);
        let dg = i64::from(current.g) - i64::from(c.g);
        let db = i64::from(current.b) - i64::from(c.b);
        let dist = dr * dr + dg * dg + db * db;
        if dist < best_dist {
            best = idx;
            best_dist = dist;
        }
    }
    best
}

fn ansi256_rgb(index: u8) -> ThemeColor {
    match index {
        0 => ThemeColor::from_u8(0, 0, 0),
        1 => ThemeColor::from_u8(128, 0, 0),
        2 => ThemeColor::from_u8(0, 128, 0),
        3 => ThemeColor::from_u8(128, 128, 0),
        4 => ThemeColor::from_u8(0, 0, 128),
        5 => ThemeColor::from_u8(128, 0, 128),
        6 => ThemeColor::from_u8(0, 128, 128),
        7 => ThemeColor::from_u8(192, 192, 192),
        8 => ThemeColor::from_u8(128, 128, 128),
        9 => ThemeColor::from_u8(255, 0, 0),
        10 => ThemeColor::from_u8(0, 255, 0),
        11 => ThemeColor::from_u8(255, 255, 0),
        12 => ThemeColor::from_u8(0, 0, 255),
        13 => ThemeColor::from_u8(255, 0, 255),
        14 => ThemeColor::from_u8(0, 255, 255),
        15 => ThemeColor::from_u8(255, 255, 255),
        16..=231 => {
            let idx = index - 16;
            let r = idx / 36;
            let g = (idx % 36) / 6;
            let b = idx % 6;
            let level = [0, 95, 135, 175, 215, 255];
            ThemeColor::from_u8(level[r as usize], level[g as usize], level[b as usize])
        }
        232..=255 => {
            let v = 8 + (index - 232) * 10;
            ThemeColor::from_u8(v, v, v)
        }
    }
}

fn nearest_ansi256_index(c: ThemeColor) -> u8 {
    let mut best = 0u8;
    let mut best_dist = i64::MAX;
    for idx in 0u8..=255 {
        let sw = ansi256_rgb(idx);
        let dr = i64::from(c.r) - i64::from(sw.r);
        let dg = i64::from(c.g) - i64::from(sw.g);
        let db = i64::from(c.b) - i64::from(sw.b);
        let dist = dr * dr + dg * dg + db * db;
        if dist < best_dist {
            best = idx;
            best_dist = dist;
        }
    }
    best
}

fn builtin_theme_names() -> Vec<String> {
    vec![
        "default".to_string(),
        "amber".to_string(),
        "ocean".to_string(),
        "mono".to_string(),
    ]
}

fn builtin_theme_config(name: &str) -> Option<ThemePaletteConfig> {
    let cfg = match name {
        "amber" => ThemePaletteConfig {
            main_text_color: ThemeColor::from_u8(255, 255, 255),
            secondary_text_color: ThemeColor::from_u8(160, 160, 160),
            border_color: ThemeColor::from_u8(255, 215, 64),
            section_title_color: ThemeColor::from_u8(255, 215, 64),
            focus_color: ThemeColor::from_u8(255, 235, 140),
            title_color: ThemeColor::from_u8(255, 215, 64),
            extra: BTreeMap::new(),
        },
        "ocean" => ThemePaletteConfig {
            main_text_color: ThemeColor::from_u8(255, 255, 255),
            secondary_text_color: ThemeColor::from_u8(160, 160, 160),
            border_color: ThemeColor::from_u8(64, 133, 255),
            section_title_color: ThemeColor::from_u8(130, 200, 255),
            focus_color: ThemeColor::from_u8(130, 255, 255),
            title_color: ThemeColor::from_u8(64, 133, 255),
            extra: BTreeMap::new(),
        },
        "mono" => ThemePaletteConfig {
            main_text_color: ThemeColor::from_u8(255, 255, 255),
            secondary_text_color: ThemeColor::from_u8(160, 160, 160),
            border_color: ThemeColor::from_u8(140, 140, 140),
            section_title_color: ThemeColor::from_u8(255, 255, 255),
            focus_color: ThemeColor::from_u8(255, 255, 255),
            title_color: ThemeColor::from_u8(140, 140, 140),
            extra: BTreeMap::new(),
        },
        "default" => ThemePaletteConfig {
            main_text_color: ThemeColor::from_u8(255, 255, 255),
            secondary_text_color: ThemeColor::from_u8(160, 160, 160),
            border_color: ThemeColor::from_u8(40, 180, 140),
            section_title_color: ThemeColor::from_u8(120, 255, 220),
            focus_color: ThemeColor::from_u8(120, 255, 220),
            title_color: ThemeColor::from_u8(40, 180, 140),
            extra: BTreeMap::new(),
        },
        _ => return None,
    };
    Some(cfg)
}

fn resolve_theme_palette_config(config: &AppConfig, theme: &str) -> ThemePaletteConfig {
    config
        .themes
        .get(theme)
        .cloned()
        .or_else(|| builtin_theme_config(theme))
        .unwrap_or_else(ThemePaletteConfig::default)
}

fn theme_color_field_name(field: ThemeColorField) -> &'static str {
    match field {
        ThemeColorField::MainText => "main_text_color",
        ThemeColorField::SecondaryText => "secondary_text_color",
        ThemeColorField::Border => "border_color",
        ThemeColorField::SectionTitle => "section_title_color",
        ThemeColorField::Focus => "focus_color",
        ThemeColorField::Title => "title_color",
    }
}

fn theme_color_from_field(cfg: &ThemePaletteConfig, field: ThemeColorField) -> ThemeColor {
    match field {
        ThemeColorField::MainText => cfg.main_text_color,
        ThemeColorField::SecondaryText => cfg.secondary_text_color,
        ThemeColorField::Border => cfg.border_color,
        ThemeColorField::SectionTitle => cfg.section_title_color,
        ThemeColorField::Focus => cfg.focus_color,
        ThemeColorField::Title => cfg.title_color,
    }
}

fn theme_color_from_field_mut(
    cfg: &mut ThemePaletteConfig,
    field: ThemeColorField,
) -> &mut ThemeColor {
    match field {
        ThemeColorField::MainText => &mut cfg.main_text_color,
        ThemeColorField::SecondaryText => &mut cfg.secondary_text_color,
        ThemeColorField::Border => &mut cfg.border_color,
        ThemeColorField::SectionTitle => &mut cfg.section_title_color,
        ThemeColorField::Focus => &mut cfg.focus_color,
        ThemeColorField::Title => &mut cfg.title_color,
    }
}

fn rotate_name_owned(items: &[String], current: &str, delta: i32) -> String {
    if items.is_empty() {
        return current.to_string();
    }
    let len = items.len() as i32;
    let idx = items.iter().position(|n| n == current).unwrap_or(0) as i32;
    let next = (idx + delta).rem_euclid(len) as usize;
    items[next].clone()
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

fn customize_option_line(
    selected: bool,
    editing: bool,
    name: &str,
    value: &str,
    focus_color: Color,
) -> Line<'static> {
    if selected {
        let mut style = Style::default()
            .fg(focus_color)
            .add_modifier(Modifier::BOLD);
        if editing {
            style = style.add_modifier(Modifier::UNDERLINED);
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
