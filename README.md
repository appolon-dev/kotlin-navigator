# Kotlin Navigator for Zed (experimental MVP)

A tiny LSP for **Go to Definition of Kotlin and Java class-like symbols across modules**. It never calls Gradle, Kotlin compiler, JVM or Android Studio. Standard-library Python 3.9+. It relies on source scanning; **not a semantic Kotlin compiler**.

## Install on macOS

1. Keep the ordinary **Kotlin** Zed extension installed.
2. Open Terminal, enter this folder and run `./install.sh`. It installs `navigator.py` into `~/.local/share/kotlin-navigator/` and prints your exact Zed settings.
3. Merge the printed JSON into `~/.config/zed/settings.json` (do not replace other unrelated settings).
4. Restart Zed, open the **repository root**, open any `.kt` file, then try `Cmd + Click` on a class from another module.

No Gradle import is performed; the index is built from all `.kt`/`.java` source files in the workspace in a background thread. The first lookup may block briefly until indexing finishes. Use `Cmd + Shift + P` → `dev: open language server logs` to troubleshoot.

### Settings template (edit your user paths; installer prints correct ones)

```json
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
        "path": "/path/to/python3",
        "arguments": ["-u", "/Users/YOU/.local/share/kotlin-navigator/navigator.py"]
      }
    }
  }
}
```

This uses Zed's documented per-LSP binary override to run our program under the ID of the already installed Kotlin extension's alternate LSP. That avoids writing a separate Rust/WASM Zed extension for the first prototype. **Zed integration is unverified on your machine**, so if the extension ignores the binary override, we can create a dedicated extension wrapper.

## Capabilities

- `textDocument/definition`: map `Name` to a class/interface/object/typealias/enum/record in `.kt` or `.java`.
- `workspace/symbol`: fuzzy substring search of class names across repo.
- Explicit import, import alias, same file/package and wildcard-import heuristics.
- Live unsaved buffers (`didOpen`, full-sync `didChange`) and `didSave`/`didClose` refreshes.
- Returns multiple candidates for ambiguous names instead of silently choosing the wrong one.

## Limitations

- Regular-expression index: may mistake class names in comments or strings for declarations, and may not understand nested classes, Kotlin complex syntax and multicline class modifiers.
- No Gradle variant graph, type inference, overload resolution, generics, generated sources, dependencies from AAR/JAR, or reliable method/property navigation.
- Ignores the `build/` and `generated/` directories. Index refresh of external on-disk changes relies on editor LSP file-watch notifications; restarting Zed always refreshes.
- Does not verify import visibility or module dependencies. With common names used in different modules it may return several candidates.
- Index operation is multi-threaded and intended for quick personal experiments, not production use.

## Diagnostics

Set environment variable `KOTLIN_NAVIGATOR_DEBUG=1` in the `binary.env` section to print indexing stats to LSP stderr. All protocol messages go to stdout only.

## Run tests

```bash
python3 -m unittest discover -s tests -v
```
