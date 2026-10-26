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
cp target/release/kotlin-navigator "$DIR/kotlin-navigator"
printf '\nInstalled %s\n' "$DIR/kotlin-navigator"
printf '\nMerge this snippet into ~/.config/zed/settings.json, then restart Zed.\n'
printf 'Keep "arguments": [] so arguments of the old Python setup are dropped.\n\n'
cat <<EOF
{
  "languages": {
    "Kotlin": {
      "enable_language_server": true,
      "language_servers": ["kotlin-language-server", "!kotlin-lsp"]
    }
  },
  "lsp": {
    "kotlin-language-server": {
      "binary": {
        "path": "$DIR/kotlin-navigator",
        "arguments": []
      }
    }
  }
}
EOF
