//! Types by name, and the members a type offers: its own (and its companion's), its
//! supertypes', and extensions declared on it.

use std::collections::HashSet;
use std::path::Path;

use crate::index::Index;
use crate::model::{Decl, FileSummary, Flags, Lang};
use crate::resolve::by_fqn;
use crate::resolve::rank::DeclRef;

/// Supertypes are followed this many levels deep.
const MAX_DEPTH: usize = 5;

/// A type as written (`name`) and the class-like declarations it may mean.
#[derive(Clone, Debug)]
pub struct Ty<'a> {
    pub name: String,
    pub decls: Vec<DeclRef<'a>>,
    /// Imports where the receiver type was written, also needed for external types.
    pub context: &'a FileSummary,
}

/// Class-like declarations a type name written in the file `context` means.
pub fn resolve_type<'a>(index: &'a Index, context: &'a FileSummary, name: &str) -> Vec<DeclRef<'a>> {
    resolve_type_in(index, context, name, None)
}

/// Resolves a type in a declaration's lexical scope; unrelated nested types are not visible.
pub(crate) fn resolve_type_in<'a>(
    index: &'a Index,
    context: &'a FileSummary,
    name: &str,
    scope: Option<&'a Decl>,
) -> Vec<DeclRef<'a>> {
    let Some((first, rest)) = name.split_once('.') else { return simple_type(index, context, name, scope) };
    let exact = class_like(by_fqn(index, name));
    if !exact.is_empty() {
        return exact;
    }
    // `Outer.Inner`: the outer type by its simple name, then down its nested types.
    let mut found = simple_type(index, context, first, scope);
    for segment in rest.split('.') {
        found = found.iter().flat_map(|&(f, d)| children(f, d).filter(move |c| c.name == segment && c.kind.is_class_like()).map(move |c| (f, c))).collect();
    }
    // A qualified type missing from the project must not become an unrelated
    // project class merely because its last segment has the same name.
    found
}

fn simple_type<'a>(
    index: &'a Index,
    context: &'a FileSummary,
    name: &str,
    scope: Option<&'a Decl>,
) -> Vec<DeclRef<'a>> {
    for owner in std::iter::successors(scope, |d| container(context, d)) {
        let nested = class_like(own_members(context, owner, name, false));
        if !nested.is_empty() {
            return nested;
        }
    }
    let key = |path: &'a str, alias: &'a Option<String>| alias.as_deref().unwrap_or_else(|| path.rsplit('.').next().unwrap_or(path));
    if let Some(import) = context.imports.iter().find(|i| !i.wildcard && key(&i.path, &i.alias) == name) {
        return class_like(by_fqn(index, &import.path));
    }
    let own: Vec<DeclRef> = context.decls.iter().filter(|d| d.container.is_none() && d.name == name && d.kind.is_class_like()).map(|d| (context, d)).collect();
    if !own.is_empty() {
        return own;
    }
    let candidates = class_like(index.lookup(name));
    let levels: [&dyn Fn(&FileSummary, &Decl) -> bool; 2] = [
        &|f, d| d.container.is_none() && f.package == context.package,
        &|_, d| context.imports.iter().any(|i| i.wildcard && i.path == d.parent_fqn()),
    ];
    levels
        .iter()
        .map(|keep| candidates.iter().copied().filter(|(f, d)| keep(f, d)).collect::<Vec<_>>())
        .find(|selected| !selected.is_empty())
        .unwrap_or_else(|| candidates.into_iter().filter(|(_, d)| d.container.is_none()).collect())
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
    let mut receivers: HashSet<String> = super_names;
    if ty.decls.is_empty() {
        receivers.extend(external_type_names(ty.context, &ty.name));
    } else {
        receivers.extend(ty.decls.iter().map(|(_, d)| d.fqn.clone()));
    }
    index.lookup(name).into_iter()
        .filter(|(f, d)| {
            d.receiver.as_deref().is_some_and(|r| {
                type_names(index, f, r, Some(d)).iter().any(|name| receivers.contains(name))
            })
        })
        .collect()
}

