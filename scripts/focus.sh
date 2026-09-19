#!/usr/bin/env bash
set -euo pipefail

crate="$1"
motor="${2:-}"

case "$crate" in
    esp32s3)
        default_motor="rs485"
        indicator_feature="indicator-ws2812b"
        ;;
    esp32)
        default_motor="stepdir"
        indicator_feature="indicator-ws2812b"
        ;;
    *)
        echo "Error: unknown arch '$crate'" >&2
        echo "Valid arches: esp32, esp32s3" >&2
        exit 1
        ;;
esac

motor="${motor:-$default_motor}"
feature="motor-${motor}"

jq --arg proj "firmware/${crate}/Cargo.toml" --arg feat "$feature" \
   --arg indicator "$indicator_feature" \
   '. + {
     "rust-analyzer.linkedProjects": [$proj],
     "rust-analyzer.cargo.features": [$feat, $indicator] | map(select(length > 0))
   }' .vscode/settings.template.json > .vscode/settings.json

jq --arg proj "firmware/${crate}/Cargo.toml" --arg feat "$feature" \
   --arg indicator "$indicator_feature" \
   '.lsp["rust-analyzer"].initialization_options.linkedProjects = [$proj]
    | .lsp["rust-analyzer"].initialization_options.cargo.features =
        ([$feat, $indicator] | map(select(length > 0)))' \
   .zed/settings.template.json > .zed/settings.json

features="$feature${indicator_feature:+,$indicator_feature}"
echo "rust-analyzer focused on ${crate} with --features ${features}"
