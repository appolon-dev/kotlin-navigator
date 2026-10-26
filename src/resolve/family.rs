//! Declarations related to one another: subtypes, overrides, expect/actual pairs.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

use crate::cancel::{Cancel, Cancelled};
use crate::index::Index;
use crate::model::{Decl, FileSummary, Flags};
use crate::resolve::members::{Ty, container, own_members, resolve_type, supertype_members};
use crate::resolve::rank::DeclRef;
use crate::resolve::{Current, Found, Target, by_fqn, target_at};
use crate::sources::Sources;

/// Declarations are compared by place, not by pointer: summaries of open buffers are copies.
type Key = (PathBuf, u32, u32, u32);

fn key((file, decl): DeclRef) -> Key {
    let r = decl.name_range;
    (file.path.clone(), r.line, r.start, r.end)
}

/// Sorted by (path, line, start), each declaration once.
fn ordered(mut list: Vec<DeclRef>) -> Vec<DeclRef> {
    list.sort_by(|(fa, da), (fb, db)| (&fa.path, da.name_range.line, da.name_range.start).cmp(&(&fb.path, db.name_range.line, db.name_range.start)));
    list.dedup_by_key(|&mut r| key(r));
    list
}

/// Subtype search over one index: the map of heirs is built once, subtypes once per class.
struct Hierarchy<'a> {
    index: &'a Index,
    /// Last segment of a written supertype -> the classes that name it.
    heirs: HashMap<&'a str, Vec<DeclRef<'a>>>,
    cache: HashMap<Key, Vec<DeclRef<'a>>>,
}

impl<'a> Hierarchy<'a> {
    fn new(index: &'a Index) -> Self {
        let mut heirs: HashMap<&str, Vec<DeclRef<'a>>> = HashMap::new();
        for (file, decl) in index.decls().filter(|(_, d)| d.kind.is_class_like()) {
            for written in &decl.supertypes {
                heirs.entry(written.rsplit('.').next().unwrap_or(written)).or_default().push((file, decl));
            }
        }
        Hierarchy { index, heirs, cache: HashMap::new() }
    }

    fn subtypes(&mut self, class: DeclRef<'a>, cancel: &Cancel) -> Result<Vec<DeclRef<'a>>, Cancelled> {
        if let Some(done) = self.cache.get(&key(class)) {
            return Ok(done.clone());
        }
        let mut seen: HashSet<Key> = HashSet::from([key(class)]);
        let mut queue = VecDeque::from([class]);
        let mut found = Vec::new();
        while let Some(current) = queue.pop_front() {
            cancel.check()?;
            let (current_file, current_decl) = current;
            for &(file, decl) in self.heirs.get(current_decl.name.as_str()).into_iter().flatten() {
                let names_current = decl.supertypes.iter().filter(|w| w.rsplit('.').next() == Some(current_decl.name.as_str())).any(|w| {
                    resolve_type(self.index, file, w).iter().any(|&r| key(r) == key((current_file, current_decl)))
                });
                if names_current && seen.insert(key((file, decl))) {
                    found.push((file, decl));
                    queue.push_back((file, decl));
                }
            }
        }
        let found = ordered(found);
        self.cache.insert(key(class), found.clone());
        Ok(found)
    }

    /// Overrides named `name` in the subtypes of `class`.
    fn overrides(&mut self, class: DeclRef<'a>, name: &str, cancel: &Cancel) -> Result<Vec<DeclRef<'a>>, Cancelled> {
        Ok(self
            .subtypes(class, cancel)?
            .into_iter()
            .flat_map(|(f, c)| own_members(f, c, name, false))
            .filter(|(_, d)| d.flags.contains(Flags::OVERRIDE))
            .collect())
    }
}

/// Direct and indirect subtypes of `class`, without `class` itself.
pub fn subtypes<'a>(index: &'a Index, class: DeclRef<'a>, cancel: &Cancel) -> Result<Vec<DeclRef<'a>>, Cancelled> {
    Hierarchy::new(index).subtypes(class, cancel)
}

/// An `expect` declaration, or a member of an `expect` class: those carry no flag of their own.
fn is_expect(file: &FileSummary, decl: &Decl) -> bool {
    std::iter::successors(Some(decl), |d| container(file, d)).any(|d| d.flags.contains(Flags::EXPECT))
}

/// One side of an expect/actual pair. Members of an `actual` class are marked `actual` themselves.
fn is_pair((file, decl): DeclRef) -> bool {
    is_expect(file, decl) || decl.flags.contains(Flags::ACTUAL)
}

/// The class of a member, when it is a member of a class-like declaration.
fn owner<'a>(file: &'a FileSummary, decl: &Decl) -> Option<DeclRef<'a>> {
    container(file, decl).filter(|c| c.kind.is_class_like()).map(|c| (file, c))
}

/// `decl` with everything it overrides, everything overriding it, and its expect/actual
/// counterparts.
pub fn family<'a>(index: &'a Index, decl: DeclRef<'a>, cancel: &Cancel) -> Result<Vec<DeclRef<'a>>, Cancelled> {
    let mut hierarchy = Hierarchy::new(index);
    let mut seen: HashSet<Key> = HashSet::from([key(decl)]);
    let mut work = vec![decl];
    let mut all = vec![decl];
    while let Some((file, d)) = work.pop() {
        cancel.check()?;
        let mut related: Vec<DeclRef> = Vec::new();
        if let Some(class) = owner(file, d) {
            if d.flags.contains(Flags::OVERRIDE) {
                let ty = Ty { name: class.1.name.clone(), decls: vec![class] };
                related.extend(supertype_members(index, &ty, &d.name).0);
            }
            related.extend(hierarchy.overrides(class, &d.name, cancel)?);
        }
        if is_pair((file, d)) {
            related.extend(by_fqn(index, &d.fqn).into_iter().filter(|&o| is_pair(o)));
        }
        for r in related {
            if seen.insert(key(r)) {
                work.push(r);
                all.push(r);
            }
        }
    }
    Ok(ordered(all))
}

/// What implements or overrides `decl`: subtypes of a class, overrides of a member, the
/// actuals of an `expect`.
pub fn implementations<'a>(index: &'a Index, decl: DeclRef<'a>, cancel: &Cancel) -> Result<Vec<DeclRef<'a>>, Cancelled> {
    let (file, d) = decl;
    let mut hierarchy = Hierarchy::new(index);
    let mut found = if d.kind.is_class_like() {
        hierarchy.subtypes(decl, cancel)?
    } else if let Some(class) = owner(file, d) {
        hierarchy.overrides(class, &d.name, cancel)?
    } else {
        Vec::new()
    };
    if is_expect(file, d) {
        found.extend(by_fqn(index, &d.fqn).into_iter().filter(|(_, o)| o.flags.contains(Flags::ACTUAL)));
    }
    Ok(ordered(found))
}

/// Implementations of what the identifier at `byte` declares or refers to.
pub fn implementation<'a>(index: &'a Index, current: &Current<'a>, sources: &Sources, byte: usize, cancel: &Cancel) -> Result<Vec<Target>, Cancelled> {
    let Some(resolution) = target_at(index, current, sources, byte) else { return Ok(Vec::new()) };
    let mut found = Vec::new();
    for item in &resolution.found {
        if let Found::Decl(decl) = item {
            found.extend(implementations(index, *decl, cancel)?);
        }
    }
    Ok(ordered(found).into_iter().map(|(file, decl)| Target { path: file.path.clone(), range: decl.name_range }).collect())
}
