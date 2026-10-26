//! What is under the cursor, which declarations it may refer to, and how sure that is.

pub mod args;
pub mod context;
pub mod family;
pub mod members;
pub mod rank;
pub mod scope;
pub mod typedef;

use std::path::PathBuf;

use tree_sitter::Tree;

use self::context::{Receiver, Reference, Role, reference_at};
use self::members::{Ty, leaves_project, members_named, resolve_type, supertype_members};
use self::rank::{DeclRef, prefer, rank, rank_with_level, tier, visible};
use self::scope::{Implicit, is_property, Local, LocalKind, enclosing_declarations, find_local, implicit_receivers};
use crate::docs::OpenDoc;
use crate::index::Index;
use crate::model::{DeclKind, FileSummary, Flags, Import, Lang, Span};
use crate::parse::{span_at, span_of, type_child};
use crate::sources::Sources;
use Certainty::{Exact, Probable};

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

impl<'a> Current<'a> {
    /// The buffer `doc` with the index's summary of its file when there is one: that one
    /// knows the Gradle module.
    pub fn of(index: &'a Index, doc: &'a OpenDoc) -> Current<'a> {
        let summary = index.file(&doc.summary.path).unwrap_or(&doc.summary);
        Current { text: &doc.text, tree: &doc.tree, summary }
    }
}

/// How the resolver reached its targets: `Exact` by a reliable path, `Probable` by a
/// search by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Certainty {
    Exact,
    Probable,
}

/// A declaration of the current file the index does not hold: a parameter or a local.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalDecl {
    pub path: PathBuf,
    pub range: Span,
    pub type_name: Option<String>,
}

/// A declaration a reference may refer to.
#[derive(Clone, Debug)]
pub enum Found<'a> {
    Decl(DeclRef<'a>),
    Local(LocalDecl),
}

impl Found<'_> {
    pub fn target(&self) -> Target {
        match self {
            Found::Decl((file, decl)) => Target { path: file.path.clone(), range: decl.name_range },
            Found::Local(local) => Target { path: local.path.clone(), range: local.range },
        }
    }
}

/// The reference under the cursor, the declarations it may refer to (best first), and
/// how sure that is.
pub struct Resolution<'a> {
    pub reference: Reference,
    pub found: Vec<Found<'a>>,
    pub certainty: Certainty,
}

/// Declarations the identifier at `byte` may refer to, best first.
pub fn definition(index: &Index, current: &Current, sources: &Sources, byte: usize) -> Vec<Target> {
    resolve(index, current, sources, byte).map(|r| r.found.iter().map(Found::target).collect()).unwrap_or_default()
}

/// What the identifier at `byte` refers to; `None` only when there is no name there.
pub fn resolve<'a>(index: &'a Index, current: &Current<'a>, sources: &Sources, byte: usize) -> Option<Resolution<'a>> {
    let reference = reference_at(current.tree, current.text, byte)?;
    let resolver = Resolver {
        index,
        text: current.text,
        tree: current.tree,
        summary: current.summary,
        name: &reference.name,
        role: &reference.role,
        at: reference.range.start,
        qualified: matches!(reference.receiver, Receiver::Path(_) | Receiver::Unknown),
    };
    let (found, certainty) = if let Role::Import(fqn) = &reference.role {
        (decls(by_fqn(index, fqn)), Exact)
    } else if matches!(reference.role, Role::Argument) {
        return Some(label(index, current, sources, reference));
    } else if current.summary.lang == Lang::Java {
        resolver.java()
    } else {
        match &reference.receiver {
            Receiver::None => resolver.simple_name(),
            Receiver::This => match implicit_receivers(current.tree, current.text, resolver.at).first() {
                Some(receiver) => resolver.on_implicit(receiver),
                None => resolver.by_name(),
            },
            Receiver::Super => resolver.on_super(),
            Receiver::Path(path) => resolver.on_path(path),
            Receiver::Unknown => resolver.by_name(),
        }
    };
    Some(Resolution { reference, found, certainty })
}

