//! Library sources for Go to Definition on what the workspace lacks: `*-sources.jar` from
//! the Gradle cache and the Android SDK's `sources/android-N`. A jar entry is extracted to
//! the cache directory when it is jumped to: Zed opens only plain files.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use rayon::prelude::*;
use zip::ZipArchive;

use crate::model::{FileSummary, Lang};
use crate::parse::summarize_text;
use crate::resolve::context::{Receiver, Role, reference_at};
use crate::resolve::{Current, Target, import_key};

/// Bytes read from the start of a jar entry to find its `package` line, past a license header.
const HEADER_BYTES: u64 = 16 * 1024;

/// Packages every Kotlin file imports.
const KOTLIN_DEFAULT_IMPORTS: [&str; 10] = [
    "kotlin",
    "kotlin.annotation",
    "kotlin.collections",
    "kotlin.comparisons",
    "kotlin.io",
    "kotlin.ranges",
    "kotlin.sequences",
    "kotlin.text",
    "kotlin.jvm",
    "java.lang",
];

#[derive(Default)]
pub struct Library {
    jars: Vec<PathBuf>,
    /// Package → the jar entries declaring it: (index in `jars`, entry name).
    packages: HashMap<String, Vec<(u32, String)>>,
    /// The Android SDK's `sources/android-N` with the highest N.
    android: Option<PathBuf>,
    /// Where jar entries are extracted.
    cache: PathBuf,
    /// FQN → where it was found, misses included.
    found: Mutex<HashMap<String, Option<Target>>>,
}

