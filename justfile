default:
    @just --list

build:
    cargo +esp build -p ossm-alt-m57aim --target xtensa-esp32s3-none-elf -Z build-std=alloc,core --release

flash:
    cargo +esp build -p ossm-alt-m57aim --target xtensa-esp32s3-none-elf -Z build-std=alloc,core --release
    espflash flash --monitor target/xtensa-esp32s3-none-elf/release/ossm-alt-m57aim

build-wasm:
    wasm-pack build firmware/sim-wasm --target web

build-m5cores3:
    cargo +esp build -p sim-m5cores3 --target xtensa-esp32s3-none-elf -Z build-std=alloc,core --release

flash-m5cores3:
    cargo +esp build -p sim-m5cores3 --target xtensa-esp32s3-none-elf -Z build-std=alloc,core --release
    espflash flash --monitor target/xtensa-esp32s3-none-elf/release/sim-m5cores3

build-all: build build-wasm build-m5cores3
