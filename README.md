# sys-tui

Split Rust TUI system for a Mac mini (agent) and Mac Pro (viewer).

- `mini-agent`: runs on the Mac mini, collects system + LLM + file-event telemetry, serves JSON.
- `pro-tui`: runs on the Mac Pro, polls the agent and renders a live dashboard.

## Quick Start (scripts)

1. Create local config:
```bash
cd sys-tui
cp .env.local.example .env.local
```

2. Edit `.env.local` with your values (`AGENT_BIND`, `AGENT_ENDPOINT`, `AGENT_TOKEN`, etc).

3. On Mac mini, run agent:
```bash
cd sys-tui
./scripts/run-mini-agent.sh
```

4. On Mac Pro, run dashboard:
```bash
cd sys-tui
./scripts/run-pro-tui.sh
```

## Quick Start (manual env)

### 1) On Mac mini: start `mini-agent`

```bash
cd sys-tui
TOKEN="$(openssl rand -hex 32)"
AGENT_BIND=0.0.0.0:8787 AGENT_TOKEN="$TOKEN" cargo run -p mini-agent
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

Notes:
- `AGENT_ENDPOINT` is a URL, so hostname is fine here (for example `core`, `core.local`, or Tailscale DNS name), as long as it resolves.
- Example: `AGENT_ENDPOINT=http://core:8787/state`

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
- `OLLAMA_CHAT_URL` default: `http://127.0.0.1:11434/api/generate`
- `OLLAMA_TAGS_URL` default: `http://127.0.0.1:11434/api/tags`
- `OLLAMA_MODEL` default: `llama3.2` (fallback chat model)

### `pro-tui`
- `AGENT_ENDPOINT` default: `http://127.0.0.1:8787/state`
- `AGENT_TOKEN` optional; required if agent token is enabled
- `PRO_TUI_CONFIG` optional absolute path for persistent local config
  - default path: `~/.config/pro-tui/config.toml`

## Keybinds (`pro-tui`)

### Global (all screens)
- `q`: quit
- `Tab`: open screen navigator
- `h`: open help
- `Esc`: close active modal/input

### Dashboard screen
- `r`: refresh now
- `e`: expand/collapse file events
- `i`: focus chat input
- `m`: open model selector
- `Up/Down`: scroll chat

### Chat input mode
- `Enter`: send prompt
- `Esc`: exit input focus
- `Backspace`: delete character

### Model selector modal
- `Up/Down`: move
- `Enter`: apply model
- `Esc`: close selector

### Customize screen
- `[ / ]`: switch section (`Global`, `Dashboard`)
- `Up/Down`: move option
- `Left/Right`: change value
- `Enter`: edit/apply selected option
- `s`: save config to disk
- `r`: reset selected option

## Customize persistence

- Edits in `Customize` are applied live in memory.
- They become permanent only after pressing `s` to save.
- Save uses an atomic write (temp file then rename).
- Unknown config keys are preserved for forward compatibility.

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
