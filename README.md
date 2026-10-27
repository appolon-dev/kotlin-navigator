# Kotlin Navigator for Zed

A small language server for **Go to Definition across modules in large Kotlin/Java (Android, KMP) projects**. It never runs Gradle, the Kotlin compiler, a JVM or Android Studio: it parses sources with tree-sitter and keeps a compact declaration index in memory.

This is **stage 4** of the Rust rewrite. Stage 1 reached parity with the original Python prototype, now kept in `legacy/`; stage 2 added members and locals; stage 3 added Find All References, implementations, type definition and highlighting; stage 4 adds the Zed extension and prebuilt macOS binaries.

## What it does

- `textDocument/definition` for class-like symbols: classes of every kind, interfaces (incl. `fun interface`), objects, companion objects, enum classes and their entries, type aliases, Java records and annotation types. Nested classes (`Outer.Inner`) are resolved through their full name.
- Go to functions, properties, parameters and local variables (incl. `it`, `for`/`catch`/`when` variables and destructuring); top-level functions and properties are found across modules and through their imports.
- The type of a receiver comes from a declared type (`val repo: Repo?`) or a constructor call (`val r = Repo(x)`). `repo.load()`, `Foo.create()` (companion), `this.x`, `super.x` and `x?.y` look among the type's members, then its supertypes (5 levels deep), then extension functions and properties declared on them.
- Source sets and Gradle modules come from the path: `androidMain` code is not sent to an `iosMain` declaration while a compatible one exists, same-module candidates come first, `internal` ones of other modules last.
- `expect`/`actual`: an `expect` brings its `actual`s, the one for the current source set first.
- Candidate ranking: an explicit import (incl. `as` aliases), then the same file, the same package, wildcard imports; otherwise all candidates (up to 50). `private` declarations of other files are never offered. Clicking inside an `import` line jumps to exactly that declaration.
- Library sources: a name the workspace lacks is looked up in the `*-sources.jar` files of the Gradle cache (`$GRADLE_USER_HOME`, else `~/.gradle`) and in the Android SDK's `sources/android-N` with the highest N (`$ANDROID_HOME`, `$ANDROID_SDK_ROOT`, else `~/Library/Android/sdk`). It covers import lines, explicitly imported names and simple names through wildcard and default imports (`kotlin.*`, `kotlin.collections.*`, `java.lang.*` and the rest), classes as well as top-level functions and properties. One version per artifact is used: the one written in the workspace's `gradle/*.toml` version catalogs, else the latest downloaded; artifacts from the catalogs win when artifacts share a package (`okhttp` over `okhttp-jvm`). A jar entry is extracted read-only to `~/Library/Caches/kotlin-navigator/sources` (`$XDG_CACHE_HOME`, else `~/.cache`, off macOS) on the first jump to it, and navigation goes on from there. The jars are listed in the background at start (about 1.3 s for 700 jars); a name is searched once and remembered.
- On the first Go to Definition into a library jar of a session, before answering, the rest of the jar's sources are extracted and the library folder is added to the current Zed project (`zed -a`), where the project panel and `Cmd + P` list the library's files. The editor then opens the file as one of the project's, so the project panel shows it; opened before the folder was added, it would stay a file of its own. Zed asks for definitions on `Cmd` + hover too, so hovering over a library name adds its folder as well. The project's roots are passed along, since `zed -a` adds to the project holding one of the paths, else to an arbitrary one of the window's projects. The same is offered again by `textDocument/codeAction` in such a file, `Show library <name> in the project panel` (command `kotlin-navigator.openLibrary`): for a library removed from the project meanwhile. Remove the folder with the project panel's `Remove from Project`. The Zed CLI is `$ZED_CLI`, else `zed` on `PATH`, else `Zed.app/Contents/MacOS/cli` in `/Applications` or `~/Applications`. Zed tasks cannot do this: they get no `$ZED_FILE` for a file outside the project's folders. To open the menu with one key, bind `editor::ToggleCodeActions` (`"alt-f1": "editor::ToggleCodeActions"` in the `Editor` context), then press `Enter`.
- `textDocument/references` (Find All References): usages across all files of the project, scoped by visibility (`private` to its file, `internal` to its module, locals to the file). The search covers the declaration's family: `override`s up and down the hierarchy and `expect` together with all its `actual`s, members of an `expect class` included. It also finds named-argument labels of a parameter (`f(count = 1)`), `$name` and `${name}` in string templates, and usages from Java files. Open buffers are searched from memory.
- `textDocument/implementation` (Go to Implementation): subclasses and implementors of a class or interface, `override`s of a method, `actual`s of an `expect` (members of an `expect class` included).
- `textDocument/typeDefinition` (Go to Type Definition): the type of a variable, parameter or property, or the result type of a function.
- `textDocument/documentHighlight`: exact usages of the symbol under the cursor in the current file; the declaration and assignment targets are marked as writes, the rest as reads.
- `$/cancelRequest`: a cancelled search stops between files and answers with error `-32800`.
- `Cmd + Click` on a named-argument label (`count = 1`) goes to the parameter.
- `workspace/symbol`: case-insensitive substring search, exact and prefix matches first, up to 100 results.
- Unsaved buffers win over files on disk (`didOpen`, full-sync `didChange`); `didClose`, `didSave` and `workspace/didChangeWatchedFiles` refresh from disk. An open `build.gradle.kts` can navigate but is not indexed.
- The workspace walk honours `.gitignore`, `.ignore`, `.git/info/exclude` and the global gitignore, skips hidden directories (so `.claude/worktrees` is not indexed) and `build`, `out`, `target`, `bin`, `node_modules`, `generated`, plus files over 2 MB. Files tracked by git are indexed even if an ignore rule matches them, except inside `build`, `out` and the other always-skipped directories.
- While the first scan runs, requests are answered from the part indexed so far once 10 s have passed; a crash in the scan still lets requests through.

