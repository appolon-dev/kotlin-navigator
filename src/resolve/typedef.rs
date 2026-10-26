//! Go to type definition: the class a variable, property or call result is an instance of.

use super::members::resolve_type;
use super::rank::{DeclRef, rank};
use super::{Current, Found, Target, target_at};
use crate::index::Index;
use crate::model::DeclKind;
use crate::sources::Sources;

/// The declarations of the type of what the identifier at `byte` refers to, best first and
/// without repeats. A class is its own type; a type outside the project leads nowhere.
pub fn type_definition(index: &Index, current: &Current, sources: &Sources, byte: usize) -> Vec<Target> {
    let Some(resolution) = target_at(index, current, sources, byte) else { return Vec::new() };
    let mut types: Vec<DeclRef> = Vec::new();
    for found in &resolution.found {
        let own = match found {
            Found::Local(local) => {
                let file = if local.path == current.summary.path { Some(current.summary) } else { index.file(&local.path) };
                match (file, &local.type_name) {
                    (Some(file), Some(name)) => resolve_type(index, file, name),
                    _ => Vec::new(),
                }
            }
            Found::Decl(own) if own.1.kind.is_class_like() => vec![*own],
            Found::Decl((file, decl)) if matches!(decl.kind, DeclKind::Function | DeclKind::Property) => {
                decl.type_hint.as_deref().map(|hint| resolve_type(index, file, hint)).unwrap_or_default()
            }
            Found::Decl(_) => Vec::new(),
        };
        types.extend(own);
    }
    let mut targets: Vec<Target> = Vec::new();
    for (file, decl) in rank(index, current.summary, types) {
        let target = Target { path: file.path.clone(), range: decl.name_range };
        if !targets.contains(&target) {
            targets.push(target);
        }
    }
    targets
}
