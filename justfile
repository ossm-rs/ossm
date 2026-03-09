default:
    @just --list

# OSSM Stock (ESP32)
build-ossm-stock:
    cd firmware/ossm-stock && cargo +esp build --release

flash-ossm-stock:
    cd firmware/ossm-stock && cargo +esp run --release

# OSSM Alt (ESP32-S3)
build-ossm-alt:
    cd firmware/ossm-alt-m57aim && cargo +esp build --release

flash-ossm-alt:
    cd firmware/ossm-alt-m57aim && cargo +esp run --release

# M5CoreS3 Simulator (ESP32-S3)
build-m5cores3:
    cd firmware/sim-m5cores3 && cargo +esp build --release

flash-m5cores3:
    cd firmware/sim-m5cores3 && cargo +esp run --release

# WASM Simulator
build-wasm:
    wasm-pack build firmware/sim-wasm --target web

# Dev server (watches Rust sources and hot-reloads WASM)
dev-patterns: build-wasm
    cd apps/simulator && pnpm dev

# All
build-all: build-ossm-stock build-ossm-alt build-wasm build-m5cores3

# Focus rust-analyzer on a specific target (esp32, esp32s3, wasm)
focus target:
    #!/usr/bin/env bash
    case "{{ target }}" in
        esp32)
            ra_target="xtensa-esp32-none-elf"
            ra_features='["esp32"]'
            ;;
        esp32s3)
            ra_target="xtensa-esp32s3-none-elf"
            ra_features='["esp32s3"]'
            ;;
        wasm)
            ra_target="wasm32-unknown-unknown"
            ra_features='[]'
            ;;
        *)
            echo "Unknown target '{{ target }}'. Valid targets:"
            echo "  esp32   — Xtensa ESP32"
            echo "  esp32s3 — Xtensa ESP32-S3"
            echo "  wasm    — WASM simulator"
            exit 1
            ;;
    esac
    cat > rust-analyzer.toml << SETTINGS
    [cargo]
    target = "$ra_target"
    features = $ra_features
    allTargets = false
    SETTINGS
    echo "rust-analyzer focused on {{ target }} ($ra_target)"
