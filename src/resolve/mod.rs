//! What is under the cursor, and which declarations it may refer to.

pub mod context;
pub mod members;
pub mod rank;
pub mod scope;

use std::path::PathBuf;

use tree_sitter::Tree;

use self::context::{Receiver, Role, reference_at};
use self::members::{Ty, members_named, resolve_type, supertype_members};
use self::rank::{DeclRef, prefer, rank, tier};
use self::scope::{Implicit, Local, LocalKind, find_local, implicit_receivers};
use crate::index::Index;
use crate::model::{DeclKind, FileSummary, Flags, Import, Lang, Span};
use crate::parse::span_at;

pub const MAX_CANDIDATES: usize = 50;

/// A place to jump to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub path: PathBuf,
    pub range: Span,
}

/// The file the cursor is in, as currently edited.
pub struct Current<'a> {
    pub text: &'a str,
    pub tree: &'a Tree,
    pub summary: &'a FileSummary,
}

/// Declarations the identifier at `byte` may refer to, best first.
pub fn definition(index: &Index, current: &Current, byte: usize) -> Vec<Target> {
    let Some(reference) = reference_at(current.tree, current.text, byte) else { return Vec::new() };
    let resolver = Resolver {
        index,
        text: current.text,
        tree: current.tree,
        summary: current.summary,
        name: &reference.name,
        role: &reference.role,
        at: reference.range.start,
    };
    if let Role::Import(fqn) = &reference.role {
        return targets(by_fqn(index, fqn));
    }
    if current.summary.lang == Lang::Java {
        return resolver.java();
    }
    match &reference.receiver {
        Receiver::None => resolver.simple_name(),
        Receiver::This => match implicit_receivers(current.tree, current.text, resolver.at).first() {
            Some(receiver) => resolver.on_implicit(receiver),
            None => resolver.by_name(),
        },
        Receiver::Super => {
            let receivers = implicit_receivers(current.tree, current.text, resolver.at);
            let container = receivers.iter().filter(|r| matches!(r, Implicit::Container(_))).find_map(|r| resolver.ty_of(r));
            let Some(ty) = container else { return Vec::new() };
            let (inherited, _) = supertype_members(index, &ty, &reference.name);
            targets(rank(index, current.summary, prefer(&reference.role, inherited)))
        }
        Receiver::Path(path) => resolver.on_path(path),
        Receiver::Unknown => resolver.by_name(),
    }
}

/// A variable a receiver path names, and where its type is written.
struct Variable<'a> {
    type_name: Option<String>,
    from_call: bool,
    file: &'a FileSummary,
}

struct Resolver<'a> {
    index: &'a Index,
    text: &'a str,
    tree: &'a Tree,
    summary: &'a FileSummary,
    name: &'a str,
    role: &'a Role,
    /// Start of the identifier.
    at: usize,
}

impl<'a> Resolver<'a> {
    /// Locals, then members of implicit receivers, then imports and top-level declarations.
    fn simple_name(&self) -> Vec<Target> {
        let locals = find_local(self.tree, self.text, self.at, self.name);
        if let Some(local) = self.pick_local(&locals) {
            return vec![Target { path: self.summary.path.clone(), range: span_at(self.text, local.range.clone()) }];
        }
        for receiver in implicit_receivers(self.tree, self.text, self.at) {
            let Some(ty) = self.ty_of(&receiver) else { continue };
            let found = self.rank(prefer(self.role, members_named(self.index, &ty, self.name, true)));
            if !found.is_empty() {
                return targets(found);
            }
        }
        if let Some(import) = self.explicit_import() {
            return targets(self.rank(by_fqn(self.index, &import.path)));
        }
        let top: Vec<DeclRef> =
            self.index.lookup(self.name).into_iter().filter(|(_, d)| d.container.is_none() || d.kind.is_class_like()).collect();
        let found = self.rank(prefer(self.role, top));
        if !found.is_empty() {
            return targets(found);
        }
        self.by_name()
    }