/// Like `resolve`, but on the name of a declaration the target is that declaration alone:
/// not, say, the actuals of an `expect`. That is exact, even where a search by name found it,
/// as in Java.
pub fn target_at<'a>(index: &'a Index, current: &Current<'a>, sources: &Sources, byte: usize) -> Option<Resolution<'a>> {
    let mut resolution = resolve(index, current, sources, byte)?;
    let span = span_at(current.text, resolution.reference.range.clone());
    let here = resolution.found.iter().position(|f| {
        let target = f.target();
        target.path == current.summary.path && target.range == span
    });
    if let Some(i) = here {
        let own = resolution.found.swap_remove(i);
        resolution.found = vec![own];
        resolution.certainty = Exact;
    }
    Some(resolution)
}

/// A named-argument label: the parameter of the callee, found in the file that declares it.
/// A `val`/`var` parameter of a class is its property; any other is a local declaration.
fn label<'a>(index: &'a Index, current: &Current<'a>, sources: &Sources, reference: Reference) -> Resolution<'a> {
    let start = reference.range.start;
    let callee = current.tree.root_node().named_descendant_for_byte_range(start, start).and_then(args::label_callee);
    let Some(callee) = callee else { return Resolution { reference, found: Vec::new(), certainty: Probable } };
    let Some(called) = resolve(index, current, sources, callee.start_byte()) else {
        return Resolution { reference, found: Vec::new(), certainty: Probable };
    };
    let mut found: Vec<Found<'a>> = Vec::new();
    for target in called.found.iter().map(Found::target) {
        let doc;
        let (text, tree) = if target.path == current.summary.path {
            (current.text, current.tree)
        } else {
            let Some(loaded) = sources.load(&target.path) else { continue };
            doc = loaded;
            (doc.text.as_str(), &doc.tree)
        };
        let Some(param) = args::parameter(tree, text, target.range, &reference.name) else { continue };
        let range = span_of(text, param);
        let property = param.parent().filter(|p| p.kind() == "class_parameter" && is_property(*p));
        let summary = if target.path == current.summary.path { Some(current.summary) } else { index.file(&target.path) };
        let decl = property.and(summary).and_then(|f| f.decls.iter().find(|d| d.name_range == range).map(|d| (f, d)));
        let item = match decl {
            Some(decl) => Found::Decl(decl),
            None => {
                let type_name = param.parent().and_then(|p| type_child(text, p));
                Found::Local(LocalDecl { path: target.path.clone(), range, type_name })
            }
        };
        if !found.iter().any(|f| f.target() == item.target()) {
            found.push(item);
        }
    }
    Resolution { reference, found, certainty: called.certainty }
}

/// A variable a receiver path names, and where its type is written.
struct Variable<'a> {
    type_name: Option<String>,
    from_call: bool,
    file: &'a FileSummary,
}

struct Resolver<'r, 'a> {
    index: &'a Index,
    text: &'a str,
    tree: &'a Tree,
    summary: &'a FileSummary,
    name: &'r str,
    role: &'r Role,
    /// Start of the identifier.
    at: usize,
    /// The name follows a receiver: `x.name`.
    qualified: bool,
}

