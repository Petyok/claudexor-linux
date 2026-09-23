#!/bin/sh
# User-local install: binary, launcher entry, icon. Re-run after `cargo build --release`.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
bin="${XDG_BIN_HOME:-$HOME/.local/bin}"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
install -Dm755 "$here/target/release/claudexor-linux" "$bin/claudexor-linux"
install -Dm644 "$here/packaging/claudexor-linux.desktop" "$data/applications/claudexor-linux.desktop"
install -Dm644 "$here/packaging/claudexor-linux.svg" "$data/icons/hicolor/scalable/apps/claudexor-linux.svg"
command -v update-desktop-database >/dev/null && update-desktop-database "$data/applications" || true
echo "installed: $bin/claudexor-linux (make sure $bin is on PATH)"
