#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

# Load optional local env file (kept out of git).
if [[ -f .env.local ]]; then
  # shellcheck disable=SC1091
  source .env.local
fi

AGENT_ENDPOINT="${AGENT_ENDPOINT:-}"
AGENT_TOKEN="${AGENT_TOKEN:-}"

if [[ -z "$AGENT_ENDPOINT" ]]; then
  echo "AGENT_ENDPOINT is required (example: http://core:8787/state)"
  exit 1
fi

echo "Starting pro-tui"
echo "  AGENT_ENDPOINT=${AGENT_ENDPOINT}"
if [[ -n "$AGENT_TOKEN" ]]; then
  echo "  AGENT_TOKEN is set"
else
  echo "  AGENT_TOKEN is not set"
fi

AGENT_ENDPOINT="$AGENT_ENDPOINT" \
AGENT_TOKEN="$AGENT_TOKEN" \
cargo run -p pro-tui
