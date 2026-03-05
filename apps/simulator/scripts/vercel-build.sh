#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"

# Install Rust (minimal profile, no prompts)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
source "$HOME/.cargo/env"

# Install wasm-pack
curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh

# Build the WASM package
cd "$REPO_ROOT"
wasm-pack build firmware/sim-wasm --target web

# Install dependencies and build the web app
cd "$REPO_ROOT/apps/simulator"
pnpm install
pnpm run build
