default:
    @just --list

# Build ossm-alt-m57aim example for ESP32-S3
build-ossm-alt-m57aim:
    cargo +esp build -p ossm-alt-m57aim --target xtensa-esp32s3-none-elf -Z build-std=alloc,core

# Flash ossm-alt-m57aim to a connected ESP32-S3
flash-ossm-alt-m57aim:
    cargo +esp build -p ossm-alt-m57aim --target xtensa-esp32s3-none-elf -Z build-std=alloc,core
    espflash flash --monitor target/xtensa-esp32s3-none-elf/debug/ossm-alt-m57aim

# Build all targets
build-all: build-ossm-alt-m57aim