    /// The nearest local of the best kind tier for the role, else the nearest one.
    fn pick_local<'l>(&self, locals: &'l [Local]) -> Option<&'l Local> {
        let tier_of = |l: &Local| {
            let kind = match l.kind {
                LocalKind::Value => DeclKind::Property,
                LocalKind::Function => DeclKind::Function,
                LocalKind::Class => DeclKind::Class,
            };
            tier(self.role, kind)
        };
        match locals.iter().filter_map(tier_of).min() {
            Some(best) => locals.iter().find(|l| tier_of(l) == Some(best)),
            None => locals.first(),
        }
    }

    /// `this.name` and the members of the receiver around the cursor.
    fn on_implicit(&self, receiver: &Implicit) -> Vec<Target> {
        match self.ty_of(receiver) {
            Some(ty) => self.members(&ty, true),
            None => self.by_name(),
        }
    }

    fn ty_of(&self, receiver: &Implicit) -> Option<Ty<'a>> {
        match receiver {
            Implicit::Extension(name) => {
                Some(Ty { name: name.clone(), decls: resolve_type(self.index, self.summary, name) })
            }
            Implicit::Container(range) => {
                let span = span_at(self.text, range.clone());
                let decl = self.summary.decls.iter().find(|d| d.name_range == span)?;
                Some(Ty { name: decl.name.clone(), decls: vec![(self.summary, decl)] })
            }
        }
    }

    /// `p.name`: `p` is a variable (its type's members) or a type (its and its companion's).
    fn on_path(&self, path: &str) -> Vec<Target> {
        if let Some(variable) = self.variable(path).filter(|_| !path.contains('.')) {
            let Some(type_name) = variable.type_name else { return self.by_name() };
            let decls = resolve_type(self.index, variable.file, &type_name);
            if variable.from_call && decls.is_empty() {
                return self.by_name();
            }
            return self.members(&Ty { name: type_name, decls }, false);
        }
        let decls = resolve_type(self.index, self.summary, path);
        if decls.is_empty() {
            return self.by_name();
        }
        self.members(&Ty { name: path.to_string(), decls }, true)
    }

    /// Members of `ty`; nothing when the type is known but lacks them, a name search when
    /// the type is not in the project and no extension matches.
    fn members(&self, ty: &Ty<'a>, companion: bool) -> Vec<Target> {
        let found = self.rank(prefer(self.role, members_named(self.index, ty, self.name, companion)));
        if found.is_empty() && ty.decls.is_empty() {
            return self.by_name();
        }
        targets(found)
    }

    /// The variable `name` seen from the cursor: a local, a property of an implicit
    /// receiver, or a top-level property.
    fn variable(&self, name: &str) -> Option<Variable<'a>> {
        let locals = find_local(self.tree, self.text, self.at, name);
        if let Some(local) = locals.into_iter().find(|l| l.kind == LocalKind::Value) {
            return Some(Variable { type_name: local.type_name, from_call: local.from_call, file: self.summary });
        }
        let property = |(f, d): DeclRef<'a>| Variable {
            type_name: d.type_hint.clone(),
            from_call: d.flags.contains(Flags::CALL_HINT),
            file: f,
        };
        for receiver in implicit_receivers(self.tree, self.text, self.at) {
            let Some(ty) = self.ty_of(&receiver) else { continue };
            let found = members_named(self.index, &ty, name, true).into_iter().find(|(_, d)| d.kind == DeclKind::Property);
            if let Some(found) = found {
                return Some(property(found));
            }
        }
        let top = self.index.lookup(name).into_iter().filter(|(_, d)| d.kind == DeclKind::Property && d.container.is_none());
        self.rank(top.collect()).into_iter().next().map(property)
    }

    fn java(&self) -> Vec<Target> {
        match self.explicit_import() {
            Some(import) => targets(self.rank(by_fqn(self.index, &import.path))),
            None => self.by_name(),
        }
    }

    /// Every declaration with the name, ranked.
    fn by_name(&self) -> Vec<Target> {
        targets(self.rank(prefer(self.role, self.index.lookup(self.name))))
    }

    fn explicit_import(&self) -> Option<&'a Import> {
        self.summary.imports.iter().find(|i| !i.wildcard && import_key(i) == self.name)
    }

    fn rank(&self, candidates: Vec<DeclRef<'a>>) -> Vec<DeclRef<'a>> {
        rank(self.index, self.summary, candidates)
    }
}

fn targets(found: Vec<DeclRef>) -> Vec<Target> {
    found.into_iter().map(|(f, d)| Target { path: f.path.clone(), range: d.name_range }).collect()
}

pub(crate) fn by_fqn<'a>(index: &'a Index, fqn: &str) -> Vec<DeclRef<'a>> {
    let name = fqn.rsplit('.').next().unwrap_or(fqn);
    index.lookup(name).into_iter().filter(|(_, d)| d.fqn == fqn).collect()
}

fn import_key(import: &Import) -> &str {
    import.alias.as_deref().unwrap_or_else(|| import.path.rsplit('.').next().unwrap_or(&import.path))
}
