//! Types by name, and the members a type offers: its own (and its companion's), its
//! supertypes', and extensions declared on it.

use std::collections::HashSet;
use std::path::Path;

use crate::index::Index;
use crate::model::{Decl, FileSummary, Flags};
use crate::resolve::by_fqn;
use crate::resolve::rank::DeclRef;

/// Supertypes are followed this many levels deep.
const MAX_DEPTH: usize = 5;

/// A type as written (`name`) and the class-like declarations it may mean.
#[derive(Clone, Debug)]
pub struct Ty<'a> {
    pub name: String,
    pub decls: Vec<DeclRef<'a>>,
}

/// Class-like declarations a type name written in the file `context` means.
pub fn resolve_type<'a>(index: &'a Index, context: &'a FileSummary, name: &str) -> Vec<DeclRef<'a>> {
    let Some((first, rest)) = name.split_once('.') else { return simple_type(index, context, name) };
    let exact = class_like(by_fqn(index, name));
    if !exact.is_empty() {
        return exact;
    }
    // `Outer.Inner`: the outer type by its simple name, then down its nested types.
    let mut found = simple_type(index, context, first);
    for segment in rest.split('.') {
        found = found.iter().flat_map(|&(f, d)| children(f, d).filter(move |c| c.name == segment && c.kind.is_class_like()).map(move |c| (f, c))).collect();
    }
    if !found.is_empty() {
        return found;
    }
    simple_type(index, context, name.rsplit('.').next().unwrap_or(name))
}

fn simple_type<'a>(index: &'a Index, context: &'a FileSummary, name: &str) -> Vec<DeclRef<'a>> {
    let key = |path: &'a str, alias: &'a Option<String>| alias.as_deref().unwrap_or_else(|| path.rsplit('.').next().unwrap_or(path));
    if let Some(import) = context.imports.iter().find(|i| !i.wildcard && key(&i.path, &i.alias) == name) {
        return class_like(by_fqn(index, &import.path));
    }
    let own: Vec<DeclRef> = context.decls.iter().filter(|d| d.name == name && d.kind.is_class_like()).map(|d| (context, d)).collect();
    if !own.is_empty() {
        return own;
    }
    let candidates = class_like(index.lookup(name));
    let levels: [&dyn Fn(&FileSummary, &Decl) -> bool; 2] = [
        &|f, _| f.package == context.package,
        &|_, d| context.imports.iter().any(|i| i.wildcard && i.path == d.parent_fqn()),
    ];
    levels
        .iter()
        .map(|keep| candidates.iter().copied().filter(|(f, d)| keep(f, d)).collect::<Vec<_>>())
        .find(|selected| !selected.is_empty())
        .unwrap_or(candidates)
}

/// The first non-empty group: members of T (and of its companion when `companion`),
/// members of its supertypes, extensions on it or its supertypes.
pub fn members_named<'a>(index: &'a Index, ty: &Ty<'a>, name: &str, companion: bool) -> Vec<DeclRef<'a>> {
    let own: Vec<DeclRef> = ty.decls.iter().flat_map(|&(f, d)| own_members(f, d, name, companion)).collect();
    if !own.is_empty() {
        return own;
    }
    let (inherited, super_names) = supertype_members(index, ty, name);
    if !inherited.is_empty() {
        return inherited;
    }
    let last = |s: &str| s.rsplit('.').next().unwrap_or(s).to_string();
    let mut receivers: HashSet<String> = super_names;
    receivers.insert(last(&ty.name));
    index.lookup(name).into_iter().filter(|(_, d)| d.receiver.as_deref().is_some_and(|r| receivers.contains(&last(r)))).collect()
}

/// Members named `name` on the nearest supertype level that has any (breadth-first,
/// `MAX_DEPTH` levels, each type once), and the last segments of every supertype name met.
pub fn supertype_members<'a>(index: &'a Index, ty: &Ty<'a>, name: &str) -> (Vec<DeclRef<'a>>, HashSet<String>) {
    let mut names = HashSet::new();
    let mut seen: HashSet<(&Path, usize)> = ty.decls.iter().filter_map(|&(f, d)| Some((f.path.as_path(), position(f, d)?))).collect();
    let mut level: Vec<DeclRef> = ty.decls.clone();
    for _ in 0..MAX_DEPTH {
        let mut next = Vec::new();
        for (f, d) in level {
            for written in &d.supertypes {
                names.insert(written.rsplit('.').next().unwrap_or(written).to_string());
                for (sf, sd) in resolve_type(index, f, written) {
                    if position(sf, sd).is_some_and(|i| seen.insert((sf.path.as_path(), i))) {
                        next.push((sf, sd));
                    }
                }
            }
        }
        let found: Vec<DeclRef> = next.iter().flat_map(|&(f, d)| own_members(f, d, name, false)).collect();
        if !found.is_empty() || next.is_empty() {
            return (found, names);
        }
        level = next;
    }
    (Vec::new(), names)
}

/// Declarations named `name` directly inside `owner`, plus inside its companion when asked.
pub(crate) fn own_members<'a>(file: &'a FileSummary, owner: &'a Decl, name: &str, companion: bool) -> Vec<DeclRef<'a>> {
    let mut found: Vec<DeclRef> = children(file, owner).filter(|d| d.name == name).map(|d| (file, d)).collect();
    if companion {
        for c in children(file, owner).filter(|c| c.flags.contains(Flags::COMPANION)) {
            found.extend(children(file, c).filter(|d| d.name == name).map(|d| (file, d)));
        }
    }
    found
}

/// The declaration enclosing `decl` in its file.
pub(crate) fn container<'a>(file: &'a FileSummary, decl: &Decl) -> Option<&'a Decl> {
    decl.container.and_then(|i| file.decls.get(i as usize))
}

fn children<'a>(file: &'a FileSummary, owner: &'a Decl) -> impl Iterator<Item = &'a Decl> {
    let index = position(file, owner).map(|i| i as u32);
    file.decls.iter().filter(move |d| index.is_some() && d.container == index)
}

/// Index of `decl` in its file's `decls`.
fn position(file: &FileSummary, decl: &Decl) -> Option<usize> {
    file.decls.iter().position(|d| std::ptr::eq(d, decl))
}

fn class_like(found: Vec<DeclRef>) -> Vec<DeclRef> {
    found.into_iter().filter(|(_, d)| d.kind.is_class_like()).collect()
}
