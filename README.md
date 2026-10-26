# Kotlin Navigator for Zed

A small language server for **Go to Definition across modules in large Kotlin/Java (Android, KMP) projects**. It never runs Gradle, the Kotlin compiler, a JVM or Android Studio: it parses sources with tree-sitter and keeps a compact declaration index in memory.

This is **stage 2** of the Rust rewrite (designs: `docs/superpowers/specs/2026-10-25-rust-navigator-design.md`, `docs/superpowers/specs/2026-10-26-rust-navigator-stage2-design.md`). Stage 1 reached parity with the original Python prototype, now kept in `legacy/`; stage 2 adds members and locals.

## What it does

- `textDocument/definition` for class-like symbols: classes of every kind, interfaces (incl. `fun interface`), objects, companion objects, enum classes and their entries, type aliases, Java records and annotation types. Nested classes (`Outer.Inner`) are resolved through their full name.
- Go to functions, properties, parameters and local variables (incl. `it`, `for`/`catch`/`when` variables and destructuring); top-level functions and properties are found across modules and through their imports.
- The type of a receiver comes from a declared type (`val repo: Repo?`) or a constructor call (`val r = Repo(x)`). `repo.load()`, `Foo.create()` (companion), `this.x`, `super.x` and `x?.y` look among the type's members, then its supertypes (5 levels deep), then extension functions and properties declared on them.
- Source sets and Gradle modules come from the path: `androidMain` code is not sent to an `iosMain` declaration while a compatible one exists, same-module candidates come first, `internal` ones of other modules last.
- `expect`/`actual`: an `expect` brings its `actual`s, the one for the current source set first.
- Candidate ranking: an explicit import (incl. `as` aliases), then the same file, the same package, wildcard imports; otherwise all candidates (up to 50). `private` declarations of other files are never offered. Clicking inside an `import` line jumps to exactly that declaration.
- `workspace/symbol`: case-insensitive substring search, exact and prefix matches first, up to 100 results.
- Unsaved buffers win over files on disk (`didOpen`, full-sync `didChange`); `didClose`, `didSave` and `workspace/didChangeWatchedFiles` refresh from disk. An open `build.gradle.kts` can navigate but is not indexed.
- The workspace walk honours `.gitignore`, `.ignore`, `.git/info/exclude` and the global gitignore, skips hidden directories (so `.claude/worktrees` is not indexed) and `build`, `out`, `target`, `bin`, `node_modules`, `generated`, plus files over 2 MB. Files tracked by git are indexed even if an ignore rule matches them, except inside `build`, `out` and the other always-skipped directories.
- While the first scan runs, requests are answered from the part indexed so far once 10 s have passed; a crash in the scan still lets requests through.

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

## Limitations

- No type inference, generics or call chains: `foo().bar` and `it.bar` fall back to a ranked search by name. Overloads are not told apart: every `load(…)` of a class is offered.
- Named arguments and lambdas with receivers (`apply {}`, `with(x) {}`) are not resolved; they fall back to the search by name.
- No synthetic properties for Java getters (`getFoo()` as `foo`). From Java files, members are found by name with ranking only.
- No Gradle variant graph, generated sources or classes from AAR/JAR dependencies. An explicitly imported declaration that is not in the workspace (a library class) yields no result rather than a same-named project one.
- A parse that runs past its time budget (250 ms plus 2 ms per KB; a half-typed backtick can keep the Kotlin grammar busy for minutes) is abandoned: an open buffer keeps its previous version and an indexed file its previous declarations until the text parses again.
- Find Usages, `$/cancelRequest`, an on-disk cache and file-watch registration come in later stages.

## Diagnostics

Logs go to stderr; view them with `Cmd + Shift + P` → `dev: open language server logs`. Set the level with `KOTLIN_NAVIGATOR_LOG` (`error`, `warn` by default, `info`, `debug`, `trace`) in the `binary.env` section:

```json
"binary": { "path": "...", "arguments": [], "env": { "KOTLIN_NAVIGATOR_LOG": "info" } }
```

## Benchmark

`kotlin-navigator --bench <root>` indexes `<root>` once and prints file and declaration counts, files with parse errors, cold indexing time and peak memory. Then it runs Go to Definition on 1 000 identifiers picked at random (fixed seed 42) from the indexed Kotlin files and prints the share of lookups with no, one and several targets, and the median and 95th-percentile lookup time.

Measured on a real KMP project (2026-10-26, Apple Silicon, 10 cores):

| | Python prototype | Rust, stage 1 | Rust, stage 2 |
|---|---|---|---|
| files indexed | 42 150 (walked into 18 git worktrees) | 4 924 | 4 924 |
| declarations | | | 47 530 |
| cold index | 34.8 s | 0.75–1.3 s | 0.88–1.41 s |
| peak memory | 147 MB | 32–34 MB | 54–58 MB (incl. sampling) |
| unambiguous definition (one target) | | | 57.6 % (none: 35.7 %, several: 6.7 %) |
| definition time, p50 / p95 | | | 16–18 µs / 66–70 µs |

Most lookups without a target are library and generated names (`String`, `Serializable`, `Modifier`, DI DSL functions such as `single`, generated database and resource classes) and `null`/`true`/`false`, which the grammar reports as identifiers.

## Tests

```bash
cargo test
python3 -m unittest discover -s legacy/tests -v   # Python reference
```

Two legacy tests fail on macOS because the prototype resolves `/var` to `/private/var` while the tests compare unresolved temp paths; that behaviour is left as it was.

## Rolling back to Python

`install.sh` leaves `~/.local/share/kotlin-navigator/navigator.py` in place. Point `binary.path` back to `python3` with `"arguments": ["-u", "/Users/YOU/.local/share/kotlin-navigator/navigator.py"]`; `legacy/install.sh` reinstalls it.
