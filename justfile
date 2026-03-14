set windows-shell := ["powershell.exe", "-NoLogo", "-c"]
set dotenv-load := true

default:
    @just --list

# OSSM Alt (ESP32-S3)
[working-directory: 'firmware/ossm-alt']
build-ossm-alt:
    cargo +esp build --release

[working-directory: 'firmware/ossm-alt']
flash-ossm-alt:
    cargo +esp run --release

# M5CoreS3 Simulator (ESP32-S3)
[working-directory: 'firmware/sim-m5cores3']
build-m5cores3:
    cargo +esp build --release
    
[working-directory: 'firmware/sim-m5cores3']
flash-m5cores3:
    cargo +esp run --release

# WASM Simulator
build-wasm:
    wasm-pack build firmware/sim-wasm --target web

# Dev server (watches Rust sources and hot-reloads WASM)
[working-directory: 'apps/simulator']
dev-patterns: build-wasm
    pnpm dev

# All
[parallel]
build-all: build-ossm-alt build-wasm build-m5cores3

# Focus rust-analyzer on a firmware crate by symlinking its .cargo to the workspace root
[unix]
focus crate:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ ! -d "firmware/{{ crate }}/.cargo" ]; then
        echo "Error: firmware/{{ crate }}/.cargo does not exist"
        exit 1
    fi
    if [ -d .cargo ] && [ ! -L .cargo ]; then
        echo "Error: .cargo exists and is not a symlink, refusing to remove"
        exit 1
    fi
    rm -f .cargo
    ln -sn "firmware/{{ crate }}/.cargo" .cargo
    ln -sn "firmware/{{ crate }}/rust-toolchain.toml" rust-toolchain.toml
    echo "rust-analyzer focused on {{ crate }}"


# Focus rust-analyzer on a firmware crate by copying its .cargo to the workspace root
[windows]
focus crate:
    # Uses copy instead of symlink because symlinks on Windows require elevated privileges
    if (-not (Test-Path "firmware/{{ crate }}/.cargo" -PathType Container)) { Write-Error "firmware/{{ crate }}/.cargo does not exist"; exit 1 }
    if (Test-Path ".cargo") { Remove-Item ".cargo" -Recurse -Force }
    if (Test-Path "rust-toolchain.toml") { Remove-Item "rust-toolchain.toml" -Force }
    Copy-Item -Path "firmware/{{ crate }}/.cargo" -Destination ".cargo" -Recurse
    Copy-Item -Path "firmware/{{ crate }}/rust-toolchain.toml" -Destination "rust-toolchain.toml"
    Write-Host "rust-analyzer focused on {{ crate }}"
