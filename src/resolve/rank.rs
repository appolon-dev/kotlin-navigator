//! Which candidates fit the reference's role, and in which order they are offered.

use crate::index::Index;
use crate::model::{Decl, DeclKind, FileSummary, Flags};
use crate::resolve::MAX_CANDIDATES;
use crate::resolve::context::Role;
use crate::source_set::sees;

pub type DeclRef<'a> = (&'a FileSummary, &'a Decl);

/// Candidates of the best kind tier for the role; all of them if none fits.
pub fn prefer<'a>(role: &Role, candidates: Vec<DeclRef<'a>>) -> Vec<DeclRef<'a>> {
    let class_like = |k: DeclKind| k.is_class_like();
    let function = |k: DeclKind| k == DeclKind::Function;
    let tiers: Vec<&dyn Fn(DeclKind) -> bool> = match role {
        Role::Type => vec![&class_like],
        Role::Call => vec![&function, &class_like],
        Role::CallableRef => vec![&|k| matches!(k, DeclKind::Function | DeclKind::Property) || k.is_class_like()],
        Role::Expr => vec![&|k| matches!(k, DeclKind::Property | DeclKind::EnumEntry) || k.is_class_like()],
        Role::Import(_) => vec![],
    };
    for fits in tiers {
        let selected: Vec<_> = candidates.iter().copied().filter(|(_, d)| fits(d.kind)).collect();
        if !selected.is_empty() {
            return selected;
        }
    }
    candidates
}

/// Visibility, stage-1 levels, source set, module order and expect/actual, capped at `MAX_CANDIDATES`.
pub fn rank<'a>(index: &'a Index, current: &FileSummary, candidates: Vec<DeclRef<'a>>) -> Vec<DeclRef<'a>> {
    let mut list: Vec<DeclRef> =
        candidates.into_iter().filter(|(f, d)| !d.flags.contains(Flags::PRIVATE) || f.path == current.path).collect();

    let levels: [&dyn Fn(&FileSummary, &Decl) -> bool; 4] = [
        &|_, d| current.imports.iter().any(|i| !i.wildcard && i.path == d.fqn),
        &|f, _| f.path == current.path,
        &|f, _| f.package == current.package,
        &|_, d| current.imports.iter().any(|i| i.wildcard && i.path == d.parent_fqn()),
    ];
    if let Some(selected) = levels.iter().map(|keep| keep_only(&list, |f, d| keep(f, d))).find(|s| !s.is_empty()) {
        list = selected;
    }

    let compatible = keep_only(&list, |f, _| sees(current.source_set.as_deref(), f.source_set.as_deref()));
    if !compatible.is_empty() {
        list = compatible;
    }

    let group = |f: &FileSummary, d: &Decl| match (&current.module, &f.module) {
        (Some(a), Some(b)) if a == b => 0,
        (Some(_), Some(_)) if d.flags.contains(Flags::INTERNAL) => 2,
        _ => 1,
    };
    list.sort_by_key(|(f, d)| (group(f, d), &f.path, d.name_range.line));

    if list.iter().any(|(_, d)| d.flags.contains(Flags::EXPECT)) {
        list = with_actuals(index, current, list);
    }
    list.truncate(MAX_CANDIDATES);
    list
}

fn keep_only<'a>(list: &[DeclRef<'a>], keep: impl Fn(&FileSummary, &Decl) -> bool) -> Vec<DeclRef<'a>> {
    list.iter().copied().filter(|(f, d)| keep(f, d)).collect()
}

/// `actual`s seen from the current source set, then `list`, then the other `actual`s.
fn with_actuals<'a>(index: &'a Index, current: &FileSummary, list: Vec<DeclRef<'a>>) -> Vec<DeclRef<'a>> {
    let mut actuals: Vec<DeclRef> = Vec::new();
    for (_, expect) in list.iter().filter(|(_, d)| d.flags.contains(Flags::EXPECT)) {
        for candidate in index.lookup(&expect.name) {
            if candidate.1.fqn == expect.fqn && candidate.1.flags.contains(Flags::ACTUAL) && !contains(&actuals, candidate) {
                actuals.push(candidate);
            }
        }
    }
    let (seen, others): (Vec<_>, Vec<_>) =
        actuals.into_iter().partition(|(f, _)| sees(current.source_set.as_deref(), f.source_set.as_deref()));
    let mut out = seen;
    for item in list.into_iter().chain(others) {
        if !contains(&out, item) {
            out.push(item);
        }
    }
    out
}

fn contains(list: &[DeclRef], (_, decl): DeclRef) -> bool {
    list.iter().any(|(_, d)| std::ptr::eq(*d, decl))
}
