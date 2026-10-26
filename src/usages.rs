//! Find Usages: the places that refer to a declaration and its family, or to a local.

use std::path::{Path, PathBuf};

use rayon::prelude::*;
use tree_sitter::{Node, Tree};

use crate::cancel::{Cancel, Cancelled};
use crate::docs::byte_offset;
use crate::index::Index;
use crate::model::{FileSummary, Flags, Span};
use crate::parse::{span_at, span_of};
use crate::resolve::args::parameter_owner;
use crate::resolve::family::family;
use crate::resolve::rank::tier;
use crate::resolve::{Certainty, Current, Found, Resolution, target_at};
use crate::source_set::sees;
use crate::sources::Sources;

/// A place that refers to the subject, and how sure the resolver is of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Usage {
    pub path: PathBuf,
    pub range: Span,
    pub certainty: Certainty,
}

/// What usages are searched for: the declarations under the cursor and their families.
pub(crate) struct Subject<'a> {
    /// The name occurrences are looked for by; for `import a.B as C` it is `B`.
    pub name: String,
    pub targets: Vec<Found<'a>>,
    pub family: Vec<Found<'a>>,
}

/// The subject of the identifier at `byte`; `None` when it refers to nothing.
pub(crate) fn subject_at<'a>(
    index: &'a Index,
    current: &Current<'a>,
    sources: &Sources,
    byte: usize,
    cancel: &Cancel,
) -> Result<Option<Subject<'a>>, Cancelled> {
    let Some(resolution) = target_at(index, current, sources, byte) else { return Ok(None) };
    let name = match resolution.found.first() {
        Some(Found::Decl((_, decl))) => decl.name.clone(),
        Some(Found::Local(_)) => resolution.reference.name.clone(),
        None => return Ok(None),
    };
    let mut members: Vec<Found<'a>> = Vec::new();
    for target in &resolution.found {
        let related = match target {
            Found::Decl(decl) => family(index, *decl, cancel)?.into_iter().map(Found::Decl).collect(),
            Found::Local(_) => vec![target.clone()],
        };
        for member in related {
            if !members.iter().any(|m| place(m) == place(&member)) {
                members.push(member);
            }
        }
    }
    Ok(Some(Subject { name, targets: resolution.found, family: members }))
}

/// Usages of what the identifier at `byte` declares or refers to, sorted by (path, line,
/// start), each place once. Declarations of the family are included; the target's own
/// only with `include_declaration`.
pub fn references(
    index: &Index,
    current: &Current,
    sources: &Sources,
    byte: usize,
    include_declaration: bool,
    cancel: &Cancel,
) -> Result<Vec<Usage>, Cancelled> {
    let Some(subject) = subject_at(index, current, sources, byte, cancel)? else { return Ok(Vec::new()) };
    let files = scope(index, current, sources, &subject, cancel)?;
    let declarations = subject.family.iter().map(|member| {
        let (path, range) = place(member);
        Usage { path: path.to_path_buf(), range, certainty: Certainty::Exact }
    });
    let mut usages: Vec<Usage> = search(index, sources, &files, &subject, cancel)?.into_iter().chain(declarations).collect();
    if !include_declaration {
        usages.retain(|u| !subject.targets.iter().any(|t| place(t) == (u.path.as_path(), u.range)));
    }
    usages.sort_by(|a, b| (&a.path, a.range.line, a.range.start, a.certainty).cmp(&(&b.path, b.range.line, b.range.start, b.certainty)));
    // Of two finds of one place the first, `Exact`, stays.
    usages.dedup_by(|later, kept| later.path == kept.path && later.range == kept.range);
    Ok(usages)
}

/// Usages of `subject` in `files`, a file per task on the pool of the caller: the request
/// pool in the server.
fn search(index: &Index, sources: &Sources, files: &[PathBuf], subject: &Subject, cancel: &Cancel) -> Result<Vec<Usage>, Cancelled> {
    let found: Vec<Vec<Usage>> =
        files.par_iter().map(|path| usages_in(index, sources, path, subject, cancel)).collect::<Result<_, _>>()?;
    Ok(found.into_iter().flatten().collect())
}

/// The files a usage of the family may be in: those its visibility allows and, for a
/// parameter, the files that use its function or class, where the `name =` labels are.
fn scope(index: &Index, current: &Current, sources: &Sources, subject: &Subject, cancel: &Cancel) -> Result<Vec<PathBuf>, Cancelled> {
    let (mut files, all) = visible_scope(index, current, subject);
    if all {
        return Ok(files);
    }
    for member in &subject.family {
        let Some(owner) = owner_of(index, current, sources, member, cancel)? else { continue };
        let calls = search(index, sources, &visible_scope(index, current, &owner).0, &owner, cancel)?;
        files.extend(calls.into_iter().map(|u| u.path));
    }
    files.sort();
    files.dedup();
    Ok(files)
}

