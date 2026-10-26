# Installing Kotlin Navigator in Zed

Kotlin Navigator comes as a Zed dev extension. The extension downloads the prebuilt navigator for your Mac from [GitHub Releases](https://github.com/appolon-dev/kotlin-navigator/releases) and keeps it up to date; you never build the navigator itself.

## 1. Requirements

- macOS on Apple Silicon or Intel.
- [Zed](https://zed.dev) 0.205 or newer.
- Rust 1.85 or newer installed with **rustup** (<https://rustup.rs>); if yours is older, run `rustup update`. Zed needs it only to compile the small extension and adds the `wasm32-wasip2` target itself. A Rust from Homebrew does not build dev extensions.

## 2. Zed extensions

In Zed, open the Extensions panel (`Cmd + Shift + X`) and install **Kotlin** and **Java**. They provide the syntax for `.kt` and `.java` files; Kotlin Navigator only adds the language server.

## 3. Clone and install the extension

```bash
git clone https://github.com/appolon-dev/kotlin-navigator.git
```

In Zed, run `zed: install dev extension` from the command palette (`Cmd + Shift + P`) and choose the `zed-extension` folder of the clone. Keep the clone: Zed builds the extension from it.

## 4. Settings

Run `zed: open settings file` from the command palette (it opens `~/.config/zed/settings.json`) and merge this next to your other settings:

```json
{
  "languages": {
    "Kotlin": { "language_servers": ["kotlin-navigator", "!kotlin-language-server", "!kotlin-lsp", "..."] },
    "Java": { "language_servers": ["kotlin-navigator", "!jdtls", "..."] }
  }
}
```

Kotlin Navigator comes first, so Zed asks it for Go to Definition. The JVM language servers of the Kotlin and Java extensions are turned off: they are slow to start on large Android projects and would answer the same requests.

## 5. First run

1. Open the **repository root** of your project (`File → Open`), not a single module.
2. Open a `.kt` file. The first time, the status bar shows the navigator being downloaded (about 2 MB). Indexing takes 1–2 s for 5 000 files.
3. `Cmd + Click` a class from another module.

## 6. Updating

- **The navigator** updates itself: on every Zed start the extension checks for a newer release and downloads it.
- **The extension**: `git pull` in the clone, then **Rebuild** on Kotlin Navigator in the Extensions panel.

## 7. Troubleshooting

- **Logs.** `Cmd + Shift + P` → `dev: open language server logs`, then pick `kotlin-navigator`. For more detail, set the level (`error`, `warn` by default, `info`, `debug`, `trace`):

  ```json
  { "lsp": { "kotlin-navigator": { "binary": { "env": { "KOTLIN_NAVIGATOR_LOG": "info" } } } } }
  ```

- **`Kotlin Navigator: could not download the navigator (…). Put kotlin-navigator on PATH or set lsp.kotlin-navigator.binary.path.`** The first download failed: no network, GitHub's API limit (60 requests an hour per address), or downloads blocked. Once one version is downloaded, the extension falls back to it while offline. If you restricted `granted_extension_capabilities`, allow `download_file` from `github.com`.
- **`Kotlin Navigator: no prebuilt binary for … . Put kotlin-navigator on PATH or set lsp.kotlin-navigator.binary.path.`** Releases exist for macOS only. Build the navigator yourself (see the README) and point to it as below.
- **`feature edition2024 is required` when installing the extension.** Your Rust is older than 1.85: run `rustup update`, then install the dev extension again.
- **Where the download lives:** `~/Library/Application Support/Zed/extensions/work/kotlin-navigator/kotlin-navigator-<version>/`.
- **Your own build.** `binary.path` wins over the download; `kotlin-navigator` on your `PATH` wins too:

  ```json
  { "lsp": { "kotlin-navigator": { "binary": { "path": "/path/to/kotlin-navigator", "arguments": [] } } } }
  ```

- **Go to Definition goes to the wrong server.** Check that `kotlin-navigator` is first in `language_servers` and that no project-level `.zed/settings.json` overrides it.

## 8. Migrating from the manual setup

If you used Kotlin Navigator through the `kotlin-language-server` binary override:

1. Remove the `"kotlin-language-server": { "binary": … }` block from `lsp` in your settings.
2. Replace your `languages.Kotlin.language_servers` with the snippet of step 4.
3. Optionally delete `~/.local/share/kotlin-navigator`. It also holds `navigator.py`, the Python version that the README's rollback uses.

## 9. Uninstalling

1. Uninstall Kotlin Navigator in the Extensions panel.
2. Remove `kotlin-navigator` and the `!…` entries from `language_servers`, and any `lsp.kotlin-navigator` block.
3. Delete the clone.
