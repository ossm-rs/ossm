default:
    @just --list

esp_target := "xtensa-esp32s3-none-elf"
esp_flags := "--target " + esp_target + " -Z build-std=alloc,core --release"

# OSSM Alt
build-ossm-alt:
    cargo +esp build -p ossm-alt-m57aim {{esp_flags}}

flash-ossm-alt: build-ossm-alt
    espflash flash --monitor target/{{esp_target}}/release/ossm-alt-m57aim

# M5CoreS3 Simulator
build-m5cores3:
    cargo +esp build -p sim-m5cores3 {{esp_flags}}

flash-m5cores3: build-m5cores3
    espflash flash --monitor target/{{esp_target}}/release/sim-m5cores3

# WASM Simulator
build-wasm:
    wasm-pack build firmware/sim-wasm --target web

# All
build-all: build-ossm-alt build-wasm build-m5cores3