/// The indexed files and the current one that a usage of a family member may be in:
/// a `private` member's file, an `internal` one's module, a local's file, else all; and
/// whether that is every file.
fn visible_scope(index: &Index, current: &Current, subject: &Subject) -> (Vec<PathBuf>, bool) {
    let mut files: Vec<(&Path, Option<&Path>)> = index.files().map(|f| (f.path.as_path(), f.module.as_deref())).collect();
    if index.file(&current.summary.path).is_none() {
        files.push((&current.summary.path, current.summary.module.as_deref()));
    }
    let wanted = |path: &Path, module: Option<&Path>| {
        subject.family.iter().any(|member| match member {
            Found::Local(local) => local.path == path,
            Found::Decl((file, decl)) if decl.flags.contains(Flags::PRIVATE) => file.path == path,
            Found::Decl((file, decl)) if decl.flags.contains(Flags::INTERNAL) => {
                file.module.is_none() || file.module.as_deref() == module
            }
            Found::Decl(_) => true,
        })
    };
    let total = files.len();
    let kept: Vec<PathBuf> = files.into_iter().filter(|&(path, module)| wanted(path, module)).map(|(path, _)| path.to_path_buf()).collect();
    let all = kept.len() == total;
    (kept, all)
}

/// The function or class whose parameter `member` is, as a subject: its usages hold the
/// calls the parameter's labels are in. `None` for anything else, and for a local function
/// or class, whose calls are in the parameter's own file. The owner is looked up by place,
/// not resolved by name: in `fun title(title: String)` the name resolves to the parameter.
fn owner_of<'a>(index: &'a Index, current: &Current<'a>, sources: &Sources, member: &Found, cancel: &Cancel) -> Result<Option<Subject<'a>>, Cancelled> {
    let (path, range) = place(member);
    let here = path == current.summary.path;
    let loaded;
    let (text, tree) = if here {
        (current.text, current.tree)
    } else {
        let Some(doc) = sources.load(path) else { return Ok(None) };
        loaded = doc;
        (loaded.text.as_str(), &loaded.tree)
    };
    let name = byte_offset(text, range.line, range.start).and_then(|b| tree.root_node().descendant_for_byte_range(b, b));
    let Some(owner) = name.and_then(parameter_owner) else { return Ok(None) };
    let span = span_of(text, owner);
    let summary = if here { Some(current.summary) } else { index.file(path) };
    let Some(decl) = summary.and_then(|f| f.decls.iter().find(|d| d.name_range == span).map(|d| (f, d))) else { return Ok(None) };
    let family = family(index, decl, cancel)?.into_iter().map(Found::Decl).collect();
    Ok(Some(Subject { name: decl.1.name.clone(), targets: vec![Found::Decl(decl)], family }))
}

/// Usages of `subject` in the file at `path`: its occurrences of the name that the
/// resolver ties to the family.
fn usages_in(index: &Index, sources: &Sources, path: &Path, subject: &Subject, cancel: &Cancel) -> Result<Vec<Usage>, Cancelled> {
    cancel.check()?;
    let Some(doc) = sources.load_containing(path, &subject.name) else { return Ok(Vec::new()) };
    let current = Current::of(index, &doc);
    let mut found = Vec::new();
    for byte in occurrences(&doc.tree, &doc.text, &subject.name) {
        // `target_at`, not `resolve`: on its own name, `go(String)` is no usage of `go(Int)`.
        let Some(resolution) = target_at(index, &current, sources, byte) else { continue };
        if let Some(certainty) = certainty_of(current.summary, &resolution, subject) {
            let range = span_at(&doc.text, resolution.reference.range);
            found.push(Usage { path: path.to_path_buf(), range, certainty });
        }
    }
    Ok(found)
}

/// Start bytes of the identifiers named `name` (backticks dropped) and of `"$name"` templates.
pub(crate) fn occurrences(tree: &Tree, text: &str, name: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut cursor = tree.walk();
    loop {
        let node = cursor.node();
        let hit = match node.kind() {
            // `ident_text` without the allocation.
            "identifier" | "type_identifier" => text[node.byte_range()].trim_matches('`') == name,
            "string_content" => is_template(text, node, name),
            _ => false,
        };
        if hit {
            out.push(node.start_byte());
        }
        if cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                return out;
            }
        }
    }
}

/// `"$name…"`: a `string_content` right after a `string_content` `$` that starts with the
/// whole name.
fn is_template(text: &str, node: Node, name: &str) -> bool {
    let whole = text[node.byte_range()].strip_prefix(name).is_some_and(|rest| !rest.starts_with(|c: char| c == '_' || c.is_alphanumeric()));
    whole && node.prev_sibling().is_some_and(|p| p.kind() == "string_content" && &text[p.byte_range()] == "$")
}

/// Whether an occurrence in `file` that resolved as `resolution` is a usage of `subject`,
/// and how sure. A name search counts when a family member fits the role, is visible from
/// the file and lives in a source set the file sees.
pub(crate) fn certainty_of(file: &FileSummary, resolution: &Resolution, subject: &Subject) -> Option<Certainty> {
    if resolution.found.iter().any(|f| subject.family.iter().any(|m| place(m) == place(f))) {
        return Some(resolution.certainty);
    }
    let fits = |member: &Found| match member {
        Found::Decl((f, d)) => {
            tier(&resolution.reference.role, d.kind).is_some()
                && (!d.flags.contains(Flags::PRIVATE) || f.path == file.path)
                && sees(file.source_set.as_deref(), f.source_set.as_deref())
        }
        Found::Local(_) => false,
    };
    (resolution.certainty == Certainty::Probable && subject.family.iter().any(fits)).then_some(Certainty::Probable)
}

/// Where a declaration's name is: `Found::target` without cloning the path.
fn place<'f>(found: &'f Found) -> (&'f Path, Span) {
    match found {
        Found::Decl((file, decl)) => (&file.path, decl.name_range),
        Found::Local(local) => (&local.path, local.range),
    }
}
