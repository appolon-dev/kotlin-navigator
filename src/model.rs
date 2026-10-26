//! Compact per-file summary that survives after the tree-sitter tree is dropped.

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Kotlin,
    Java,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeclKind {
    Class,
    Interface,
    Object,
    Enum,
    EnumEntry,
    TypeAlias,
    Function,
    Property,
}

impl DeclKind {
    pub fn is_class_like(self) -> bool {
        matches!(self, DeclKind::Class | DeclKind::Interface | DeclKind::Object | DeclKind::Enum | DeclKind::TypeAlias)
    }
}

/// Modifiers and facts the resolver ranks by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags(u8);

impl Flags {
    pub const EXPECT: Flags = Flags(1);
    pub const ACTUAL: Flags = Flags(2);
    pub const PRIVATE: Flags = Flags(4);
    pub const INTERNAL: Flags = Flags(8);
    pub const OVERRIDE: Flags = Flags(16);
    pub const COMPANION: Flags = Flags(32);
    /// `type_hint` comes from a call in the initializer (`val r = Repo()`), not a declared type.
    pub const CALL_HINT: Flags = Flags(64);

    pub fn contains(self, other: Flags) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn insert(&mut self, other: Flags) {
        self.0 |= other.0;
    }
}

impl std::ops::BitOr for Flags {
    type Output = Flags;

    fn bitor(self, other: Flags) -> Flags {
        Flags(self.0 | other.0)
    }
}

/// Single-line range; columns are UTF-16 code units, as LSP expects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub line: u32,
    pub start: u32,
    pub end: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Import {
    /// Dotted path; for wildcard imports it is stored without the trailing `.*`.
    pub path: String,
    pub alias: Option<String>,
    pub wildcard: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decl {
    pub name: String,
    pub fqn: String,
    pub kind: DeclKind,
    /// Index of the enclosing declaration in `FileSummary::decls`.
    pub container: Option<u32>,
    pub name_range: Span,
    /// Receiver type of an extension: `fun Foo.bar()` → `Foo`.
    pub receiver: Option<String>,
    /// Type of a property: declared (`val r: Repo?` → `Repo`) or called (`val r = Repo()`, see `Flags::CALL_HINT`).
    pub type_hint: Option<String>,
    /// Supertypes of a class-like declaration as written, without type arguments.
    pub supertypes: Vec<String>,
    pub flags: Flags,
}

impl Decl {
    pub fn parent_fqn(&self) -> &str {
        self.fqn.rsplit_once('.').map_or("", |(parent, _)| parent)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileSummary {
    pub path: PathBuf,
    pub lang: Lang,
    pub package: String,
    pub imports: Vec<Import>,
    pub decls: Vec<Decl>,
    pub has_errors: bool,
    /// Nearest directory up from the file with a Gradle build script.
    pub module: Option<PathBuf>,
    /// The `src/<name>/` directory the file lives in.
    pub source_set: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(fqn: &str) -> Decl {
        let name = fqn.rsplit('.').next().unwrap().to_string();
        Decl {
            name,
            fqn: fqn.into(),
            kind: DeclKind::Class,
            container: None,
            name_range: Span { line: 0, start: 0, end: 0 },
            receiver: None,
            type_hint: None,
            supertypes: Vec::new(),
            flags: Flags::default(),
        }
    }

    #[test]
    fn parent_fqn_drops_last_segment() {
        assert_eq!(decl("com.a.Outer.Inner").parent_fqn(), "com.a.Outer");
        assert_eq!(decl("Top").parent_fqn(), "");
    }
}
