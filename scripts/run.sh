#!/usr/bin/env bash
set -euo pipefail

ENV=${ENV:-dev}
CONFIG_FLAG=${CONFIG_FLAG:-}

echo "[run] ENV=${ENV}"
exec cargo run --bin server -- ${CONFIG_FLAG:+-f "$CONFIG_FLAG"}
