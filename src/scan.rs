//! Workspace walk (gitignore-aware) and parallel initial indexing.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
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
    }
    found.into_iter().collect()
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
