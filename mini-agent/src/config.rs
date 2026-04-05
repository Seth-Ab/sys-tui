use std::env;

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub bind_addr: String,
    pub token: Option<String>,
    pub watch_dirs: Vec<String>,
    pub ollama_ps_url: String,
    pub ollama_tags_url: String,
    pub ollama_chat_url: String,
    pub ollama_model: String,
}

impl AgentConfig {
    pub fn from_env() -> Self {
        let bind_addr = env::var("AGENT_BIND").unwrap_or_else(|_| "127.0.0.1:8787".to_string());
        let token = env::var("AGENT_TOKEN").ok().filter(|v| !v.is_empty());
        let watch_dirs = configured_watch_dirs();
        let ollama_ps_url = env::var("OLLAMA_PS_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:11434/api/ps".to_string());
        let ollama_tags_url = env::var("OLLAMA_TAGS_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:11434/api/tags".to_string());
        let ollama_chat_url = env::var("OLLAMA_CHAT_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:11434/api/generate".to_string());
        let ollama_model = env::var("OLLAMA_MODEL").unwrap_or_else(|_| "llama3.2".to_string());

        Self {
            bind_addr,
            token,
            watch_dirs,
            ollama_ps_url,
            ollama_tags_url,
            ollama_chat_url,
            ollama_model,
        }
    }
}

fn configured_watch_dirs() -> Vec<String> {
    if let Ok(raw) = env::var("WATCH_DIRS") {
        return raw
            .split(',')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToString::to_string)
            .collect();
    }

    let mut defaults = Vec::new();
    if let Ok(home) = env::var("HOME") {
        defaults.push(format!("{home}/Downloads"));
        defaults.push(format!("{home}/.ollama"));
    }
    defaults
}
