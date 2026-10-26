//! Workspace walk (gitignore-aware) and parallel initial indexing.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use ignore::WalkBuilder;
use rayon::prelude::*;

use crate::index::Index;
use crate::parse::summarize_text;

/// Directories skipped even when no ignore file mentions them (same list as the Python version).
pub const SKIP_DIRS: [&str; 6] = ["build", "out", "target", "bin", "node_modules", "generated"];
pub const MAX_FILE_SIZE: u64 = 2 * 1024 * 1024;

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

/// `.kt`/`.java` files tracked by git under `root`; empty when git or the repository is unavailable.
fn git_tracked_sources(root: &Path) -> Vec<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z", "--", "*.kt", "*.java"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    let Ok(output) = output else { return Vec::new() };
    if !output.status.success() {
        return Vec::new();
    }
    output
        .stdout
        .split(|b| *b == 0)
        .filter_map(|rel| std::str::from_utf8(rel).ok())
        .filter(|rel| !rel.is_empty())
        .map(|rel| root.join(rel))
        .collect()
}

pub fn read_source(path: &Path) -> Option<String> {
    String::from_utf8(std::fs::read(path).ok()?).ok()
}

pub fn build_index(roots: &[PathBuf]) -> (Index, ScanStats) {
    let started = Instant::now();
    let paths = walk_sources(roots);
    let summaries: Vec<_> = paths
        .par_iter()
        .map(|path| summarize_text(path.clone(), &read_source(path)?).map(|(_, summary)| summary))
        .collect();
    let skipped = summaries.iter().filter(|s| s.is_none()).count();
    let mut index = Index::default();
    let mut error_files = 0;
    for summary in summaries.into_iter().flatten() {
        error_files += usize::from(summary.has_errors);
        index.upsert(summary);
    }
    let stats = ScanStats {
        files: index.file_count(),
        skipped,
        decls: index.decl_count(),
        error_files,
        elapsed: started.elapsed(),
    };
    (index, stats)
}
