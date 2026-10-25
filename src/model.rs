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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(fqn: &str) -> Decl {
        let name = fqn.rsplit('.').next().unwrap().to_string();
        Decl { name, fqn: fqn.into(), kind: DeclKind::Class, container: None, name_range: Span { line: 0, start: 0, end: 0 } }
    }

    #[test]
    fn parent_fqn_drops_last_segment() {
        assert_eq!(decl("com.a.Outer.Inner").parent_fqn(), "com.a.Outer");
        assert_eq!(decl("Top").parent_fqn(), "");
    }
}
