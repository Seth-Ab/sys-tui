# sys-tui

Split Rust TUI system for a Mac mini (agent) and Mac Pro (viewer).

- `mini-agent`: runs on the Mac mini, collects system + LLM + file-event telemetry, serves JSON.
- `pro-tui`: runs on the Mac Pro, polls the agent and renders a live dashboard.

## Quick Start (with token)

### 1) On Mac mini: start `mini-agent`

```bash
cd sys-tui
TOKEN="$(openssl rand -hex 32)"
AGENT_BIND=<ipv4>:8787 AGENT_TOKEN="$TOKEN" cargo run -p mini-agent
```

Notes:
- `AGENT_BIND` must be an IP:port (not a hostname). Examples:
  - `127.0.0.1:8787` (local only)
  - `0.0.0.0:8787` (all interfaces)
  - `100.x.y.z:8787` (specific Tailscale IP)
- Keep the token value; Mac Pro must use the exact same token.

### 2) On Mac Pro: run `pro-tui`

```bash
cd sys-tui
AGENT_ENDPOINT=http://<mini-host-or-ip>:8787/state AGENT_TOKEN="<same-token>" cargo run -p pro-tui
```

## Without token

If you do not set `AGENT_TOKEN` on `mini-agent`, then `pro-tui` can connect without a token:

```bash
AGENT_ENDPOINT=http://<mini-host-or-ip>:8787/state cargo run -p pro-tui
```

## Environment variables

### `mini-agent`
- `AGENT_BIND` default: `127.0.0.1:8787`
- `AGENT_TOKEN` optional shared secret (expects `x-agent-token` header)
- `OLLAMA_PS_URL` default: `http://127.0.0.1:11434/api/ps`
- `WATCH_DIRS` comma-separated dirs (default: `$HOME/Downloads,$HOME/.ollama`)

### `pro-tui`
- `AGENT_ENDPOINT` default: `http://127.0.0.1:8787/state`
- `AGENT_TOKEN` optional; required if agent token is enabled

## Keybinds (`pro-tui`)

- `q`: quit
- `r`: refresh now
- `e`: expand/collapse file events
- `h`: show/hide help

## Troubleshooting

- `401 Unauthorized`:
  - `mini-agent` has `AGENT_TOKEN` set, but `pro-tui` token is missing/wrong.
- `request failed` / timeout:
  - wrong host/IP in `AGENT_ENDPOINT`, or network path blocked.
- `core:8787` works in endpoint but not bind:
  - expected. `AGENT_BIND` needs IP:port; `AGENT_ENDPOINT` can use hostnames.

## Suggested deploy model

- Run `mini-agent` in `tmux` or as a `launchd` service on the mini.
- Prefer Tailscale over open LAN exposure.