/// Canonical project names, or the imported/written name of an external type.
fn type_names(index: &Index, file: &FileSummary, name: &str, scope: Option<&Decl>) -> HashSet<String> {
    let resolved = resolve_type_in(index, file, name, scope);
    if !resolved.is_empty() {
        return resolved.iter().map(|(_, d)| d.fqn.clone()).collect();
    }
    external_type_names(file, name)
}

/// Possible identities of an unindexed type. Explicit imports and qualified names
/// are definitive; wildcard and default imports are candidates until dependencies are indexed.
fn external_type_names(file: &FileSummary, name: &str) -> HashSet<String> {
    let (first, nested) = name.split_once('.').map_or((name, None), |(first, rest)| (first, Some(rest)));
    if let Some(import) = file.imports.iter().find(|i| {
        !i.wildcard && i.alias.as_deref().unwrap_or_else(|| i.path.rsplit('.').next().unwrap_or(&i.path)) == first
    }) {
        return HashSet::from([nested.map_or_else(|| import.path.clone(), |rest| format!("{}.{rest}", import.path))]);
    }
    if nested.is_some() {
        return HashSet::from([name.to_string()]);
    }
    let mut names = HashSet::from([name.to_string()]);
    if !file.package.is_empty() {
        names.insert(format!("{}.{name}", file.package));
    }
    names.extend(file.imports.iter().filter(|i| i.wildcard).map(|i| format!("{}.{name}", i.path)));
    names.insert(format!("java.lang.{name}"));
    if file.lang == Lang::Kotlin {
        for package in [
            "kotlin", "kotlin.annotation", "kotlin.collections", "kotlin.comparisons", "kotlin.io",
            "kotlin.ranges", "kotlin.sequences", "kotlin.text", "kotlin.jvm",
        ] {
            names.insert(format!("{package}.{name}"));
        }
    }
    names
}

/// Members named `name` on the nearest supertype level that has any (breadth-first,
/// `MAX_DEPTH` levels, each type once), and the possible identities of every supertype met.
pub fn supertype_members<'a>(index: &'a Index, ty: &Ty<'a>, name: &str) -> (Vec<DeclRef<'a>>, HashSet<String>) {
    let mut names = HashSet::new();
    let mut seen: HashSet<(&Path, usize)> = ty.decls.iter().filter_map(|&(f, d)| Some((f.path.as_path(), position(f, d)?))).collect();
    let mut level: Vec<DeclRef> = ty.decls.clone();
    for _ in 0..MAX_DEPTH {
        let mut next = Vec::new();
        for (f, d) in level {
            for written in &d.supertypes {
                let resolved = resolve_type_in(index, f, written, Some(d));
                if resolved.is_empty() {
                    names.extend(external_type_names(f, written));
                } else {
                    names.extend(resolved.iter().map(|(_, d)| d.fqn.clone()));
                }
                for (sf, sd) in resolved {
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

/// Whether a supertype written up the chain of `ty` (`MAX_DEPTH` levels, each type once) is
/// not in the project, like `AppCompatActivity`: its members are unknown.
pub fn leaves_project<'a>(index: &'a Index, ty: &Ty<'a>) -> bool {
    let mut seen: HashSet<(&Path, usize)> = ty.decls.iter().filter_map(|&(f, d)| Some((f.path.as_path(), position(f, d)?))).collect();
    let mut level: Vec<DeclRef> = ty.decls.clone();
    for _ in 0..MAX_DEPTH {
        let mut next = Vec::new();
        for (f, d) in level {
            for written in &d.supertypes {
                let found = resolve_type_in(index, f, written, Some(d));
                if found.is_empty() {
                    return true;
                }
                next.extend(found.into_iter().filter(|&(sf, sd)| position(sf, sd).is_some_and(|i| seen.insert((sf.path.as_path(), i)))));
            }
        }
        if next.is_empty() {
            return false;
        }
        level = next;
    }
    false
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
