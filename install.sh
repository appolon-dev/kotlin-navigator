#!/bin/sh
set -eu
if ! command -v python3 >/dev/null 2>&1; then
  echo 'Python 3 is required. Install Python 3 before proceeding.' >&2
  exit 1
fi
DIR="$HOME/.local/share/kotlin-navigator"
mkdir -p "$DIR"
cp "$(dirname "$0")/navigator.py" "$DIR/navigator.py"
chmod 700 "$DIR/navigator.py"
PYTHON="$(command -v python3)"
printf '\nInstalled navigator in %s\n' "$DIR/navigator.py"
printf '\nMerge this snippet into ~/.config/zed/settings.json, then restart Zed:\n\n'
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
        "path": "$PYTHON",
        "arguments": ["-u", "$DIR/navigator.py"]
      }
    }
  }
}
EOF