impl Library {
    /// The Gradle cache, the Android SDK and the cache directory at their usual places;
    /// the version catalogs of the workspace `roots` tell which libraries the project uses.
    pub fn scan(roots: &[PathBuf]) -> Library {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let gradle = std::env::var_os("GRADLE_USER_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".gradle"));
        let sdk = ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
            .into_iter()
            .find_map(std::env::var_os)
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("Library/Android/sdk"));
        let cache = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(if cfg!(target_os = "macos") { "Library/Caches" } else { ".cache" }));
        let catalogs: String = roots
            .iter()
            .flat_map(|root| fs::read_dir(root.join("gradle")).into_iter().flatten().flatten())
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|e| e == "toml"))
            .filter_map(|path| fs::read_to_string(path).ok())
            .collect();
        let files = gradle.join("caches/modules-2/files-2.1");
        Library::scan_in(&files, &sdk.join("sources"), cache.join("kotlin-navigator/sources"), &catalogs)
    }

    /// `files` is laid out as Gradle's `modules-2/files-2.1`, `android_sources` as the SDK's
    /// `sources`; `catalogs` is the text of the project's version catalogs.
    pub fn scan_in(files: &Path, android_sources: &Path, cache: PathBuf, catalogs: &str) -> Library {
        let jars = sources_jars(files, catalogs);
        let listed: Vec<Vec<(String, String)>> = jars.par_iter().map(|jar| packages_of(jar).unwrap_or_default()).collect();
        let mut packages: HashMap<String, Vec<(u32, String)>> = HashMap::new();
        for (i, entries) in listed.into_iter().enumerate() {
            for (package, entry) in entries {
                packages.entry(package).or_default().push((i as u32, entry));
            }
        }
        Library { jars, packages, android: newest_android(android_sources), cache, found: Mutex::default() }
    }

    pub fn jar_count(&self) -> usize {
        self.jars.len()
    }

    pub fn package_count(&self) -> usize {
        self.packages.len()
    }

    /// The declaration `fqn` in the library sources.
    pub fn find(&self, fqn: &str) -> Option<Target> {
        if let Some(found) = self.found().get(fqn) {
            return found.clone();
        }
        // Searched outside the lock; two requests for one name may both search.
        let found = self.search(fqn);
        self.found().insert(fqn.to_string(), found.clone());
        found
    }

    /// The package is the longest prefix of `fqn` that has sources; the segment after it
    /// names the top-level declaration.
    fn search(&self, fqn: &str) -> Option<Target> {
        let segments: Vec<&str> = fqn.split('.').collect();
        (1..segments.len()).rev().find_map(|split| {
            let package = segments[..split].join(".");
            let top = segments[split];
            self.in_android(&package, top, fqn).or_else(|| self.in_jars(&package, top, fqn))
        })
    }

    fn in_android(&self, package: &str, top: &str, fqn: &str) -> Option<Target> {
        let path = self.android.as_ref()?.join(package.replace('.', "/")).join(format!("{top}.java"));
        let text = fs::read_to_string(&path).ok()?;
        target_in(path, &text, fqn)
    }

    /// The file named after `top` first, then those that mention it: a type alias, a file
    /// of several classes, an `expect` in `commonMain` with its `actual` elsewhere.
    fn in_jars(&self, package: &str, top: &str, fqn: &str) -> Option<Target> {
        let entries = self.packages.get(package)?;
        let named = |entry: &str| Path::new(entry).file_stem().is_some_and(|s| s == top);
        let ordered = entries.iter().filter(|(_, e)| named(e)).chain(entries.iter().filter(|(_, e)| !named(e)));
        for (jar, entry) in ordered {
            let jar = &self.jars[*jar as usize];
            let Some(text) = read_entry(jar, entry).filter(|text| declares(text, top)) else { continue };
            let path = self.cache.join(cache_dir(jar)).join(entry);
            if let Some(target) = target_in(path.clone(), &text, fqn) {
                return extract(&path, &text).then_some(target);
            }
        }
        None
    }

    /// The folder of the library `path` was extracted from, if it was.
    pub fn folder_of(&self, path: &Path) -> Option<PathBuf> {
        self.source_of(path).map(|(_, dir)| dir)
    }

    /// Extracts every source file of the library `path` was extracted from, next to it,
    /// and gives the library's folder.
    pub fn extract_all(&self, path: &Path) -> Option<PathBuf> {
        let (jar, dir) = self.source_of(path)?;
        let mut archive = ZipArchive::new(File::open(jar).ok()?).ok()?;
        for i in 0..archive.len() {
            let Ok(mut file) = archive.by_index(i) else { continue };
            let Some(rel) = file.enclosed_name().filter(|_| is_source(file.name())) else { continue };
            let target = dir.join(rel);
            let mut text = String::new();
            if !target.is_file() && file.read_to_string(&mut text).is_ok() {
                extract(&target, &text);
            }
        }
        Some(dir)
    }

    /// The jar `path` was extracted from and the folder it was extracted to.
    fn source_of(&self, path: &Path) -> Option<(&Path, PathBuf)> {
        let rel = path.strip_prefix(&self.cache).ok()?;
        self.jars.iter().find_map(|jar| {
            let dir = cache_dir(jar);
            rel.starts_with(&dir).then(|| (jar.as_path(), self.cache.join(dir)))
        })
    }

    fn found(&self) -> MutexGuard<'_, HashMap<String, Option<Target>>> {
        self.found.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// What the reference at `byte` may name outside the workspace, most likely first: an
/// import line; a simple name through imports; a name after a path (`Interceptor.Chain`)
/// as a member of what the path's first segment names, else the path as a package.
pub fn candidates(current: &Current, byte: usize) -> Vec<String> {
    let Some(reference) = reference_at(current.tree, current.text, byte) else { return Vec::new() };
    let name = &reference.name;
    match (&reference.role, &reference.receiver) {
        (Role::Import(fqn), _) => vec![fqn.clone()],
        (Role::Argument, _) => Vec::new(),
        (_, Receiver::None) => imported(current.summary, name),
        (_, Receiver::Path(path)) => {
            let (head, rest) = path.split_once('.').map_or((path.as_str(), ""), |(h, r)| (h, r));
            let rest = if rest.is_empty() { String::new() } else { format!(".{rest}") };
            imported(current.summary, head)
                .into_iter()
                .map(|outer| format!("{outer}{rest}.{name}"))
                .chain([format!("{path}.{name}")])
                .collect()
        }
        _ => Vec::new(),
    }
}

