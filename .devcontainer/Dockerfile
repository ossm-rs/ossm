FROM mcr.microsoft.com/devcontainers/rust:2-1-trixie

# Install just
RUN curl --proto '=https' --tlsv1.2 -sSf https://just.systems/install.sh | bash -s -- --to /usr/local/bin

# Install nvm + Node LTS Krypton (v24)
ENV NVM_DIR=/usr/local/share/nvm
RUN mkdir -p "$NVM_DIR" \
    && curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.3/install.sh | bash \
    && . "$NVM_DIR/nvm.sh" \
    && nvm install lts/krypton \
    && nvm alias default lts/krypton \
    && nvm use default

# Install pnpm and create a stable symlink for node binaries
RUN . "$NVM_DIR/nvm.sh" \
    && corepack enable pnpm \
    && ln -s "$(dirname "$(which node)")" "$NVM_DIR/current"

ENV PATH="$NVM_DIR/current:$PATH"
ENV COREPACK_ENABLE_DOWNLOAD_PROMPT=0

# Add wasm target and install build tools. wasm-bindgen-cli is installed in
# postCreateCommand instead, as it must match the bindings' Cargo.lock.
RUN rustup toolchain install stable --target wasm32-unknown-unknown \
    && apt-get update && apt-get install -y --no-install-recommends binaryen && rm -rf /var/lib/apt/lists/*

# ESP Xtensa toolchain (+esp) for the esp32/esp32s3 firmware, at the version
# CI uses (.esp-toolchain-version). The justfile sources ~/export-esp.sh for
# the linker and libclang paths.
ARG ESPUP_VERSION=0.17.1
COPY .esp-toolchain-version /tmp/
RUN curl -fsSL "https://github.com/esp-rs/espup/releases/download/v${ESPUP_VERSION}/espup-$(uname -m)-unknown-linux-gnu" -o /usr/local/bin/espup \
    && chmod +x /usr/local/bin/espup \
    && espup install --targets esp32,esp32s3 --toolchain-version "$(tr -d '\r\n' < /tmp/.esp-toolchain-version)" --export-file /home/vscode/export-esp.sh

# espflash, e.g. for `espflash save-image`. Same version as ESPFLASH_VERSION in
# .github/workflows/build-firmware.yml.
ARG ESPFLASH_VERSION=4.3.0
RUN curl -fsSL "https://github.com/esp-rs/espflash/releases/download/v${ESPFLASH_VERSION}/espflash-$(uname -m)-unknown-linux-gnu.zip" -o /tmp/espflash.zip \
    && unzip -o /tmp/espflash.zip -d /usr/local/bin \
    && rm /tmp/espflash.zip
