use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use reqwest::Client;
use serde_json::{Value, json};
use tokio::sync::RwLock;

use crate::models::{AgentState, ChatRequest, ChatResponse, ModelsResponse};
use crate::ollama::{fetch_installed_models, fetch_running_models};

#[derive(Clone)]
pub struct AppCtx {
    pub token: Option<String>,
    pub snapshot: Arc<RwLock<AgentState>>,
    pub client: Client,
    pub ollama_chat_url: String,
    pub ollama_model: String,
    pub ollama_ps_url: String,
    pub ollama_tags_url: String,
}

pub async fn health_handler() -> &'static str {
    "ok"
}

pub async fn state_handler(
    State(ctx): State<AppCtx>,
    headers: HeaderMap,
) -> Result<Json<AgentState>, StatusCode> {
    authorize(&ctx, &headers)?;
    let snapshot = ctx.snapshot.read().await.clone();
    Ok(Json(snapshot))
}

pub async fn chat_handler(
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

pub async fn models_handler(
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
