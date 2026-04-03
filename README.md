# sys-tui

Split Rust TUI system for a Mac mini (agent) and Mac Pro (viewer).

- `mini-agent`: runs on the Mac mini, collects system + LLM + file-event telemetry, serves JSON.
- `pro-tui`: runs on the Mac Pro, polls the agent and renders a live dashboard.

## 1) Run the mini agent (on Mac mini)

```bash
cd /Users/sethabramowitz/cs-projects/sys-tui
cargo run -p mini-agent
```

Optional env vars:

- `AGENT_BIND` (default `127.0.0.1:8787`)
- `AGENT_TOKEN` (optional auth token required in `x-agent-token` header)
- `OLLAMA_PS_URL` (default `http://127.0.0.1:11434/api/ps`)
- `WATCH_DIRS` (comma-separated dirs, default `$HOME/Downloads,$HOME/.ollama`)

## 2) Run the pro dashboard (on Mac Pro)

```bash
cd /Users/sethabramowitz/cs-projects/sys-tui
AGENT_ENDPOINT=http://<mini-host-or-tailnet-ip>:8787/state cargo run -p pro-tui
```

If using a token:

```bash
AGENT_ENDPOINT=http://<mini-host-or-tailnet-ip>:8787/state AGENT_TOKEN=<same-token> cargo run -p pro-tui
```

## TUI keybinds (pro-tui)

- `q`: quit
- `r`: refresh now
- `e`: expand/collapse file events
- `h`: show/hide help

## Suggested deploy model

- Run `mini-agent` in `tmux` or as a `launchd` service on the mini.
- Connect over Tailscale instead of exposing to LAN.
