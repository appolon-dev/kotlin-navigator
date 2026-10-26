//! The files one request reads: the editor's buffers as of its start, else files on disk.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::docs::OpenDoc;
use crate::scan;

#[derive(Default)]
pub struct Sources {
    open: HashMap<PathBuf, Arc<OpenDoc>>,
    /// Files parsed from disk, kept until the request ends.
    parsed: Mutex<HashMap<PathBuf, Arc<OpenDoc>>>,
}

impl Sources {
    pub fn new(open: Vec<Arc<OpenDoc>>) -> Sources {
        let open = open.into_iter().map(|doc| (doc.summary.path.clone(), doc)).collect();
        Sources { open, parsed: Mutex::default() }
    }

    /// The open buffer of `path`, else the file on disk, parsed once per request.
    pub fn load(&self, path: &Path) -> Option<Arc<OpenDoc>> {
        if let Some(doc) = self.known(path) {
            return Some(doc);
        }
        // Parsed outside the lock; if another thread got there first, its copy is kept.
        let doc = Arc::new(OpenDoc::parse(path.to_path_buf(), scan::read_source(path)?, None)?);
        Some(Arc::clone(self.parsed().entry(path.to_path_buf()).or_insert(doc)))
    }

    /// `load`, but `None` without parsing when the text lacks `needle`. A file read from
    /// disk for this is not kept: a usage search goes through thousands of files.
    pub fn load_containing(&self, path: &Path, needle: &str) -> Option<Arc<OpenDoc>> {
        if let Some(doc) = self.known(path) {
            return doc.text.contains(needle).then_some(doc);
        }
        let text = scan::read_source(path)?;
        if !text.contains(needle) {
            return None;
        }
        OpenDoc::parse(path.to_path_buf(), text, None).map(Arc::new)
    }

    fn known(&self, path: &Path) -> Option<Arc<OpenDoc>> {
        self.open.get(path).cloned().or_else(|| self.parsed().get(path).cloned())
    }

    fn parsed(&self) -> MutexGuard<'_, HashMap<PathBuf, Arc<OpenDoc>>> {
        self.parsed.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
