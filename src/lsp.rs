//! LSP server: the main thread applies buffer updates, requests run on a
//! small thread pool, the initial index is built in the background.

use std::any::Any;
use std::collections::HashMap;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::thread::JoinHandle;
use std::time::Duration;

use lsp_server::{Connection, ErrorCode, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidChangeWatchedFiles, DidCloseTextDocument, DidOpenTextDocument, DidSaveTextDocument,
    Notification as _,
};
use lsp_types::request::{
    DocumentHighlightRequest, GotoDefinition, GotoImplementation, GotoTypeDefinition, References, Request as _,
    WorkspaceSymbolRequest,
};
use lsp_types::{
    CancelParams, DocumentHighlight, DocumentHighlightKind, DocumentHighlightParams, GotoDefinitionParams,
    GotoDefinitionResponse, ImplementationProviderCapability, Location, NumberOrString, OneOf, Position, Range,
    ReferenceParams, ServerCapabilities, SymbolInformation, SymbolKind, TextDocumentSyncCapability, TextDocumentSyncKind,
    TypeDefinitionProviderCapability, WorkspaceSymbolParams, WorkspaceSymbolResponse,
};
use serde_json::{Value, json};
use tree_sitter::Tree;

use crate::cancel::{Cancel, Cancelled};
use crate::docs::{self, Documents, OpenDoc, path_to_uri, uri_to_path};
use crate::index::Index;
use crate::model::{Decl, DeclKind, FileSummary, Span};
use crate::sources::Sources;
use crate::{highlight, parse, resolve, scan, usages};

pub const READY_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_SYMBOLS: usize = 100;
pub const REQUEST_THREADS: usize = 4;

pub struct State {
    pub index: RwLock<Index>,
    pub docs: RwLock<Documents>,
    roots: Vec<PathBuf>,
    ready: Mutex<bool>,
    ready_changed: Condvar,
    ready_timeout: Duration,
    /// Cancel flags of the requests in flight.
    requests: Mutex<HashMap<RequestId, Cancel>>,
}

