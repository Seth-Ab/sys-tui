use std::collections::{HashMap, HashSet, VecDeque};
use std::env;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sysinfo::{Disks, System};
use tokio::sync::RwLock;

const PROCESS_WINDOW_MS: u128 = 5 * 60 * 1000;
const TOP_PROCESS_COUNT: usize = 5;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ProcessInfo {
    pid: u32,
    name: String,
    cpu_percent: f32,
    current_cpu_percent: f32,
    memory_bytes: u64,
    samples_5m: u32,
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
    client: Client,
    ollama_chat_url: String,
    ollama_model: String,
    ollama_ps_url: String,
    ollama_tags_url: String,
}

#[derive(Debug, Clone)]
struct ProcessSample {
    ts_ms: u128,
    cpu_percent: f32,
    memory_bytes: u64,
}

#[derive(Debug, Clone)]
struct ProcessHistoryEntry {
    pid: u32,
    name: String,
    samples: VecDeque<ProcessSample>,
}

type ProcessHistory = HashMap<String, ProcessHistoryEntry>;

#[derive(Debug, Deserialize)]
struct ChatRequest {
    prompt: String,
    model: Option<String>,
}

#[derive(Debug, Serialize)]
struct ChatResponse {
    response: String,
    model: String,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct ModelsResponse {
    selected_model: String,
    running_models: Vec<String>,
    installed_models: Vec<String>,
    error: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let bind_addr = env::var("AGENT_BIND").unwrap_or_else(|_| "127.0.0.1:8787".to_string());
    let token = env::var("AGENT_TOKEN").ok().filter(|v| !v.is_empty());
    let watch_dirs = configured_watch_dirs();
    let ollama_ps_url =
        env::var("OLLAMA_PS_URL").unwrap_or_else(|_| "http://127.0.0.1:11434/api/ps".to_string());
    let ollama_tags_url = env::var("OLLAMA_TAGS_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:11434/api/tags".to_string());
    let ollama_chat_url = env::var("OLLAMA_CHAT_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:11434/api/generate".to_string());
    let ollama_model = env::var("OLLAMA_MODEL").unwrap_or_else(|_| "llama3.2".to_string());

    let events = Arc::new(Mutex::new(VecDeque::with_capacity(200)));
    let _watcher = start_file_watcher(&watch_dirs, Arc::clone(&events))?;

    let snapshot = Arc::new(RwLock::new(AgentState {
        watched_dirs: watch_dirs.clone(),
        ..Default::default()
    }));

    let client = Client::new();

    let collector_snapshot = Arc::clone(&snapshot);
    let collector_events = Arc::clone(&events);
    let collector_client = client.clone();
    let collector_ps_url = ollama_ps_url.clone();
    tokio::spawn(async move {
        let mut seq: u64 = 0;
        let mut system = System::new_all();
        let mut process_history: ProcessHistory = HashMap::new();

        loop {
            seq = seq.saturating_add(1);
            let next = collect_snapshot(
                seq,
                &collector_ps_url,
                &collector_client,
                &watch_dirs,
                &collector_events,
                &mut system,
                &mut process_history,
            )
            .await;
            *collector_snapshot.write().await = next;
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/state", get(state_handler))
        .route("/chat", post(chat_handler))
        .route("/models", get(models_handler))
        .with_state(AppCtx {
            token,
            snapshot,
            client,
            ollama_chat_url,
            ollama_model,
            ollama_ps_url,
            ollama_tags_url,
        });

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
    authorize(&ctx, &headers)?;
    let snapshot = ctx.snapshot.read().await.clone();
    Ok(Json(snapshot))
}

async fn chat_handler(
    State(ctx): State<AppCtx>,
    headers: HeaderMap,
    Json(payload): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, StatusCode> {
    authorize(&ctx, &headers)?;

    if payload.prompt.trim().is_empty() {
        return Ok(Json(ChatResponse {
            response: String::new(),
            model: ctx.ollama_model.clone(),
            error: Some("prompt is empty".to_string()),
        }));
    }

    let selected_model = payload
        .model
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .unwrap_or(&ctx.ollama_model)
        .to_string();

    let req_body = json!({
        "model": selected_model,
        "prompt": payload.prompt,
        "stream": false
    });

    let response = ctx
        .client
        .post(&ctx.ollama_chat_url)
        .json(&req_body)
        .send()
        .await;
    let Ok(resp) = response else {
        return Ok(Json(ChatResponse {
            response: String::new(),
            model: selected_model,
            error: Some("failed to reach ollama".to_string()),
        }));
    };

    let parsed = resp.json::<Value>().await;
    let Ok(body) = parsed else {
        return Ok(Json(ChatResponse {
            response: String::new(),
            model: selected_model,
            error: Some("invalid response from ollama".to_string()),
        }));
    };

    let model = body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or(&ctx.ollama_model)
        .to_string();

    let answer = body
        .get("response")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .or_else(|| {
            body.get("message")
                .and_then(|v| v.get("content"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
        })
        .unwrap_or_default();

    Ok(Json(ChatResponse {
        response: answer,
        model,
        error: None,
    }))
}

async fn models_handler(
    State(ctx): State<AppCtx>,
    headers: HeaderMap,
) -> Result<Json<ModelsResponse>, StatusCode> {
    authorize(&ctx, &headers)?;

    let running_models = fetch_running_models(&ctx.client, &ctx.ollama_ps_url).await;
    let installed_models = fetch_installed_models(&ctx.client, &ctx.ollama_tags_url).await;

    let error = if installed_models.is_empty() {
        Some("no installed models found or ollama unavailable".to_string())
    } else {
        None
    };

    Ok(Json(ModelsResponse {
        selected_model: ctx.ollama_model.clone(),
        running_models,
        installed_models,
        error,
    }))
}

fn authorize(ctx: &AppCtx, headers: &HeaderMap) -> Result<(), StatusCode> {
    if let Some(expected) = &ctx.token {
        let provided = headers
            .get("x-agent-token")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();

        if provided != expected {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }
    Ok(())
}

async fn collect_snapshot(
    seq: u64,
    ollama_ps_url: &str,
    client: &Client,
    watch_dirs: &[String],
    events: &Arc<Mutex<VecDeque<String>>>,
    system: &mut System,
    process_history: &mut ProcessHistory,
) -> AgentState {
    let ts_ms = now_ms();
    system.refresh_all();

    let cpu_percent = system.global_cpu_usage();
    let memory_total_bytes = system.total_memory();
    let memory_used_bytes = system.used_memory();
    let swap_total_bytes = system.total_swap();
    let swap_used_bytes = system.used_swap();

    update_process_history(system, process_history, ts_ms);
    let top_processes = compute_top_processes(process_history, ts_ms);

    let (root_total_bytes, root_used_bytes) = root_disk_usage();

    let llm = collect_llm_metrics(ollama_ps_url, client).await;

    AgentState {
        seq,
        ts_ms,
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

fn update_process_history(system: &System, history: &mut ProcessHistory, ts_ms: u128) {
    for (pid, proc_) in system.processes() {
        let key = process_key(pid.as_u32(), proc_.start_time());
        let entry = history.entry(key).or_insert_with(|| ProcessHistoryEntry {
            pid: pid.as_u32(),
            name: proc_.name().to_string_lossy().into_owned(),
            samples: VecDeque::new(),
        });

        entry.pid = pid.as_u32();
        entry.name = proc_.name().to_string_lossy().into_owned();
        entry.samples.push_back(ProcessSample {
            ts_ms,
            cpu_percent: proc_.cpu_usage(),
            memory_bytes: proc_.memory(),
        });

        prune_old_samples(&mut entry.samples, ts_ms);
    }

    history.retain(|_, entry| {
        prune_old_samples(&mut entry.samples, ts_ms);
        !entry.samples.is_empty()
    });
}

fn compute_top_processes(history: &ProcessHistory, ts_ms: u128) -> Vec<ProcessInfo> {
    let mut out: Vec<ProcessInfo> = history
        .values()
        .filter_map(|entry| {
            let valid_samples: Vec<&ProcessSample> = entry
                .samples
                .iter()
                .filter(|s| ts_ms.saturating_sub(s.ts_ms) <= PROCESS_WINDOW_MS)
                .collect();

            if valid_samples.is_empty() {
                return None;
            }

            let sum_cpu: f32 = valid_samples.iter().map(|s| s.cpu_percent).sum();
            let avg_cpu = sum_cpu / valid_samples.len() as f32;
            let current = valid_samples.last().copied()?;

            Some(ProcessInfo {
                pid: entry.pid,
                name: entry.name.clone(),
                cpu_percent: avg_cpu,
                current_cpu_percent: current.cpu_percent,
                memory_bytes: current.memory_bytes,
                samples_5m: valid_samples.len() as u32,
            })
        })
        .collect();

    out.sort_by(|a, b| {
        b.cpu_percent
            .total_cmp(&a.cpu_percent)
            .then_with(|| b.current_cpu_percent.total_cmp(&a.current_cpu_percent))
    });
    out.truncate(TOP_PROCESS_COUNT);
    out
}

fn prune_old_samples(samples: &mut VecDeque<ProcessSample>, now_ms: u128) {
    while samples
        .front()
        .map(|s| now_ms.saturating_sub(s.ts_ms) > PROCESS_WINDOW_MS)
        .unwrap_or(false)
    {
        samples.pop_front();
    }
}

fn process_key(pid: u32, start_time: u64) -> String {
    format!("{pid}:{start_time}")
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

async fn fetch_running_models(client: &Client, ollama_ps_url: &str) -> Vec<String> {
    let resp = client.get(ollama_ps_url).send().await;
    let Ok(resp) = resp else {
        return Vec::new();
    };
    let body = resp.json::<Value>().await;
    let Ok(body) = body else {
        return Vec::new();
    };

    body.get("models")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.get("name").and_then(|v| v.as_str()))
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

async fn fetch_installed_models(client: &Client, ollama_tags_url: &str) -> Vec<String> {
    let resp = client.get(ollama_tags_url).send().await;
    let Ok(resp) = resp else {
        return Vec::new();
    };
    let body = resp.json::<Value>().await;
    let Ok(body) = body else {
        return Vec::new();
    };

    let models = body
        .get("models")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.get("name").and_then(|v| v.as_str()))
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    dedupe_keep_order(models)
}

fn dedupe_keep_order(models: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for model in models {
        if seen.insert(model.clone()) {
            out.push(model);
        }
    }
    out
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
                let line = format!("{} [{:?}] {}", now_ms(), event.kind, paths);
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
