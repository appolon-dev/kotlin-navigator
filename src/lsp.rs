//! LSP server: the main thread applies buffer updates, requests run on a
//! small thread pool, the initial index is built in the background.

use std::any::Any;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::Duration;

use lsp_server::{Connection, ErrorCode, Message, Notification, Request, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidChangeWatchedFiles, DidCloseTextDocument, DidOpenTextDocument, DidSaveTextDocument,
    Notification as _,
};
use lsp_types::request::{GotoDefinition, Request as _, WorkspaceSymbolRequest};
use lsp_types::{
    GotoDefinitionParams, GotoDefinitionResponse, Location, OneOf, Position, Range, ServerCapabilities, SymbolInformation,
    SymbolKind, TextDocumentSyncCapability, TextDocumentSyncKind, WorkspaceSymbolParams, WorkspaceSymbolResponse,
};
use serde_json::{Value, json};

use crate::docs::{self, Documents, path_to_uri, uri_to_path};
use crate::index::Index;
use crate::model::{Decl, DeclKind, FileSummary, Span};
use crate::{parse, resolve, scan};

pub const READY_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_SYMBOLS: usize = 100;
const REQUEST_THREADS: usize = 4;

pub struct State {
    pub index: RwLock<Index>,
    pub docs: RwLock<Documents>,
    ready: Mutex<bool>,
    ready_changed: Condvar,
    ready_timeout: Duration,
}

impl State {
    pub fn new(ready_timeout: Duration) -> Arc<State> {
        Arc::new(State {
            index: RwLock::default(),
            docs: RwLock::default(),
            ready: Mutex::new(false),
            ready_changed: Condvar::new(),
            ready_timeout,
        })
    }

    pub fn mark_ready(&self) {
        *self.ready.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.ready_changed.notify_all();
    }

    /// Waits for the initial index; `false` if the timeout passed first.
    pub fn wait_ready(&self) -> bool {
        let ready = self.ready.lock().unwrap_or_else(PoisonError::into_inner);
        let (ready, _) = self
            .ready_changed
            .wait_timeout_while(ready, self.ready_timeout, |ready| !*ready)
            .unwrap_or_else(PoisonError::into_inner);
        *ready
    }

    pub fn open_or_change(&self, path: PathBuf, text: String) {
        let mut docs = write(&self.docs);
        let Some(doc) = docs.set(path.clone(), text) else { return };
        if scan::is_indexed(&path) {
            write(&self.index).upsert(doc.summary.clone());
        }
    }

    pub fn close(&self, path: &Path) {
        write(&self.docs).close(path);
        self.reload_from_disk(path);
    }

    /// Re-reads a file that is not open in the editor; a missing or unreadable file leaves the index.
    pub fn reload_from_disk(&self, path: &Path) {
        let docs = read(&self.docs);
        if docs.get(path).is_some() || !scan::is_indexed(path) {
            return;
        }
        let summary = scan::read_source(path).and_then(|text| parse::summarize_text(path.to_path_buf(), &text));
        let mut index = write(&self.index);
        match summary {
            Some((_, summary)) => index.upsert(summary),
            None => index.remove(path),
        }
    }

    /// Installs the freshly built index; open buffers win over what was read from disk.
    pub fn finish_indexing(&self, mut index: Index) {
        let docs = read(&self.docs);
        for doc in docs.iter().filter(|d| scan::is_indexed(&d.summary.path)) {
            index.upsert(doc.summary.clone());
        }
        *write(&self.index) = index;
        drop(docs);
        self.mark_ready();
    }
}

fn read<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(PoisonError::into_inner)
}

fn write<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(PoisonError::into_inner)
}

pub fn definition(state: &State, path: &Path, position: Position) -> Vec<Location> {
    state.wait_ready();
    let docs = read(&state.docs);
    let from_disk;
    let (text, tree, summary) = match docs.get(path) {
        Some(doc) => (&doc.text, &doc.tree, &doc.summary),
        None => {
            let Some(text) = scan::read_source(path) else { return Vec::new() };
            let Some((tree, summary)) = parse::summarize_text(path.to_path_buf(), &text) else { return Vec::new() };
            from_disk = (text, tree, summary);
            (&from_disk.0, &from_disk.1, &from_disk.2)
        }
    };
    let Some(byte) = docs::byte_offset(text, position.line, position.character) else { return Vec::new() };
    let Some(reference) = resolve::reference_at(tree, text, byte) else { return Vec::new() };
    let index = read(&state.index);
    resolve::definition(&index, summary, &reference).into_iter().map(|(f, d)| location(f, d)).collect()
}

