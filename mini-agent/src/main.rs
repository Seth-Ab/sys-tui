mod api;
mod collector;
mod config;
mod models;
mod ollama;
mod watcher;

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use axum::Router;
use axum::routing::{get, post};
use reqwest::Client;
use tokio::sync::RwLock;

use crate::api::{AppCtx, chat_handler, health_handler, models_handler, state_handler};
use crate::collector::SnapshotCollector;
use crate::config::AgentConfig;
use crate::models::AgentState;
use crate::watcher::start_file_watcher;

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = AgentConfig::from_env();

    let events = Arc::new(Mutex::new(VecDeque::with_capacity(200)));
    let _watcher = start_file_watcher(&cfg.watch_dirs, Arc::clone(&events))?;

    let snapshot = Arc::new(RwLock::new(AgentState {
        watched_dirs: cfg.watch_dirs.clone(),
        ..Default::default()
    }));

    let client = Client::new();

    let collector_snapshot = Arc::clone(&snapshot);
    let collector_events = Arc::clone(&events);
    let collector_client = client.clone();
    let collector_watch_dirs = cfg.watch_dirs.clone();
    let collector_ps_url = cfg.ollama_ps_url.clone();
    tokio::spawn(async move {
        let mut seq: u64 = 0;
        let mut collector = SnapshotCollector::new();

        loop {
            seq = seq.saturating_add(1);
            let next = collector
                .collect_snapshot(
                    seq,
                    &collector_ps_url,
                    &collector_client,
                    &collector_watch_dirs,
                    &collector_events,
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
            token: cfg.token,
            snapshot,
            client,
            ollama_chat_url: cfg.ollama_chat_url,
            ollama_model: cfg.ollama_model,
            ollama_ps_url: cfg.ollama_ps_url,
            ollama_tags_url: cfg.ollama_tags_url,
        });

    let addr: SocketAddr = cfg.bind_addr.parse()?;
    println!("mini-agent listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    eprintln!("received shutdown signal");
}
