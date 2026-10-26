//! Declaration index. Updating a file replaces only that file's entries.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::model::{Decl, FileSummary};

#[derive(Clone, Default)]
pub struct Index {
    files: HashMap<PathBuf, Arc<FileSummary>>,
    /// Name → files declaring it, each file once per distinct name.
    by_name: HashMap<String, Vec<Arc<FileSummary>>>,
    decl_count: usize,
}

impl Index {
    pub fn upsert(&mut self, summary: FileSummary) {
        self.remove(&summary.path);
        let file = Arc::new(summary);
        for name in distinct_names(&file) {
            self.by_name.entry(name.to_string()).or_default().push(Arc::clone(&file));
        }
        self.decl_count += file.decls.len();
        self.files.insert(file.path.clone(), file);
    }

    pub fn remove(&mut self, path: &Path) {
        let Some(old) = self.files.remove(path) else { return };
        self.decl_count -= old.decls.len();
        for name in distinct_names(&old) {
            if let Some(files) = self.by_name.get_mut(name) {
                files.retain(|f| !Arc::ptr_eq(f, &old));
                if files.is_empty() {
                    self.by_name.remove(name);
                }
            }
        }
    }

    pub fn file(&self, path: &Path) -> Option<&FileSummary> {
        self.files.get(path).map(Arc::as_ref)
    }

    /// Declarations named `name`, sorted by (path, line).
    pub fn lookup(&self, name: &str) -> Vec<(&FileSummary, &Decl)> {
        let mut found: Vec<(&FileSummary, &Decl)> = self
            .by_name
            .get(name)
            .into_iter()
            .flatten()
            .flat_map(|f| f.decls.iter().filter(|d| d.name == name).map(move |d| (f.as_ref(), d)))
            .collect();
        found.sort_by(|(fa, da), (fb, db)| (&fa.path, da.name_range.line).cmp(&(&fb.path, db.name_range.line)));
        found
    }

    pub fn files(&self) -> impl Iterator<Item = &FileSummary> {
        self.files.values().map(Arc::as_ref)
    }

    pub fn decls(&self) -> impl Iterator<Item = (&FileSummary, &Decl)> {
        self.files.values().flat_map(|f| f.decls.iter().map(move |d| (f.as_ref(), d)))
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    pub fn decl_count(&self) -> usize {
        self.decl_count
    }
}

fn distinct_names(file: &FileSummary) -> HashSet<&str> {
    file.decls.iter().map(|d| d.name.as_str()).collect()
}
