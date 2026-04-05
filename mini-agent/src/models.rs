use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_percent: f32,
    pub current_cpu_percent: f32,
    pub memory_bytes: u64,
    pub samples_5m: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SystemMetrics {
    pub cpu_percent: f32,
    pub memory_used_bytes: u64,
    pub memory_total_bytes: u64,
    pub swap_used_bytes: u64,
    pub swap_total_bytes: u64,
    pub root_used_bytes: u64,
    pub root_total_bytes: u64,
    pub top_processes: Vec<ProcessInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LlmMetrics {
    pub ollama_online: bool,
    pub ollama_ps_url: String,
    pub running_models: Vec<String>,
    pub model_count: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentState {
    pub seq: u64,
    pub ts_ms: u128,
    pub hostname: String,
    pub watched_dirs: Vec<String>,
    pub recent_file_events: Vec<String>,
    pub system: SystemMetrics,
    pub llm: LlmMetrics,
}

#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    pub prompt: String,
    pub model: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ChatResponse {
    pub response: String,
    pub model: String,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ModelsResponse {
    pub selected_model: String,
    pub running_models: Vec<String>,
    pub installed_models: Vec<String>,
    pub error: Option<String>,
}