## Install on macOS

See **[INSTALL.md](INSTALL.md)**: install the Kotlin Navigator Zed dev extension from `zed-extension/`, and it downloads the prebuilt navigator for your Mac (Apple Silicon or Intel) from GitHub Releases and keeps it up to date. No Rust build of the navigator is needed.

**From source.** Install the dev extension as above, then run `./install.sh`: it builds a release binary, copies it to `~/.local/share/kotlin-navigator/kotlin-navigator` and prints the settings to merge, which point `lsp.kotlin-navigator.binary.path` at it. The extension runs that binary instead of downloading one.

## Limitations

- No type inference, generics or call chains: `foo().bar` and `it.bar` fall back to a ranked search by name. Overloads are not told apart: every `load(…)` of a class is offered.
- Named-argument labels are resolved, but lambdas with receivers (`apply {}`, `with(x) {}`) are not; they fall back to the search by name.
- Usages found by name (`foo().bar`, Java files, a name missing from a project type whose supertypes are library classes, as `activity.toast()` with `class MyActivity : AppCompatActivity()`) may include unrelated places, and Zed does not tell them from exact ones.
- Anonymous objects (`object : Base {}`) are not counted as implementations.
- Generic arguments are not parsed in Go to Type Definition: `List<Repo>` leads to `List`. A variable initialized by a function call (`val x = createRepo()`) has no type definition; one initialized by a constructor call (`val x = Repo()`) has.
- Java getters are not synthetic properties (`getFoo()` is not found as `foo`), and named arguments of `copy(…)` are not resolved. From Java files, members are found by name with ranking only.
- Java methods are not recognized as overrides of Kotlin members (`@Override` is not read): Go to Implementation and Find Usages leave them out.
- Extensions on `Any` or on a type parameter (`fun <T> T.log()`) are not found through a receiver of a project type: `a.log()` has no definition. It is missing from the extension's usages when the type's supertypes are all in the project; with a library supertype it is listed as found by name.
- Go to Definition on a member of an `expect class` does not offer its `actual`s, and entries of an `actual enum class` are not paired with the `expect` entries.
- Usages through an import alias (`import a.B as C`, then `C`) are not found.
- No Gradle variant graph or generated sources. Libraries are reached only through sources jars Gradle has downloaded: there is no decompiler for class files, no JDK `src.zip`, and the version is a guess when the catalogs do not pin one. Library declarations are only targets of Go to Definition: members (`client.newCall()`) and Find All References into libraries are not resolved. A name the project also declares (a `String` property somewhere) is resolved to the project's declarations by name and never reaches the library.
- A parse that runs past its time budget (250 ms plus 2 ms per KB; a half-typed backtick can keep the Kotlin grammar busy for minutes) is abandoned: an open buffer keeps its previous version and an indexed file its previous declarations until the text parses again.
- An on-disk cache and file-watch registration come in later stages.

## Diagnostics

Logs go to stderr; view them with `Cmd + Shift + P` → `dev: open language server logs`. Set the level with `KOTLIN_NAVIGATOR_LOG` (`error`, `warn` by default, `info`, `debug`, `trace`) in `lsp.kotlin-navigator.binary.env`; it applies to the downloaded binary too:

```json
{ "lsp": { "kotlin-navigator": { "binary": { "env": { "KOTLIN_NAVIGATOR_LOG": "info" } } } } }
```

## Benchmark

