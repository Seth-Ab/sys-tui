use std::collections::VecDeque;
use std::env;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sysinfo::{Disks, System};
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ProcessInfo {
    pid: u32,
    name: String,
    cpu_percent: f32,
    memory_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct LlmMetrics {
    ollama_online: bool,
    ollama_ps_url: String,
    running_models: Vec<String>,
    model_count: usize,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct AgentState {
    seq: u64,
    ts_ms: u128,
    hostname: String,
    watched_dirs: Vec<String>,
    recent_file_events: Vec<String>,
    system: SystemMetrics,
    llm: LlmMetrics,
}

#[derive(Clone)]
struct AppCtx {
    token: Option<String>,
    snapshot: Arc<RwLock<AgentState>>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let bind_addr = env::var("AGENT_BIND").unwrap_or_else(|_| "127.0.0.1:8787".to_string());
    let token = env::var("AGENT_TOKEN").ok().filter(|v| !v.is_empty());
    let watch_dirs = configured_watch_dirs();
    let ollama_ps_url =
        env::var("OLLAMA_PS_URL").unwrap_or_else(|_| "http://127.0.0.1:11434/api/ps".to_string());

    let events = Arc::new(Mutex::new(VecDeque::with_capacity(200)));
    let _watcher = start_file_watcher(&watch_dirs, Arc::clone(&events))?;

    let snapshot = Arc::new(RwLock::new(AgentState {
        watched_dirs: watch_dirs.clone(),
        ..Default::default()
    }));

    let collector_snapshot = Arc::clone(&snapshot);
    let collector_events = Arc::clone(&events);
    tokio::spawn(async move {
        let client = Client::new();
        let mut seq: u64 = 0;

        loop {
            seq = seq.saturating_add(1);
            let next =
                collect_snapshot(seq, &ollama_ps_url, &client, &watch_dirs, &collector_events)
                    .await;
            *collector_snapshot.write().await = next;
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/state", get(state_handler))
        .with_state(AppCtx { token, snapshot });

    let addr: SocketAddr = bind_addr.parse()?;
    println!("mini-agent listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn health_handler() -> &'static str {
    "ok"
}

async fn state_handler(
    State(ctx): State<AppCtx>,
    headers: HeaderMap,
) -> Result<Json<AgentState>, StatusCode> {
    if let Some(expected) = &ctx.token {
        let provided = headers
            .get("x-agent-token")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();

        if provided != expected {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }

    let snapshot = ctx.snapshot.read().await.clone();
    Ok(Json(snapshot))
}

async fn collect_snapshot(
    seq: u64,
    ollama_ps_url: &str,
    client: &Client,
    watch_dirs: &[String],
    events: &Arc<Mutex<VecDeque<String>>>,
) -> AgentState {
    let mut system = System::new_all();
    system.refresh_all();

    let cpu_percent = system.global_cpu_usage();
    let memory_total_bytes = system.total_memory();
    let memory_used_bytes = system.used_memory();
    let swap_total_bytes = system.total_swap();
    let swap_used_bytes = system.used_swap();

    let mut top_processes: Vec<ProcessInfo> = system
        .processes()
        .iter()
        .map(|(pid, proc_)| ProcessInfo {
            pid: pid.as_u32(),
            name: proc_.name().to_string_lossy().into_owned(),
            cpu_percent: proc_.cpu_usage(),
            memory_bytes: proc_.memory(),
        })
        .collect();
    top_processes.sort_by(|a, b| b.cpu_percent.total_cmp(&a.cpu_percent));
    top_processes.truncate(10);

    let (root_total_bytes, root_used_bytes) = root_disk_usage();

    let llm = collect_llm_metrics(ollama_ps_url, client).await;

    AgentState {
        seq,
        ts_ms: now_ms(),
        hostname: System::host_name().unwrap_or_else(|| "unknown".to_string()),
        watched_dirs: watch_dirs.to_vec(),
        recent_file_events: read_recent_events(events, 20),
        system: SystemMetrics {
            cpu_percent,
            memory_used_bytes,
            memory_total_bytes,
            swap_used_bytes,
            swap_total_bytes,
            root_used_bytes,
            root_total_bytes,
            top_processes,
        },
        llm,
    }
}

async fn collect_llm_metrics(ollama_ps_url: &str, client: &Client) -> LlmMetrics {
    let response = client.get(ollama_ps_url).send().await;
    let Ok(resp) = response else {
        return LlmMetrics {
            ollama_online: false,
            ollama_ps_url: ollama_ps_url.to_string(),
            running_models: Vec::new(),
            model_count: 0,
            error: Some("request failed".to_string()),
        };
    };

    let parsed = resp.json::<Value>().await;
    let Ok(body) = parsed else {
        return LlmMetrics {
            ollama_online: false,
            ollama_ps_url: ollama_ps_url.to_string(),
            running_models: Vec::new(),
            model_count: 0,
            error: Some("invalid JSON from ollama".to_string()),
        };
    };

    let running_models = body
        .get("models")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.get("name").and_then(|v| v.as_str()))
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    LlmMetrics {
        ollama_online: true,
        ollama_ps_url: ollama_ps_url.to_string(),
        model_count: running_models.len(),
        running_models,
        error: None,
    }
}

fn root_disk_usage() -> (u64, u64) {
    let disks = Disks::new_with_refreshed_list();

    let selected = disks
        .iter()
        .find(|disk| disk.mount_point() == Path::new("/"))
        .or_else(|| disks.iter().next());

    if let Some(disk) = selected {
        let total = disk.total_space();
        let avail = disk.available_space();
        let used = total.saturating_sub(avail);
        (total, used)
    } else {
        (0, 0)
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

fn start_file_watcher(
    watch_dirs: &[String],
    events: Arc<Mutex<VecDeque<String>>>,
) -> Result<RecommendedWatcher> {
    let mut watcher = RecommendedWatcher::new(
        move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                let mut queue = events.lock().expect("file event queue lock");
                let paths = event
                    .paths
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                let line = format!("{} [{}] {}", now_ms(), format!("{:?}", event.kind), paths);
                queue.push_back(line);
                while queue.len() > 200 {
                    queue.pop_front();
                }
            }
        },
        Config::default(),
    )?;

    for dir in watch_dirs {
        let path = Path::new(dir);
        if path.exists() {
            if let Err(err) = watcher.watch(path, RecursiveMode::Recursive) {
                eprintln!("watch error for {}: {err}", path.display());
            }
        }
    }

    Ok(watcher)
}

fn read_recent_events(events: &Arc<Mutex<VecDeque<String>>>, limit: usize) -> Vec<String> {
    let queue = events.lock().expect("file event queue lock");
    queue.iter().rev().take(limit).cloned().collect::<Vec<_>>()
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    eprintln!("received shutdown signal");
}
