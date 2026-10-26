//! What is under the cursor, and which declarations it may refer to.

pub mod context;

use tree_sitter::{Node, Tree};

use self::context::import_fqn;

use crate::index::Index;
use crate::model::{Decl, FileSummary, Import};
use crate::parse::ident_text;

pub const MAX_CANDIDATES: usize = 50;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    pub name: String,
    /// Set when the identifier is part of an import path: the path up to and including it.
    pub import_fqn: Option<String>,
}

pub fn reference_at(tree: &Tree, text: &str, byte: usize) -> Option<Reference> {
    let root = tree.root_node();
    // The cursor may sit right after the name, e.g. `Foo|)`.
    let node = identifier_at(root, byte).or_else(|| identifier_at(root, byte.checked_sub(1)?))?;
    Some(Reference { name: ident_text(text, node), import_fqn: import_fqn(text, node) })
}

fn identifier_at(root: Node, byte: usize) -> Option<Node> {
    let node = root.named_descendant_for_byte_range(byte, byte)?;
    matches!(node.kind(), "identifier" | "type_identifier").then_some(node)
}

/// Candidates in Python-version order: import path, explicit import, own file,
/// own package, wildcard imports, then everything (capped).
pub fn definition<'a>(index: &'a Index, current: &FileSummary, reference: &Reference) -> Vec<(&'a FileSummary, &'a Decl)> {
    if let Some(fqn) = &reference.import_fqn {
        return by_fqn(index, fqn);
    }
    if let Some(import) = current.imports.iter().find(|i| !i.wildcard && import_key(i) == reference.name) {
        // An explicit import names exactly one class; same-named project classes are never it.
        return by_fqn(index, &import.path);
    }
    let candidates = index.lookup(&reference.name);
    let levels: [&dyn Fn(&FileSummary, &Decl) -> bool; 3] = [
        &|f, _| f.path == current.path,
        &|f, _| f.package == current.package,
        &|_, d| current.imports.iter().any(|i| i.wildcard && i.path == d.parent_fqn()),
    ];
    for keep in levels {
        let selected: Vec<_> = candidates.iter().copied().filter(|(f, d)| keep(f, d)).collect();
        if !selected.is_empty() {
            return selected;
        }
    }
    candidates.into_iter().take(MAX_CANDIDATES).collect()
}

fn by_fqn<'a>(index: &'a Index, fqn: &str) -> Vec<(&'a FileSummary, &'a Decl)> {
    let name = fqn.rsplit('.').next().unwrap_or(fqn);
    index.lookup(name).into_iter().filter(|(_, d)| d.fqn == fqn).collect()
}

fn import_key(import: &Import) -> &str {
    import.alias.as_deref().unwrap_or_else(|| import.path.rsplit('.').next().unwrap_or(&import.path))
}
