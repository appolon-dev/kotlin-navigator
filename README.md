# Kotlin Navigator for Zed

A small language server for **Go to Definition across modules in large Kotlin/Java (Android, KMP) projects**. It never runs Gradle, the Kotlin compiler, a JVM or Android Studio: it parses sources with tree-sitter and keeps a compact declaration index in memory.

This is **stage 1** of the Rust rewrite (design: `docs/superpowers/specs/2026-10-25-rust-navigator-design.md`). It reaches parity with the original Python prototype, now kept in `legacy/`.

## What stage 1 does

- `textDocument/definition` for class-like symbols: classes of every kind, interfaces (incl. `fun interface`), objects, companion objects, enum classes and their entries, type aliases, Java records and annotation types. Nested classes (`Outer.Inner`) are resolved through their full name.
- Candidate ranking: an explicit import (incl. `as` aliases), then the same file, the same package, wildcard imports; otherwise all candidates (up to 50). Clicking inside an `import` line jumps to exactly that class.
- `workspace/symbol`: case-insensitive substring search, exact and prefix matches first, up to 100 results.
- Unsaved buffers win over files on disk (`didOpen`, full-sync `didChange`); `didClose`, `didSave` and `workspace/didChangeWatchedFiles` refresh from disk. An open `build.gradle.kts` can navigate but is not indexed.
- The workspace walk honours `.gitignore`, `.ignore`, `.git/info/exclude` and the global gitignore, skips hidden directories (so `.claude/worktrees` is not indexed) and `build`, `out`, `target`, `bin`, `node_modules`, `generated`, plus files over 2 MB. Files tracked by git are always indexed, even if an ignore rule matches them.

## Install on macOS

1. Install Rust from <https://rustup.rs> if `cargo` is missing.
2. Keep the ordinary **Kotlin** Zed extension installed.
3. Run `./install.sh`. It builds a release binary, copies it to `~/.local/share/kotlin-navigator/kotlin-navigator` and prints the Zed settings to merge.
4. Merge the snippet into `~/.config/zed/settings.json` without replacing unrelated settings. Keep `"arguments": []`: otherwise the arguments of the old Python setup (`-u .../navigator.py`) survive the merge and the server refuses to start.
5. Restart Zed, open the **repository root**, then `Cmd + Click` a class from another module.

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
        "path": "/Users/YOU/.local/share/kotlin-navigator/kotlin-navigator",
        "arguments": []
      }
    }
  }
}
```

This uses Zed's per-server binary override to run the navigator under the ID of the Kotlin extension's language server.

## Limitations of stage 1

- Only class-like symbols. Functions, properties, parameters and locals, Find Usages, an on-disk cache and file-watch registration come in later stages.
- No type inference, overload resolution, Gradle variant graph, generated sources or classes from AAR/JAR dependencies. An explicitly imported class that is not in the workspace (a library class) yields no result rather than a same-named project class.

## Diagnostics

Logs go to stderr; view them with `Cmd + Shift + P` → `dev: open language server logs`. Set the level with `KOTLIN_NAVIGATOR_LOG` (`error`, `warn` by default, `info`, `debug`, `trace`) in the `binary.env` section:

```json
"binary": { "path": "...", "arguments": [], "env": { "KOTLIN_NAVIGATOR_LOG": "info" } }
```

## Benchmark

`kotlin-navigator --bench <root>` indexes `<root>` once and prints file and declaration counts, files with parse errors, cold indexing time and peak memory.

Measured on a real KMP project (2026-10-26, Apple Silicon, 10 cores):

| | Python prototype | Rust, stage 1 |
|---|---|---|
| files indexed | 42 150 (walked into 18 git worktrees) | 4 924 |
| cold index | 34.8 s | 0.75–1.3 s |
| peak memory | 147 MB | 32–34 MB |

## Tests

```bash
cargo test
python3 -m unittest discover -s legacy/tests -v   # Python reference
```

Two legacy tests fail on macOS because the prototype resolves `/var` to `/private/var` while the tests compare unresolved temp paths; that behaviour is left as it was.

## Rolling back to Python

`install.sh` leaves `~/.local/share/kotlin-navigator/navigator.py` in place. Point `binary.path` back to `python3` with `"arguments": ["-u", "/Users/YOU/.local/share/kotlin-navigator/navigator.py"]`; `legacy/install.sh` reinstalls it.