impl<'a> Resolver<'_, 'a> {
    /// Locals, then members of implicit receivers, then imports and top-level declarations.
    /// A top-level declaration is exact when a ranking level (import, file, package) chose it.
    fn simple_name(&self) -> (Vec<Found<'a>>, Certainty) {
        let locals = find_local(self.tree, self.text, self.at, self.name);
        if let Some(local) = self.pick_local(&locals) {
            let range = span_at(self.text, local.range.clone());
            let local = LocalDecl { path: self.summary.path.clone(), range, type_name: local.type_name.clone() };
            return (vec![Found::Local(local)], Exact);
        }
        for receiver in implicit_receivers(self.tree, self.text, self.at) {
            let Some(ty) = self.ty_of(&receiver) else { continue };
            let (found, _) = self.pick(members_named(self.index, &ty, self.name, true));
            if !found.is_empty() {
                return (decls(found), Exact);
            }
        }
        if let Some(import) = self.explicit_import() {
            return (decls(self.rank(by_fqn(self.index, &import.path))), Exact);
        }
        let top: Vec<DeclRef> =
            self.index.lookup(self.name).into_iter().filter(|(_, d)| d.container.is_none() || d.kind.is_class_like()).collect();
        let (found, level) = self.pick(top);
        if !found.is_empty() {
            return (decls(found), if level { Exact } else { Probable });
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
    fn on_implicit(&self, receiver: &Implicit) -> (Vec<Found<'a>>, Certainty) {
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

    /// `super.name`: members of the supertypes of the class around the cursor.
    fn on_super(&self) -> (Vec<Found<'a>>, Certainty) {
        let receivers = implicit_receivers(self.tree, self.text, self.at);
        let container = receivers.iter().filter(|r| matches!(r, Implicit::Container(_))).find_map(|r| self.ty_of(r));
        let Some(ty) = container else { return (Vec::new(), Probable) };
        let (inherited, _) = supertype_members(self.index, &ty, self.name);
        (decls(self.pick(inherited).0), Exact)
    }

    /// `p.name`: `p` is a variable (its type's members) or a type (its and its companion's).
    fn on_path(&self, path: &str) -> (Vec<Found<'a>>, Certainty) {
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

    /// Members of `ty`; nothing when the type is known but lacks them (probably nothing when
    /// its supertypes leave the project), a name search when the type is not in the project
    /// and no extension matches.
    fn members(&self, ty: &Ty<'a>, companion: bool) -> (Vec<Found<'a>>, Certainty) {
        let (found, _) = self.pick(members_named(self.index, ty, self.name, companion));
        if found.is_empty() && ty.decls.is_empty() {
            return self.by_name();
        }
        let certainty = if found.is_empty() && leaves_project(self.index, ty) { Probable } else { Exact };
        (decls(found), certainty)
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

    /// Java is read without types, so only its import lines are exact.
    fn java(&self) -> (Vec<Found<'a>>, Certainty) {
        match self.explicit_import() {
            Some(import) => (decls(self.rank(by_fqn(self.index, &import.path))), Probable),
            None => self.by_name(),
        }
    }

    /// Every declaration with the name, ranked. After a receiver whose type is unknown, the
    /// function or property the cursor is in is left out: there it is the caller, as in
    /// `fun clearAll() = queries.clearAll()`.
    fn by_name(&self) -> (Vec<Found<'a>>, Certainty) {
        let mut candidates = self.index.lookup(self.name);
        if self.qualified {
            let around = enclosing_declarations(self.tree, self.text, self.at, self.name);
            candidates.retain(|(f, d)| f.path != self.summary.path || !around.contains(&d.name_range));
        }
        (decls(self.pick(candidates).0), Probable)
    }

    fn explicit_import(&self) -> Option<&'a Import> {
        self.summary.imports.iter().find(|i| !i.wildcard && import_key(i) == self.name)
    }

    fn rank(&self, candidates: Vec<DeclRef<'a>>) -> Vec<DeclRef<'a>> {
        rank(self.index, self.summary, candidates)
    }

    /// The visible candidates of the best kind tier for the role, ranked, and whether a
    /// ranking level chose them. Visibility comes first: another file's `private fun Chip`
    /// must not hide `class Chip`.
    fn pick(&self, candidates: Vec<DeclRef<'a>>) -> (Vec<DeclRef<'a>>, bool) {
        rank_with_level(self.index, self.summary, prefer(self.role, visible(self.summary, candidates)))
    }
}

fn decls(found: Vec<DeclRef>) -> Vec<Found> {
    found.into_iter().map(Found::Decl).collect()
}

pub(crate) fn by_fqn<'a>(index: &'a Index, fqn: &str) -> Vec<DeclRef<'a>> {
    let name = fqn.rsplit('.').next().unwrap_or(fqn);
    index.lookup(name).into_iter().filter(|(_, d)| d.fqn == fqn).collect()
}

fn import_key(import: &Import) -> &str {
    import.alias.as_deref().unwrap_or_else(|| import.path.rsplit('.').next().unwrap_or(&import.path))
}
