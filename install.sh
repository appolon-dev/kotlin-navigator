#!/bin/sh
set -eu
if ! command -v cargo >/dev/null 2>&1; then
  echo 'cargo not found: install Rust from https://rustup.rs' >&2
  exit 1
fi
cd "$(dirname "$0")"
cargo build --release
DIR="$HOME/.local/share/kotlin-navigator"
mkdir -p "$DIR"
# Replace by rename: overwriting a binary that already ran makes macOS kill new launches of it.
cp target/release/kotlin-navigator "$DIR/kotlin-navigator.new"
mv -f "$DIR/kotlin-navigator.new" "$DIR/kotlin-navigator"
printf '\nInstalled %s\n' "$DIR/kotlin-navigator"
printf '\nThis build runs through the Kotlin Navigator Zed extension: install it first (INSTALL.md, steps 1-3).\n'
printf 'Then merge this snippet into ~/.config/zed/settings.json and restart Zed.\n\n'
cat <<EOF
{
  "languages": {
    "Kotlin": { "language_servers": ["kotlin-navigator", "!kotlin-language-server", "!kotlin-lsp", "..."] },
    "Java": { "language_servers": ["kotlin-navigator", "!jdtls", "..."] }
  },
  "lsp": {
    "kotlin-navigator": {
      "binary": {
        "path": "$DIR/kotlin-navigator",
        "arguments": []
      }
    }
  }
}
EOF
