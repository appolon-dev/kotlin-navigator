//! Workspace walk (gitignore-aware) and parallel initial indexing.

use std::collections::{BTreeSet, HashMap};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use ignore::WalkBuilder;
use rayon::prelude::*;

use crate::index::Index;
use crate::model::FileSummary;
use crate::parse::{source_text, summarize_text};

/// Directories skipped even when no ignore file mentions them (same list as the Python version).
pub const SKIP_DIRS: [&str; 6] = ["build", "out", "target", "bin", "node_modules", "generated"];
pub const MAX_FILE_SIZE: u64 = 2 * 1024 * 1024;
/// Files parsed and published to the live index at a time during the initial scan.
pub const CHUNK: usize = 512;
/// `git ls-files` on a huge or locked repository must not hold up indexing.
pub const GIT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug)]
pub struct ScanStats {
    pub files: usize,
    pub skipped: usize,
    pub decls: usize,
    pub error_files: usize,
    pub elapsed: Duration,
}

pub fn is_indexed(path: &Path) -> bool {
    matches!(path.extension().and_then(|e| e.to_str()), Some("kt" | "java"))
}

pub fn walk_sources(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = BTreeSet::new();
    for root in roots {
        let walker = WalkBuilder::new(root)
            .hidden(true)
            .git_ignore(true)
            .git_global(true)
            .ignore(true)
            .require_git(false)
            .filter_entry(|entry| {
                let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
                entry.depth() == 0 || !is_dir || !SKIP_DIRS.iter().any(|d| entry.file_name() == *d)
            })
            .build();
        for entry in walker.flatten() {
            let path = entry.path();
            if !entry.file_type().is_some_and(|t| t.is_file()) || !is_indexed(path) {
                continue;
            }
            if entry.metadata().is_ok_and(|m| m.len() <= MAX_FILE_SIZE) {
                found.insert(path.to_path_buf());
            }
        }
        // Ignore rules never apply to files git already tracks (e.g. `.git/info/exclude: keys`
        // hitting a tracked `navigation/keys/` package), so add those back.
        for path in git_tracked_sources(root) {
            let in_skipped_dir = path.strip_prefix(root).is_ok_and(|rel| {
                rel.parent().into_iter().flat_map(Path::components).any(|c| matches!(c, Component::Normal(n) if SKIP_DIRS.iter().any(|d| n == *d)))
            });
            if !in_skipped_dir && std::fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() <= MAX_FILE_SIZE) {
                found.insert(path);
            }
        }
    }
    found.into_iter().collect()
}

/// Whether the walk reaches `path` by its location: under a root, with no hidden or
/// skipped directory in between. Ignore files are not consulted.
pub fn in_workspace(roots: &[PathBuf], path: &Path) -> bool {
    roots.iter().any(|root| {
        path.strip_prefix(root).is_ok_and(|rel| {
            !rel.parent().into_iter().flat_map(Path::components).any(|c| match c {
                Component::Normal(n) => n.as_encoded_bytes().starts_with(b".") || SKIP_DIRS.iter().any(|d| n == *d),
                _ => true,
            })
        })
    })
}

/// The Gradle module of a file: the nearest directory up from it, up to and including
/// its workspace root, that has `build.gradle.kts` or `build.gradle`.
pub fn module_of(path: &Path, roots: &[PathBuf]) -> Option<PathBuf> {
    let root = roots.iter().find(|root| path.starts_with(root))?;
    path.ancestors().skip(1).take_while(|dir| dir.starts_with(root)).find_map(|dir| {
        let has_script = ["build.gradle.kts", "build.gradle"].iter().any(|f| dir.join(f).is_file());
        has_script.then(|| dir.to_path_buf())
    })
}

/// `.kt`/`.java` files tracked by git under `root`; empty when git or the repository is
/// unavailable, or git takes longer than `GIT_TIMEOUT`.
fn git_tracked_sources(root: &Path) -> Vec<PathBuf> {
    let mut command = Command::new("git");
    command.arg("-C").arg(root).args(["ls-files", "-z", "--", "*.kt", "*.java"]);
    let Some(stdout) = output_within(&mut command, GIT_TIMEOUT) else { return Vec::new() };
    stdout
        .split(|b| *b == 0)
        .filter_map(|rel| std::str::from_utf8(rel).ok())
        .filter(|rel| !rel.is_empty())
        .map(|rel| root.join(rel))
        .collect()
}

/// The command's stdout if it exits successfully within `timeout`; otherwise it is killed and `None`.
pub fn output_within(command: &mut Command, timeout: Duration) -> Option<Vec<u8>> {
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    // A full pipe would block the child, so it is drained while we wait.
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        stdout.read_to_end(&mut out).map(|_| out)
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < timeout => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let out = reader.join().ok()?.ok()?;
    status.success().then_some(out)
}

/// A source file as the server keeps it (see `parse::source_text`).
pub fn read_source(path: &Path) -> Option<String> {
    String::from_utf8(std::fs::read(path).ok()?).ok().map(source_text)
}

pub fn build_index(roots: &[PathBuf]) -> (Index, ScanStats) {
    let mut index = Index::default();
    let stats = build_index_with(roots, &mut |chunk| chunk.into_iter().for_each(|summary| index.upsert(summary)));
    (index, stats)
}

/// Parses the workspace in chunks of `CHUNK` files, handing each chunk to `publish` as it is ready.
pub fn build_index_with(roots: &[PathBuf], publish: &mut dyn FnMut(Vec<FileSummary>)) -> ScanStats {
    let started = Instant::now();
    let paths = walk_sources(roots);
    let mut stats = ScanStats { files: 0, skipped: 0, decls: 0, error_files: 0, elapsed: Duration::ZERO };
    // Files of one directory share a module: look it up once per directory.
    let mut modules: HashMap<PathBuf, Option<PathBuf>> = HashMap::new();
    for chunk in paths.chunks(CHUNK) {
        let summaries: Vec<_> = chunk
            .par_iter()
            .map(|path| summarize_text(path.clone(), &read_source(path)?).map(|(_, summary)| summary))
            .collect();
        stats.skipped += summaries.iter().filter(|s| s.is_none()).count();
        let mut ready = Vec::with_capacity(summaries.len());
        for mut summary in summaries.into_iter().flatten() {
            stats.error_files += usize::from(summary.has_errors);
            stats.decls += summary.decls.len();
            let dir = summary.path.parent().map(Path::to_path_buf).unwrap_or_default();
            summary.module = modules.entry(dir).or_insert_with(|| module_of(&summary.path, roots)).clone();
            ready.push(summary);
        }
        stats.files += ready.len();
        publish(ready);
    }
    stats.elapsed = started.elapsed();
    stats
}
