#!/usr/bin/env bash
# Prints the wasm-bindgen crate version locked by the bindings. The
# wasm-bindgen CLI must match it exactly, so this is the version to install.
# Fails if the bindings' lockfiles disagree.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

locked_version() {
    # Strip \r so a CRLF checkout (e.g. on Windows) still matches.
    awk '{ sub(/\r$/, "") }
         /^name = "wasm-bindgen"$/ { found = 1; next }
         found && /^version = / { gsub(/"/, "", $3); print $3; exit }' "$1"
}

locks=""
for lock in "$root"/bindings/*/Cargo.lock; do
    [ -f "$lock" ] || continue
    v="$(locked_version "$lock")"
    [ -n "$v" ] || continue
    locks+="${lock#"$root/"} $v"$'\n'
done

if [ -z "$locks" ]; then
    echo "no Cargo.lock under bindings/ depends on wasm-bindgen" >&2
    exit 1
fi

versions="$(printf '%s' "$locks" | awk '{ print $2 }' | sort -u)"
if [ "$(printf '%s\n' "$versions" | wc -l)" -gt 1 ]; then
    echo "bindings lock different wasm-bindgen versions:" >&2
    printf '%s' "$locks" | awk '{ printf "  %-40s %s\n", $1, $2 }' >&2
    exit 1
fi

echo "$versions"