pub fn workspace_symbols(state: &State, query: &str) -> Vec<SymbolInformation> {
    state.wait_ready();
    let query = query.to_lowercase();
    let index = read(&state.index);
    let mut found: Vec<(u8, &FileSummary, &Decl)> = index
        .decls()
        .filter_map(|(file, decl)| {
            let name = decl.name.to_lowercase();
            let rank = if name == query {
                0
            } else if name.starts_with(&query) {
                1
            } else if name.contains(&query) {
                2
            } else {
                return None;
            };
            Some((rank, file, decl))
        })
        .collect();
    found.sort_by(|(ra, fa, da), (rb, fb, db)| {
        (ra, &da.name, &fa.path, da.name_range.line).cmp(&(rb, &db.name, &fb.path, db.name_range.line))
    });
    found.into_iter().take(MAX_SYMBOLS).map(|(_, f, d)| symbol(f, d)).collect()
}

fn location(file: &FileSummary, decl: &Decl) -> Location {
    Location { uri: path_to_uri(&file.path), range: range(decl.name_range) }
}

fn range(span: Span) -> Range {
    Range { start: Position::new(span.line, span.start), end: Position::new(span.line, span.end) }
}

#[allow(deprecated)] // `SymbolInformation::deprecated` has to be spelled out.
fn symbol(file: &FileSummary, decl: &Decl) -> SymbolInformation {
    let kind = match decl.kind {
        DeclKind::Interface => SymbolKind::INTERFACE,
        DeclKind::Enum => SymbolKind::ENUM,
        DeclKind::EnumEntry => SymbolKind::ENUM_MEMBER,
        DeclKind::Class | DeclKind::Object | DeclKind::TypeAlias => SymbolKind::CLASS,
    };
    let container = decl.parent_fqn();
    SymbolInformation {
        name: decl.name.clone(),
        kind,
        tags: None,
        deprecated: None,
        location: location(file, decl),
        container_name: (!container.is_empty()).then(|| container.to_string()),
    }
}

pub fn run(connection: Connection) -> anyhow::Result<()> {
    let (id, params) = connection.initialize_start()?;
    let roots = roots_from_init(&params);
    let capabilities = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        definition_provider: Some(OneOf::Left(true)),
        workspace_symbol_provider: Some(OneOf::Left(true)),
        ..ServerCapabilities::default()
    };
    connection.initialize_finish(
        id,
        json!({
            "capabilities": capabilities,
            "serverInfo": {"name": "kotlin-navigator", "version": env!("CARGO_PKG_VERSION")},
        }),
    )?;

    let state = State::new(READY_TIMEOUT);
    let indexer_state = Arc::clone(&state);
    std::thread::Builder::new().name("indexer".into()).spawn(move || {
        let (index, stats) = scan::build_index(&roots);
        log::info!(
            "indexed {} files, {} declarations ({} with parse errors, {} unreadable) in {} ms",
            stats.files,
            stats.decls,
            stats.error_files,
            stats.skipped,
            stats.elapsed.as_millis()
        );
        indexer_state.finish_indexing(index);
    })?;

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(REQUEST_THREADS)
        .thread_name(|i| format!("request-{i}"))
        .build()?;
    for message in &connection.receiver {
        match message {
            Message::Request(request) => {
                if connection.handle_shutdown(&request)? {
                    return Ok(());
                }
                let state = Arc::clone(&state);
                let sender = connection.sender.clone();
                pool.spawn(move || {
                    let _ = sender.send(handle_request(&state, request).into());
                });
            }
            Message::Notification(notification) => handle_notification(&state, notification),
            Message::Response(_) => {}
        }
    }
    Ok(())
}

