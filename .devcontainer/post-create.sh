#!/usr/bin/env bash
# Called as a single word from postCreateCommand: Zed joins lifecycle commands
# without quoting, so anything with spaces there breaks.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo install wasm-bindgen-cli --locked --version "$(bash scripts/wasm-bindgen-version.sh)"
[ -f .vscode/settings.json ] || [ -f .zed/settings.json ] || just focus esp32s3
(cd apps/web-tools && pnpm install --force)
