#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

# Load optional local env file (kept out of git).
if [[ -f .env.local ]]; then
  # shellcheck disable=SC1091
  source .env.local
fi

AGENT_BIND="${AGENT_BIND:-127.0.0.1:8787}"
AGENT_TOKEN="${AGENT_TOKEN:-}"
OLLAMA_PS_URL="${OLLAMA_PS_URL:-http://127.0.0.1:11434/api/ps}"
WATCH_DIRS="${WATCH_DIRS:-${HOME}/Downloads,${HOME}/.ollama}"

echo "Starting mini-agent"
echo "  AGENT_BIND=${AGENT_BIND}"
echo "  OLLAMA_PS_URL=${OLLAMA_PS_URL}"
echo "  WATCH_DIRS=${WATCH_DIRS}"
if [[ -n "$AGENT_TOKEN" ]]; then
  echo "  AGENT_TOKEN is set"
else
  echo "  AGENT_TOKEN is not set"
fi

AGENT_BIND="$AGENT_BIND" \
AGENT_TOKEN="$AGENT_TOKEN" \
OLLAMA_PS_URL="$OLLAMA_PS_URL" \
WATCH_DIRS="$WATCH_DIRS" \
cargo run -p mini-agent