/// Workspace roots: `workspaceFolders`, else `rootUri`, else `rootPath`.
fn roots_from_init(params: &Value) -> Vec<PathBuf> {
    let to_path = |uri: &Value| uri.as_str()?.parse().ok().as_ref().and_then(uri_to_path);
    let folders: Vec<PathBuf> = params["workspaceFolders"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|folder| to_path(&folder["uri"]))
        .collect();
    if !folders.is_empty() {
        return folders;
    }
    if let Some(root) = to_path(&params["rootUri"]) {
        return vec![root];
    }
    params["rootPath"].as_str().map(PathBuf::from).into_iter().collect()
}

fn handle_request(state: &State, request: Request) -> Response {
    let id = request.id.clone();
    let method = request.method.clone();
    panic::catch_unwind(AssertUnwindSafe(|| dispatch(state, request))).unwrap_or_else(|panic| {
        let message = panic_message(panic.as_ref());
        log::error!("panic in {method}: {message}");
        Response::new_err(id, ErrorCode::InternalError as i32, message)
    })
}

fn dispatch(state: &State, request: Request) -> Response {
    let id = request.id.clone();
    match request.method.as_str() {
        GotoDefinition::METHOD => match serde_json::from_value::<GotoDefinitionParams>(request.params) {
            Ok(params) => {
                let at = params.text_document_position_params;
                let locations = uri_to_path(&at.text_document.uri)
                    .map(|path| definition(state, &path, at.position))
                    .unwrap_or_default();
                Response::new_ok(id, GotoDefinitionResponse::Array(locations))
            }
            Err(error) => invalid_params(id, error),
        },
        WorkspaceSymbolRequest::METHOD => match serde_json::from_value::<WorkspaceSymbolParams>(request.params) {
            Ok(params) => Response::new_ok(id, WorkspaceSymbolResponse::Flat(workspace_symbols(state, &params.query))),
            Err(error) => invalid_params(id, error),
        },
        method => Response::new_err(id, ErrorCode::MethodNotFound as i32, format!("unsupported request: {method}")),
    }
}

fn invalid_params(id: lsp_server::RequestId, error: serde_json::Error) -> Response {
    Response::new_err(id, ErrorCode::InvalidParams as i32, error.to_string())
}

fn handle_notification(state: &State, notification: Notification) {
    let method = notification.method.clone();
    let handled = panic::catch_unwind(AssertUnwindSafe(|| apply_notification(state, notification)));
    if let Err(panic) = handled {
        log::error!("panic in {method}: {}", panic_message(panic.as_ref()));
    }
}

fn apply_notification(state: &State, notification: Notification) {
    let params = notification.params;
    match notification.method.as_str() {
        DidOpenTextDocument::METHOD => {
            if let Ok(p) = serde_json::from_value::<lsp_types::DidOpenTextDocumentParams>(params) {
                if let Some(path) = uri_to_path(&p.text_document.uri) {
                    state.open_or_change(path, p.text_document.text);
                }
            }
        }
        DidChangeTextDocument::METHOD => {
            if let Ok(p) = serde_json::from_value::<lsp_types::DidChangeTextDocumentParams>(params) {
                // Full sync: the last change without a range carries the whole text.
                let text = p.content_changes.into_iter().rev().find(|c| c.range.is_none()).map(|c| c.text);
                if let (Some(path), Some(text)) = (uri_to_path(&p.text_document.uri), text) {
                    state.open_or_change(path, text);
                }
            }
        }
        DidCloseTextDocument::METHOD => {
            if let Ok(p) = serde_json::from_value::<lsp_types::DidCloseTextDocumentParams>(params) {
                if let Some(path) = uri_to_path(&p.text_document.uri) {
                    state.close(&path);
                }
            }
        }
        DidSaveTextDocument::METHOD => {
            if let Ok(p) = serde_json::from_value::<lsp_types::DidSaveTextDocumentParams>(params) {
                if let Some(path) = uri_to_path(&p.text_document.uri) {
                    state.reload_from_disk(&path);
                }
            }
        }
        DidChangeWatchedFiles::METHOD => {
            if let Ok(p) = serde_json::from_value::<lsp_types::DidChangeWatchedFilesParams>(params) {
                // Created, changed and deleted all come down to re-reading: a deleted file fails to read.
                for path in p.changes.iter().filter_map(|c| uri_to_path(&c.uri)) {
                    state.reload_from_disk(&path);
                }
            }
        }
        _ => {}
    }
}

fn panic_message(panic: &(dyn Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic".to_string())
}