/// The simple `name` through the explicit import that names it, else through each
/// wildcard and default import.
fn imported(summary: &FileSummary, name: &str) -> Vec<String> {
    if let Some(import) = summary.imports.iter().find(|i| !i.wildcard && import_key(i) == name) {
        return vec![import.path.clone()];
    }
    let defaults: &[&str] = match summary.lang {
        Lang::Kotlin => &KOTLIN_DEFAULT_IMPORTS,
        Lang::Java => &["java.lang"],
    };
    summary
        .imports
        .iter()
        .filter(|i| i.wildcard)
        .map(|i| i.path.as_str())
        .chain(defaults.iter().copied())
        .map(|package| format!("{package}.{name}"))
        .collect()
}

/// `<group>/<artifact>/<version>/<hash>/<artifact>-<version>-sources.jar`, one version per
/// artifact, the artifacts in `catalogs` first: their jars win when artifacts share a
/// package (`okhttp` and `okhttp-jvm`). A version written in `catalogs` wins over the others.
// ponytail: otherwise the latest download stands for the version in use; read the build
// scripts (inline `group:artifact:version`) if that guesses wrong too often.
fn sources_jars(files: &Path, catalogs: &str) -> Vec<PathBuf> {
    let mut jars = Vec::new();
    for group_dir in dirs(files) {
        let group = group_dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
        for artifact_dir in dirs(&group_dir) {
            let artifact = artifact_dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let module = format!("\"{group}:{artifact}");
            let listed = catalogs.contains(&format!("{module}\"")) || catalogs.contains(&format!("{module}:"));
            let chosen = dirs(&artifact_dir)
                .flat_map(|version_dir| {
                    let version = version_dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
                    let pinned = listed && catalogs.contains(&format!("\"{version}\""));
                    let name = format!("{artifact}-{version}-sources.jar");
                    dirs(&version_dir).map(move |hash_dir| (pinned, hash_dir.join(&name)))
                })
                .filter_map(|(pinned, jar)| Some((pinned, fs::metadata(&jar).ok()?.modified().ok()?, jar)))
                .max();
            jars.extend(chosen.map(|(_, modified, jar)| (listed, modified, jar)));
        }
    }
    jars.sort_by(|a, b| b.cmp(a));
    jars.into_iter().map(|(_, _, jar)| jar).collect()
}

/// `<group>/<artifact>-<version>-sources` of a jar: artifacts of different groups may share
/// a name (`androidx.core:core` and `com.github.xgouchet.Elmyr:core`).
fn cache_dir(jar: &Path) -> PathBuf {
    let group = jar.ancestors().nth(4).and_then(Path::file_name).unwrap_or_default();
    Path::new(group).join(jar.file_stem().unwrap_or_default())
}

fn dirs(path: &Path) -> impl Iterator<Item = PathBuf> + use<> {
    fs::read_dir(path).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir())
}

/// (package, entry name) of each Kotlin and Java file in `jar`, read from its `package` line.
fn packages_of(jar: &Path) -> Option<Vec<(String, String)>> {
    let mut archive = ZipArchive::new(File::open(jar).ok()?).ok()?;
    let mut found = Vec::new();
    for i in 0..archive.len() {
        let Ok(file) = archive.by_index(i) else { continue };
        let name = file.name().to_string();
        // An entry that would land outside its folder (`../x.kt`) is never extracted.
        if !is_source(&name) || file.enclosed_name().is_none() {
            continue;
        }
        let mut head = Vec::new();
        if file.take(HEADER_BYTES).read_to_end(&mut head).is_ok() {
            found.push((package_of(&String::from_utf8_lossy(&head)), name));
        }
    }
    Some(found)
}

/// Whether a line of `text` looks like a declaration of `name`: a keyword before it with no
/// `(`, `)`, `:` or `=` in between. `fun <T> listOf(` passes, `fun f(): List<T>` does not.
/// Cheaper than parsing every file of a package that mentions the name.
fn declares(text: &str, name: &str) -> bool {
    const KEYWORDS: [&str; 7] = ["class ", "interface ", "object ", "typealias ", "fun ", "val ", "var "];
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    text.lines().any(|line| {
        line.match_indices(name).any(|(at, _)| {
            let before = &line[..at];
            let bounded = !before.ends_with(ident) && !line[at + name.len()..].starts_with(ident);
            bounded
                && KEYWORDS
                    .iter()
                    .filter_map(|k| before.rfind(k).map(|i| &before[i + k.len()..]))
                    .any(|between| !between.contains(['(', ')', ':', '=']))
        })
    })
}

