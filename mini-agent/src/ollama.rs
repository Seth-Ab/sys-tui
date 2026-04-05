use std::collections::HashSet;

use reqwest::Client;
use serde_json::Value;

use crate::models::LlmMetrics;

pub async fn collect_llm_metrics(ollama_ps_url: &str, client: &Client) -> LlmMetrics {
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

pub async fn fetch_running_models(client: &Client, ollama_ps_url: &str) -> Vec<String> {
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

pub async fn fetch_installed_models(client: &Client, ollama_tags_url: &str) -> Vec<String> {
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