impl State {
    pub fn new(roots: Vec<PathBuf>, ready_timeout: Duration) -> Arc<State> {
        Arc::new(State {
            index: RwLock::default(),
            docs: RwLock::default(),
            roots,
            ready: Mutex::new(false),
            ready_changed: Condvar::new(),
            ready_timeout,
            requests: Mutex::default(),
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

    /// Whether `path` belongs in the index: the scan found it, or it lies where the scan
    /// looks. Buffers from elsewhere (other worktrees, generated sources) still navigate.
    fn indexable(&self, index: &Index, path: &Path) -> bool {
        scan::is_indexed(path)
            && (index.file(path).is_some() || self.roots.is_empty() || scan::in_workspace(&self.roots, path))
    }

    pub fn open_or_change(&self, path: PathBuf, text: String) {
        let module = scan::module_of(&path, &self.roots);
        // Parsed outside the lock, so requests meanwhile read the previous buffer, which
        // also stays when parsing gives up.
        let Some(doc) = OpenDoc::parse(path.clone(), text, module) else { return };
        let mut docs = write(&self.docs);
        let doc = docs.insert(doc);
        let mut index = write(&self.index);
        if self.indexable(&index, &path) {
            index.upsert(doc.summary.clone());
        }
    }

    pub fn close(&self, path: &Path) {
        write(&self.docs).close(path);
        self.reload_from_disk(path);
    }

    /// Re-reads a file that is not open in the editor; a missing or unreadable file leaves the
    /// index, one whose parse gives up keeps what was indexed.
    pub fn reload_from_disk(&self, path: &Path) {
        let docs = read(&self.docs);
        if docs.get(path).is_some() || !self.indexable(&read(&self.index), path) {
            return;
        }
        let Some(text) = scan::read_source(path) else {
            write(&self.index).remove(path);
            return;
        };
        if let Some((_, summary)) = self.parse_from_disk(path, &text) {
            write(&self.index).upsert(summary);
        }
    }

    fn parse_from_disk(&self, path: &Path, text: &str) -> Option<(Tree, FileSummary)> {
        let (tree, mut summary) = parse::summarize_text(path.to_path_buf(), text)?;
        summary.module = scan::module_of(path, &self.roots);
        Some((tree, summary))
    }

    /// A cancel flag for request `id`, kept until `end_request`.
    fn begin_request(&self, id: &RequestId) -> Cancel {
        let cancel = Cancel::default();
        self.requests().insert(id.clone(), cancel.clone());
        cancel
    }

    fn end_request(&self, id: &RequestId) {
        self.requests().remove(id);
    }

    /// Raises the flag of request `id`; one already answered or never seen is ignored.
    fn cancel_request(&self, id: &RequestId) {
        if let Some(cancel) = self.requests().get(id) {
            cancel.set();
        }
    }

    fn requests(&self) -> MutexGuard<'_, HashMap<RequestId, Cancel>> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Adds scanned files to the live index; an open buffer wins over what was read from disk.
    pub fn publish(&self, summaries: Vec<FileSummary>) {
        let docs = read(&self.docs);
        let mut index = write(&self.index);
        for summary in summaries {
            match docs.get(&summary.path) {
                Some(doc) => index.upsert(doc.summary.clone()),
                None => index.upsert(summary),
            }
        }
    }
}

/// Runs `scan` on its own thread, publishing what it hands over; the server is marked
/// ready when it ends, even by a panic.
pub fn index_in_background(
    state: Arc<State>,
    scan: impl FnOnce(&mut dyn FnMut(Vec<FileSummary>)) + Send + 'static,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("indexer".into())
        .spawn(move || {
            let scanned = panic::catch_unwind(AssertUnwindSafe(|| scan(&mut |chunk| state.publish(chunk))));
            if let Err(panic) = scanned {
                log::error!("panic in the indexer: {}", panic_message(panic.as_ref()));
            }
            state.mark_ready();
        })
        .expect("the indexer thread starts")
}

fn read<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(PoisonError::into_inner)
}

fn write<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(PoisonError::into_inner)
}

/// The open buffers and what `take` makes of the index, as of one moment: the index is
/// locked before the buffers' lock is released, so an edit cannot fall in between.
fn snapshot<'a, T>(state: &'a State, take: impl FnOnce(RwLockReadGuard<'a, Index>) -> T) -> (Sources, T) {
    let docs = read(&state.docs);
    let sources = Sources::new(docs.snapshot());
    (sources, take(read(&state.index)))
}

/// The buffer of `path` and the byte `position` falls on.
fn cursor(sources: &Sources, path: &Path, position: Position) -> Option<(Arc<OpenDoc>, usize)> {
    let doc = sources.load(path)?;
    let byte = docs::byte_offset(&doc.text, position.line, position.character)?;
    Some((doc, byte))
}

pub fn definition(state: &State, path: &Path, position: Position) -> Vec<Location> {
    state.wait_ready();
    // The index stays locked for reading while resolving.
    let (sources, index) = snapshot(state, |index| index);
    let Some((doc, byte)) = cursor(&sources, path, position) else { return Vec::new() };
    let current = resolve::Current::of(&index, &doc);
    locations(resolve::definition(&index, &current, &sources, byte))
}

/// Usages of the symbol at `position`. The search runs on a copy of the index, so no lock
/// is held while it goes through the files.
pub fn references(
    state: &State,
    path: &Path,
    position: Position,
    include_declaration: bool,
    cancel: &Cancel,
) -> Result<Vec<Location>, Cancelled> {
    state.wait_ready();
    let (sources, index) = snapshot(state, |index| Index::clone(&index));
    let Some((doc, byte)) = cursor(&sources, path, position) else { return Ok(Vec::new()) };
    let current = resolve::Current::of(&index, &doc);
    let found = usages::references(&index, &current, &sources, byte, include_declaration, cancel)?;
    Ok(found.into_iter().map(|u| Location { uri: path_to_uri(&u.path), range: range(u.range) }).collect())
}

/// Implementations of the symbol at `position`, searched on a copy of the index like `references`.
pub fn implementation(state: &State, path: &Path, position: Position, cancel: &Cancel) -> Result<Vec<Location>, Cancelled> {
    state.wait_ready();
    let (sources, index) = snapshot(state, |index| Index::clone(&index));
    let Some((doc, byte)) = cursor(&sources, path, position) else { return Ok(Vec::new()) };
    let current = resolve::Current::of(&index, &doc);
    Ok(locations(resolve::family::implementation(&index, &current, &sources, byte, cancel)?))
}

pub fn type_definition(state: &State, path: &Path, position: Position) -> Vec<Location> {
    state.wait_ready();
    let (sources, index) = snapshot(state, |index| index);
    let Some((doc, byte)) = cursor(&sources, path, position) else { return Vec::new() };
    let current = resolve::Current::of(&index, &doc);
    locations(resolve::typedef::type_definition(&index, &current, &sources, byte))
}

pub fn highlight(state: &State, path: &Path, position: Position) -> Vec<DocumentHighlight> {
    state.wait_ready();
    let (sources, index) = snapshot(state, |index| index);
    let Some((doc, byte)) = cursor(&sources, path, position) else { return Vec::new() };
    let current = resolve::Current::of(&index, &doc);
    highlight::highlight(&index, &current, &sources, byte)
        .into_iter()
        .map(|h| DocumentHighlight {
            range: range(h.range),
            kind: Some(if h.write { DocumentHighlightKind::WRITE } else { DocumentHighlightKind::READ }),
        })
        .collect()
}

fn locations(targets: Vec<resolve::Target>) -> Vec<Location> {
    targets.into_iter().map(|t| Location { uri: path_to_uri(&t.path), range: range(t.range) }).collect()
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
        DeclKind::Function => SymbolKind::FUNCTION,
        DeclKind::Property => SymbolKind::PROPERTY,
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
        references_provider: Some(OneOf::Left(true)),
        implementation_provider: Some(ImplementationProviderCapability::Simple(true)),
        type_definition_provider: Some(TypeDefinitionProviderCapability::Simple(true)),
        document_highlight_provider: Some(OneOf::Left(true)),
        ..ServerCapabilities::default()
    };
    connection.initialize_finish(
        id,
        json!({
            "capabilities": capabilities,
            "serverInfo": {"name": "kotlin-navigator", "version": env!("CARGO_PKG_VERSION")},
        }),
    )?;

