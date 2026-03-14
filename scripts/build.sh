#!/usr/bin/env bash
set -euo pipefail

ENV=${ENV:-dev}
echo "[build] ENV=${ENV}"

cargo build --release --bin server
echo "[build] Done: target/release/server"
