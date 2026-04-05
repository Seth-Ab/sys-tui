use super::*;

pub(super) fn poll_state(client: &Client, app: &mut App) {
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

pub(super) fn open_model_selector(client: &Client, app: &mut App) {
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

pub(super) fn send_prompt(
    client: &Client,
    app: &mut App,
    tx: &Sender<ChatJobResult>,
    prompt: String,
) {
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