`kotlin-navigator --bench <root>` indexes `<root>` once and prints file and declaration counts, files with parse errors, cold indexing time and peak memory. Then it runs Go to Definition on 1 000 identifiers picked at random (fixed seed 42) from the indexed Kotlin files and prints the share of lookups with no, one and several targets, and the median and 95th-percentile lookup time (`definition_*`).

Stage 3 adds two groups of lines. `references_*`: Find All References on 200 random declarations (seed 42; a random Kotlin file, a random declaration in it), run on a pool of the server's size (4 threads). Each sample is timed as the server does it, including the copy of the index, but not the parsing of the current file; the lines are the sample count, median, 95th percentile and maximum in ms, and `references_probable_pct`, the share of found places marked `Probable` (found by name) among all of them. `highlight_*`: the median, 95th percentile and maximum in µs of the document highlight at the same 1 000 positions as `definition_*`.

Measured on a real KMP project (stages 1–2: 2026-10-26; stage 3: three release runs on 2026-10-26 after the final-review fixes; Apple Silicon, 10 cores):

| | Python prototype | Rust, stage 1 | Rust, stage 2 | Rust, stage 3 |
|---|---|---|---|---|
| files indexed | 42 150 (walked into 18 git worktrees) | 4 924 | 4 924 | 4 924 |
| declarations | | | 47 530 | 47 530 |
| cold index | 34.8 s | 0.75–1.3 s | 0.88–1.41 s | 0.86–1.29 s |
| peak memory | 147 MB | 32–34 MB | 54–58 MB (incl. sampling) | 71–72 MB (incl. sampling) |
| unambiguous definition (one target) | | | 57.6 % (none: 35.7 %, several: 6.7 %) | 59.2 % (none: 36.8 %, several: 4.0 %) |
| definition time, p50 / p95 | | | 16–18 µs / 66–70 µs | 16–17 µs / 89–95 µs |
| references time, p50 / p95 / max | | | | 59–61 ms / 207–225 ms / 427–489 ms |
| references found by name (`Probable`) | | | | 50.1 % |
| highlight time, p50 / p95 / max | | | | 74–77 µs / 1.44–1.54 ms / 18.7–19.3 ms |

Indexing peaks at 54–57 MB and the definition and highlight sampling adds about 1 MB, as in stage 2; the rise in stage 3 comes from the usage-search sampling: about 4–5 MB for the copies of the index (the file summaries are shared, the maps are copied) and the rest for the files parsed and searched on 4 threads.

`definition` p95 is higher than in stage 2 because a named-argument label now goes to its parameter, which parses the callee's file: labels are about 5 % of the samples (median 140–170 µs) and two thirds of the slowest 5 %; without them p95 is 59–61 µs. Timing a highlight between the samples makes no measurable difference. The slowest highlights are on such labels too (one parse of the callee's file), on names with hundreds of occurrences in one file and in `override` families of several hundred members.

Most lookups without a target are library and generated names (`String`, `Serializable`, `Modifier`, DI DSL functions such as `single`, generated database and resource classes) and `null`/`true`/`false`, which the grammar reports as identifiers.

## Tests

```bash
cargo test
(cd zed-extension && cargo test)                  # the Zed extension
python3 -m unittest discover -s legacy/tests -v   # Python reference
```

Two legacy tests fail on macOS because the prototype resolves `/var` to `/private/var` while the tests compare unresolved temp paths; that behaviour is left as it was.

## Releasing

1. Bump `version` in `Cargo.toml`, run `cargo test` (it updates `Cargo.lock`) and commit both files.
2. `git tag vX.Y.Z && git push origin vX.Y.Z`. The `release` workflow (`.github/workflows/release.yml`) runs the tests, builds `aarch64-apple-darwin` and `x86_64-apple-darwin` for macOS 11 and later, and publishes `kotlin-navigator-<target>.tar.gz` with a `.sha256` for each. Users get it on their next Zed start.

A tag with a suffix (`vX.Y.Z-rc.1`) becomes a pre-release, which the extension skips: download it by hand to check a build before everyone gets it. `scripts/package-release.sh <tag>` runs the same build and checks locally and leaves the archives in `dist/`.

## Rolling back to Python

`install.sh` leaves `~/.local/share/kotlin-navigator/navigator.py` in place. Uninstall the Kotlin Navigator extension (or put `!kotlin-navigator` in place of `kotlin-navigator` in `language_servers`), drop `!kotlin-language-server` and point `lsp.kotlin-language-server.binary.path` back to `python3` with `"arguments": ["-u", "/Users/YOU/.local/share/kotlin-navigator/navigator.py"]`; `legacy/install.sh` reinstalls it.

## License

MIT, see `LICENSE`.