fn is_source(entry: &str) -> bool {
    entry.ends_with(".kt") || entry.ends_with(".java")
}

fn package_of(head: &str) -> String {
    head.lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("package "))
        .map(|p| p.trim_end_matches(';').trim().replace('`', ""))
        .unwrap_or_default()
}

fn newest_android(sources: &Path) -> Option<PathBuf> {
    let level = |dir: &PathBuf| dir.file_name()?.to_str()?.strip_prefix("android-")?.parse::<u32>().ok();
    dirs(sources).filter_map(|dir| Some((level(&dir)?, dir))).max().map(|(_, dir)| dir)
}

fn read_entry(jar: &Path, entry: &str) -> Option<String> {
    let mut archive = ZipArchive::new(File::open(jar).ok()?).ok()?;
    let mut text = String::new();
    archive.by_name(entry).ok()?.read_to_string(&mut text).ok()?;
    Some(text)
}

/// The declaration `fqn` in `text`, as if it were the file at `path`.
fn target_in(path: PathBuf, text: &str, fqn: &str) -> Option<Target> {
    let (_, summary) = summarize_text(path, text)?;
    let decl = summary.decls.iter().find(|d| d.fqn == fqn)?;
    Some(Target { path: summary.path.clone(), range: decl.name_range })
}

