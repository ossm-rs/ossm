{
  description = "OSSM - Rust firmware, WASM and web tools";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        lib = pkgs.lib;

        # The wasm-bindgen CLI must exactly match the crate version the
        # bindings lock, so take it from their Cargo.lock files (as
        # scripts/wasm-bindgen-version.sh does) rather than nixpkgs' latest.
        wasmBindgenVersion =
          let
            bindings = lib.attrNames (lib.filterAttrs (_: type: type == "directory") (builtins.readDir ./bindings));
            locked = lib.concatMap (dir:
              let lock = ./bindings + "/${dir}/Cargo.lock"; in
              if builtins.pathExists lock then
                map (p: { inherit dir; inherit (p) version; })
                  (lib.filter (p: p.name == "wasm-bindgen") (builtins.fromTOML (builtins.readFile lock)).package)
              else [ ]) bindings;
            versions = lib.unique (map (l: l.version) locked);
          in
          if versions == [ ] then
            throw "no Cargo.lock under bindings/ depends on wasm-bindgen"
          else if builtins.length versions > 1 then
            throw "bindings lock different wasm-bindgen versions: ${lib.concatMapStringsSep ", " (l: "bindings/${l.dir} ${l.version}") locked}"
          else
            builtins.head versions;

        wasmBindgenAttr = "wasm-bindgen-cli_${builtins.replaceStrings [ "." ] [ "_" ] wasmBindgenVersion}";
        wasmBindgenCli =
          if pkgs.wasm-bindgen-cli.version == wasmBindgenVersion then pkgs.wasm-bindgen-cli
          else pkgs.${wasmBindgenAttr} or (throw
            "nixpkgs has no ${wasmBindgenAttr} for the bindings' wasm-bindgen ${wasmBindgenVersion}; run `nix flake update` or change the bindings' wasm-bindgen version");

        # Pinned VSCodium extensions, bump with `nix flake update`
        pinnedExtensions = with pkgs.vscode-extensions; [
          rust-lang.rust-analyzer
          tamasfe.even-better-toml
          vadimcn.vscode-lldb
          streetsidesoftware.code-spell-checker
          jnoortheen.nix-ide
          dbaeumer.vscode-eslint
          esbenp.prettier-vscode
        ];

        pinnedExtensionsDir = pkgs.symlinkJoin {
          name = "ossm-vscode-extensions";
          paths = pinnedExtensions;
        };

        # `codium` wrapper: keeps user-data and extensions under
        # .vscode-local/ so VS Codium state is project-local, and seeds the
        # extensions dir with recommended defaults.
        codium-local = pkgs.writeShellScriptBin "codium" ''
          set -eu
          ROOT="''${OSSM_VSCODE_ROOT:-$PWD}"
          EXT_DIR="$ROOT/.vscode-local/extensions"
          UD_DIR="$ROOT/.vscode-local/user-data"
          mkdir -p "$EXT_DIR" "$UD_DIR"
          for src in ${pinnedExtensionsDir}/share/vscode/extensions/*; do
            ln -sfn "$src" "$EXT_DIR/$(basename "$src")"
          done
          exec ${pkgs.vscodium}/bin/codium \
            --user-data-dir "$UD_DIR" \
            --extensions-dir "$EXT_DIR" \
            "$@"
        '';

        # `gh` wrapper: keeps auth under .git/gh so logins are scoped to this
        # repo (never committed, shared across worktrees). Tokens are stored in
        # that dir rather than the system keyring, which is keyed only by
        # host+user and would otherwise be shared with the global gh login.
        gh-local = pkgs.writeShellScriptBin "gh" ''
          set -eu
          if common="$(${pkgs.git}/bin/git rev-parse --path-format=absolute --git-common-dir 2>/dev/null)"; then
            export GH_CONFIG_DIR="$common/gh"
          fi
          if [ "''${1-}" = auth ] && [ "''${2-}" = login ]; then
            shift 2
            exec ${pkgs.gh}/bin/gh auth login --insecure-storage "$@"
          fi
          exec ${pkgs.gh}/bin/gh "$@"
        '';

        # Shared libs the cadquery-ocp/vtk wheels dlopen. Exposed via a
        # dedicated var (not LD_LIBRARY_PATH) so only the CAD recipes see them.
        cadLibPath = pkgs.lib.makeLibraryPath (with pkgs; [
          libGL
          libx11
          libxrender
          expat
          zlib
          stdenv.cc.cc.lib
        ]);
      in {
        devShells.default = pkgs.mkShell {
          OSSM_CAD_LD_LIBRARY_PATH = cadLibPath;

          packages = with pkgs; [
            just

            rustup

            espup
            espflash

            wasmBindgenCli
            binaryen

            nodejs_22
            pnpm

            jq
            gh-local

            # CAD tooling (tools/cad): uv manages Python + OCP wheels
            uv

            # Editor
            codium-local
          ];

          shellHook = ''
            # Route git's github.com HTTPS credentials through the repo-scoped
            # gh wrapper. Set via env-based git config so it only applies in
            # this shell and never writes a nix store path into a config file.
            # The empty value clears any inherited github.com helpers first.
            n="''${GIT_CONFIG_COUNT:-0}"
            export "GIT_CONFIG_KEY_$n=credential.https://github.com.helper"
            export "GIT_CONFIG_VALUE_$n="
            export "GIT_CONFIG_KEY_$((n + 1))=credential.https://github.com.helper"
            export "GIT_CONFIG_VALUE_$((n + 1))=!${gh-local}/bin/gh auth git-credential"
            export GIT_CONFIG_COUNT="$((n + 2))"
            unset n

            # Ensure the stable toolchain and wasm32 target are present so that
            # `just doctor`'s rustup checks succeed.
            if ! rustup toolchain list 2>/dev/null | grep -q '^stable'; then
              echo "Installing stable Rust toolchain via rustup..."
              rustup toolchain install stable --profile minimal
            fi
            if ! rustup +stable target list --installed 2>/dev/null | grep -q '^wasm32-unknown-unknown$'; then
              echo "Adding wasm32-unknown-unknown target..."
              rustup +stable target add wasm32-unknown-unknown
            fi

            # The ESP Rust toolchain is too large/custom to package in Nix; espup
            # downloads it on demand. Prompt the user to run it once if missing.
            if [ ! -f "$HOME/export-esp.sh" ] || ! cargo +esp --version >/dev/null 2>&1; then
              cat <<'EOF'

ESP toolchain not installed. Run once:
    espup install --export-file "$HOME/export-esp.sh"

EOF
            fi
          '';
        };
      });
}
