{
  description = "OSSM — firmware, web tooling, and WASM simulators";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      systems = [
        "x86_64-darwin"
        "aarch64-darwin"
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      devShells = forAllSystems (system:
        let
          pkgs = import nixpkgs {
            inherit system;
            config.allowUnfree = true;
          };

          # VSCode wrapper: keeps user-data and extensions inside the repo
          # under .vscode-local/, so the project-specific install stays
          # isolated from any system-wide VSCode.
          vscode-local = pkgs.writeShellScriptBin "code" ''
            ROOT="''${OSSM_PROJECT_ROOT:-$PWD}"
            USER_DATA="$ROOT/.vscode-local/user-data"
            EXT_DIR="$ROOT/.vscode-local/extensions"
            mkdir -p "$USER_DATA" "$EXT_DIR"
            exec ${pkgs.vscode}/bin/code \
              --user-data-dir "$USER_DATA" \
              --extensions-dir "$EXT_DIR" \
              "$@"
          '';
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              # Task runner and shell helpers (justfile + doctor.sh)
              just
              jq

              # Editor: launches with project-local config.
              vscode-local

              # Rust: rustup manages the stable + esp toolchains and the
              # wasm32-unknown-unknown target. Per-crate rust-toolchain.toml
              # files pick the right channel automatically.
              rustup

              # ESP32 firmware toolchain. Run `espup install` once to create
              # ~/export-esp.sh; the shellHook below sources it on entry.
              espup
              espflash

              # WASM build pipeline. wasm-bindgen-cli must match the
              # wasm-bindgen crate version in Cargo.lock (currently 0.2.118).
              # If `just build-wasm` complains about a version mismatch, run:
              #   cargo install wasm-bindgen-cli --version 0.2.118 --locked
              wasm-bindgen-cli
              binaryen # provides wasm-opt

              # Web tooling for apps/web-tools and apps/docs.
              nodejs_24
              pnpm
            ];

            shellHook = ''
              export OSSM_PROJECT_ROOT="$PWD"
              if [ -f "$HOME/export-esp.sh" ]; then
                . "$HOME/export-esp.sh" 2>/dev/null || true
              fi
            '';
          };
        }
      );
    };
}