/// Writes `text` to `path` once, read-only: it is a library's source, not to be edited.
fn extract(path: &Path, text: &str) -> bool {
    if path.is_file() {
        return true;
    }
    let written = path.parent().map_or(Ok(()), fs::create_dir_all).and_then(|()| {
        fs::write(path, text)?;
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_readonly(true);
        fs::set_permissions(path, permissions)
    });
    if let Err(e) = &written {
        log::warn!("cannot extract {}: {e}", path.display());
    }
    written.is_ok()
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::time::{Duration, SystemTime};

    use tempfile::TempDir;
    use zip::write::{SimpleFileOptions, ZipWriter};

    use super::*;
    use crate::docs::OpenDoc;
    use crate::index::Index;

    /// A sources jar of `group:artifact:version` in a Gradle cache under `files`.
    fn jar(files: &Path, group: &str, artifact: &str, version: &str, entries: &[(&str, &str)]) -> PathBuf {
        let dir = files.join(group).join(artifact).join(version).join("0123abcd");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{artifact}-{version}-sources.jar"));
        let mut zip = ZipWriter::new(File::create(&path).unwrap());
        for (name, text) in entries {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    fn library(dir: &TempDir) -> Library {
        Library::scan_in(&dir.path().join("files"), &dir.path().join("sdk/sources"), dir.path().join("cache"), "")
    }

    #[test]
    fn finds_a_class_whose_path_does_not_follow_its_package_and_extracts_it() {
        let dir = TempDir::new().unwrap();
        let files = dir.path().join("files");
        let flow = "/*\n * License\n */\npackage kotlinx.coroutines.flow\n\npublic interface Flow<out T>\n";
        jar(&files, "org.jetbrains.kotlinx", "kotlinx-coroutines-core-jvm", "1.8.1", &[("commonMain/flow/Flow.kt", flow)]);
        let target = library(&dir).find("kotlinx.coroutines.flow.Flow").unwrap();
        assert_eq!(target.path, dir.path().join("cache/org.jetbrains.kotlinx/kotlinx-coroutines-core-jvm-1.8.1-sources/commonMain/flow/Flow.kt"));
        assert_eq!((target.range.line, target.range.start), (5, 17));
        assert_eq!(fs::read_to_string(&target.path).unwrap(), flow);
        assert!(fs::metadata(&target.path).unwrap().permissions().readonly());
    }

    #[test]
    fn finds_a_declaration_in_a_file_named_otherwise_and_a_nested_class() {
        let dir = TempDir::new().unwrap();
        let text = "package kotlin.collections\n\npublic interface List<out E>\npublic interface Map<K, V> {\n    interface Entry<K, V>\n}\npublic fun <T> listOf(): List<T> = TODO()\n";
        jar(&dir.path().join("files"), "org.jetbrains.kotlin", "kotlin-stdlib", "2.1.21", &[("commonMain/kotlin/Collections.kt", text)]);
        let library = library(&dir);
        assert_eq!(library.find("kotlin.collections.List").unwrap().range.line, 2);
        assert_eq!(library.find("kotlin.collections.Map.Entry").unwrap().range.line, 4);
        assert_eq!(library.find("kotlin.collections.listOf").unwrap().range.line, 6);
        assert_eq!(library.find("kotlin.collections.Set"), None);
    }

    #[test]
    fn takes_the_latest_downloaded_version_of_an_artifact() {
        let dir = TempDir::new().unwrap();
        let files = dir.path().join("files");
        let old = jar(&files, "com.squareup.okhttp3", "okhttp", "4.9.3", &[("okhttp3/OkHttpClient.kt", "package okhttp3\n\nclass OkHttpClient\n")]);
        jar(&files, "com.squareup.okhttp3", "okhttp", "4.12.0", &[("okhttp3/OkHttpClient.kt", "package okhttp3\n\n\nclass OkHttpClient\n")]);
        File::options().write(true).open(&old).unwrap().set_modified(SystemTime::now() - Duration::from_secs(3600)).unwrap();
        let library = library(&dir);
        assert_eq!(library.jar_count(), 1);
        let target = library.find("okhttp3.OkHttpClient").unwrap();
        assert!(target.path.starts_with(dir.path().join("cache/com.squareup.okhttp3/okhttp-4.12.0-sources")), "{}", target.path.display());
        assert_eq!(target.range.line, 3);
    }

    #[test]
    fn prefers_artifacts_and_versions_of_the_version_catalog() {
        let dir = TempDir::new().unwrap();
        let files = dir.path().join("files");
        let client = "package okhttp3\n\nclass OkHttpClient\n";
        let pinned = jar(&files, "com.squareup.okhttp3", "okhttp", "4.12.0", &[("okhttp3/OkHttpClient.kt", client)]);
        jar(&files, "com.squareup.okhttp3", "okhttp", "4.9.3", &[("okhttp3/OkHttpClient.kt", client)]);
        jar(&files, "com.squareup.okhttp3", "okhttp-jvm", "5.5.0", &[("okhttp3/OkHttpClient.kt", client)]);
        File::options().write(true).open(&pinned).unwrap().set_modified(SystemTime::now() - Duration::from_secs(3600)).unwrap();
        let catalogs = "[versions]\nokHttp = \"4.12.0\"\n[libraries]\nokHttp = { module = \"com.squareup.okhttp3:okhttp\", version.ref = \"okHttp\" }\n";
        let library = Library::scan_in(&files, &dir.path().join("sdk"), dir.path().join("cache"), catalogs);
        assert_eq!(library.jar_count(), 2);
        let target = library.find("okhttp3.OkHttpClient").unwrap();
        assert!(target.path.starts_with(dir.path().join("cache/com.squareup.okhttp3/okhttp-4.12.0-sources")), "{}", target.path.display());
    }

    #[test]
    fn extract_all_extracts_the_library_of_a_jumped_to_file_once() {
        let dir = TempDir::new().unwrap();
        let entries = [
            ("okhttp3/Interceptor.kt", "package okhttp3\n\nfun interface Interceptor {\n  interface Chain\n}\n"),
            ("okhttp3/Call.kt", "package okhttp3\n\ninterface Call\n"),
            ("META-INF/MANIFEST.MF", "Manifest-Version: 1.0\n"),
            ("../escape.kt", "package okhttp3\n\nclass Escape\n"),
        ];
        jar(&dir.path().join("files"), "com.squareup.okhttp3", "okhttp", "4.12.0", &entries);
        let library = library(&dir);
        let chain = library.find("okhttp3.Interceptor.Chain").unwrap();
        assert_eq!(chain.range.line, 3);
        let folder = dir.path().join("cache/com.squareup.okhttp3/okhttp-4.12.0-sources");
        assert_eq!(library.folder_of(&chain.path), Some(folder.clone()));
        assert_eq!(library.folder_of(&dir.path().join("cache/com.squareup.okhttp3/okhttp-4.9.3-sources/okhttp3/Call.kt")), None);
        assert_eq!(library.extract_all(&chain.path), Some(folder.clone()));
        assert!(folder.join("okhttp3/Call.kt").is_file());
        assert!(!folder.join("META-INF/MANIFEST.MF").exists());
        assert!(!dir.path().join("cache/com.squareup.okhttp3/escape.kt").exists());
        assert_eq!(library.extract_all(&chain.path), Some(folder));
    }

    #[test]
    fn declares_tells_declarations_from_uses() {
        assert!(declares("public interface List<out E> : Collection<E>", "List"));
        assert!(declares("public fun <T> listOf(vararg elements: T): List<T>", "listOf"));
        assert!(declares("public actual typealias ArrayList<E> = java.util.ArrayList<E>", "ArrayList"));
        assert!(!declares("public fun <T> emptyList(): List<T> = EmptyList", "List"));
        assert!(!declares("class MutableList<E>", "List"));
        assert!(!declares("val list = mutableListOf<Int>()", "List"));
    }

    #[test]
    fn finds_android_sources_of_the_highest_api_level_in_place() {
        let dir = TempDir::new().unwrap();
        for (level, text) in [("34", "package android.app;\npublic class Activity {}\n"), ("36", "package android.app;\n\npublic class Activity {}\n")] {
            let package = dir.path().join(format!("sdk/sources/android-{level}/android/app"));
            fs::create_dir_all(&package).unwrap();
            fs::write(package.join("Activity.java"), text).unwrap();
        }
        let target = library(&dir).find("android.app.Activity").unwrap();
        assert_eq!(target.path, dir.path().join("sdk/sources/android-36/android/app/Activity.java"));
        assert_eq!(target.range.line, 2);
    }

    #[test]
    fn candidates_follow_import_lines_explicit_wildcard_and_default_imports() {
        let text = "package app\n\nimport okhttp3.OkHttpClient as Client\nimport kotlinx.coroutines.flow.*\n\nval c: Client? = null\nval f: Flow<Int>? = null\n";
        let doc = OpenDoc::parse(PathBuf::from("/ws/app/Use.kt"), text.to_string(), None).unwrap();
        let index = Index::default();
        let current = Current::of(&index, &doc);
        let at = |needle: &str| text.find(needle).unwrap() + 1;
        assert_eq!(candidates(&current, at("OkHttpClient")), ["okhttp3.OkHttpClient"]);
        assert_eq!(candidates(&current, at("Client?")), ["okhttp3.OkHttpClient"]);
        let flow = candidates(&current, at("Flow<"));
        assert_eq!(flow[..3], ["kotlinx.coroutines.flow.Flow", "kotlin.Flow", "kotlin.annotation.Flow"]);
        assert_eq!(flow.len(), 1 + KOTLIN_DEFAULT_IMPORTS.len());
    }

    #[test]
    fn candidates_of_a_nested_name_go_through_the_outer_name() {
        let text = "package app\n\nimport okhttp3.Interceptor\n\nfun f(chain: Interceptor.Chain, e: okhttp3.Interceptor.Chain) {}\n";
        let doc = OpenDoc::parse(PathBuf::from("/ws/app/Use.kt"), text.to_string(), None).unwrap();
        let index = Index::default();
        let current = Current::of(&index, &doc);
        let first = text.find("Chain,").unwrap() + 1;
        assert_eq!(candidates(&current, first), ["okhttp3.Interceptor.Chain", "Interceptor.Chain"]);
        let qualified = text.rfind("Chain").unwrap() + 1;
        assert_eq!(candidates(&current, qualified).last().unwrap(), "okhttp3.Interceptor.Chain");
    }
}