    let state = State::new(roots.clone(), READY_TIMEOUT);
    index_in_background(Arc::clone(&state), move |publish| {
        let stats = scan::build_index_with(&roots, publish);
        log::info!(
            "indexed {} files, {} declarations ({} with parse errors, {} unreadable) in {} ms",
            stats.files,
            stats.decls,
            stats.error_files,
            stats.skipped,
            stats.elapsed.as_millis()
        );
    });

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
                // Registered before queueing, so a `$/cancelRequest` that comes next finds it.
                let cancel = state.begin_request(&request.id);
                let state = Arc::clone(&state);
                let sender = connection.sender.clone();
                pool.spawn(move || {
                    let id = request.id.clone();
                    let _ = sender.send(answer(&state, request, &cancel).into());
                    state.end_request(&id);
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

/// The response to `request`: `RequestCanceled` when `cancel` is raised before or while it
/// runs, else what `handle_request` gives.
pub fn answer(state: &State, request: Request, cancel: &Cancel) -> Response {
    let id = request.id.clone();
    if cancel.is_set() {
        return canceled(id);
    }
    let response = handle_request(state, request, cancel);
    // A request that does not poll the flag runs to the end even when cancelled meanwhile.
    if cancel.is_set() { canceled(id) } else { response }
}

fn handle_request(state: &State, request: Request, cancel: &Cancel) -> Response {
    let id = request.id.clone();
    let method = request.method.clone();
    panic::catch_unwind(AssertUnwindSafe(|| dispatch(state, request, cancel))).unwrap_or_else(|panic| {
        let message = panic_message(panic.as_ref());
        log::error!("panic in {method}: {message}");
        Response::new_err(id, ErrorCode::InternalError as i32, message)
    })
}

fn dispatch(state: &State, request: Request, cancel: &Cancel) -> Response {
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
        References::METHOD => match serde_json::from_value::<ReferenceParams>(request.params) {
            Ok(params) => {
                let at = params.text_document_position;
                let include_declaration = params.context.include_declaration;
                let found = uri_to_path(&at.text_document.uri)
                    .map_or(Ok(Vec::new()), |path| references(state, &path, at.position, include_declaration, cancel));
                match found {
                    Ok(locations) => Response::new_ok(id, Some(locations)),
                    Err(Cancelled) => canceled(id),
                }
            }
            Err(error) => invalid_params(id, error),
        },
        GotoImplementation::METHOD => match serde_json::from_value::<GotoDefinitionParams>(request.params) {
            Ok(params) => {
                let at = params.text_document_position_params;
                let found = uri_to_path(&at.text_document.uri)
                    .map_or(Ok(Vec::new()), |path| implementation(state, &path, at.position, cancel));
                match found {
                    Ok(locations) => Response::new_ok(id, GotoDefinitionResponse::Array(locations)),
                    Err(Cancelled) => canceled(id),
                }
            }
            Err(error) => invalid_params(id, error),
        },
        GotoTypeDefinition::METHOD => match serde_json::from_value::<GotoDefinitionParams>(request.params) {
            Ok(params) => {
                let at = params.text_document_position_params;
                let locations = uri_to_path(&at.text_document.uri)
                    .map(|path| type_definition(state, &path, at.position))
                    .unwrap_or_default();
                Response::new_ok(id, GotoDefinitionResponse::Array(locations))
            }
            Err(error) => invalid_params(id, error),
        },
        DocumentHighlightRequest::METHOD => match serde_json::from_value::<DocumentHighlightParams>(request.params) {
            Ok(params) => {
                let at = params.text_document_position_params;
                let highlights = uri_to_path(&at.text_document.uri)
                    .map(|path| highlight(state, &path, at.position))
                    .unwrap_or_default();
                Response::new_ok(id, Some(highlights))
            }
            Err(error) => invalid_params(id, error),
        },
        method => Response::new_err(id, ErrorCode::MethodNotFound as i32, format!("unsupported request: {method}")),
    }
}

fn invalid_params(id: RequestId, error: serde_json::Error) -> Response {
    Response::new_err(id, ErrorCode::InvalidParams as i32, error.to_string())
}

fn canceled(id: RequestId) -> Response {
    Response::new_err(id, ErrorCode::RequestCanceled as i32, "canceled".to_string())
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
        lsp_types::notification::Cancel::METHOD => {
            if let Ok(p) = serde_json::from_value::<CancelParams>(params) {
                let id = match p.id {
                    NumberOrString::Number(n) => RequestId::from(n),
                    NumberOrString::String(s) => RequestId::from(s),
                };
                state.cancel_request(&id);
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
