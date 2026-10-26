//! tree-sitter parsing: source text → `Tree` → `FileSummary`.

mod java;
mod kotlin;

use std::cell::RefCell;
use std::ops::Range;
use std::path::{Path, PathBuf};

use tree_sitter::{Language, Node, Parser, Tree};

use crate::docs::utf16_len;
use crate::model::{Decl, DeclKind, FileSummary, Lang, Span};

thread_local! {
    static KOTLIN: RefCell<Parser> = RefCell::new(new_parser(&tree_sitter_kotlin_ng::LANGUAGE.into()));
    static JAVA: RefCell<Parser> = RefCell::new(new_parser(&tree_sitter_java::LANGUAGE.into()));
}

fn new_parser(language: &Language) -> Parser {
    let mut parser = Parser::new();
    parser.set_language(language).expect("grammar ABI matches tree-sitter");
    parser
}

pub fn lang_of(path: &Path) -> Option<Lang> {
    match path.extension()?.to_str()? {
        "kt" | "kts" => Some(Lang::Kotlin),
        "java" => Some(Lang::Java),
        _ => None,
    }
}

pub fn parse(lang: Lang, text: &str) -> Tree {
    let parser = match lang {
        Lang::Kotlin => &KOTLIN,
        Lang::Java => &JAVA,
    };
    // `parse` only returns None without a language, a timeout or a cancellation flag; none are set.
    parser.with_borrow_mut(|p| p.parse(text, None)).expect("parser has a language")
}

pub fn summarize(path: PathBuf, lang: Lang, text: &str, tree: &Tree) -> FileSummary {
    match lang {
        Lang::Kotlin => kotlin::summarize(path, text, tree),
        Lang::Java => java::summarize(path, text, tree),
    }
}

pub fn summarize_text(path: PathBuf, text: &str) -> Option<(Tree, FileSummary)> {
    let lang = lang_of(&path)?;
    let tree = parse(lang, text);
    let summary = summarize(path, lang, text, &tree);
    Some((tree, summary))
}

pub(crate) fn span_of(text: &str, node: Node) -> Span {
    let pos = node.start_position();
    let line_start = node.start_byte() - pos.column;
    let start = utf16_len(&text[line_start..node.start_byte()]);
    let end = start + utf16_len(&text[node.byte_range()]);
    Span { line: pos.row as u32, start, end }
}

/// The span of `text[range]`, for a token found without a tree node.
pub(crate) fn span_at(text: &str, range: Range<usize>) -> Span {
    let line_start = text[..range.start].rfind('\n').map_or(0, |i| i + 1);
    let line = text.as_bytes()[..line_start].iter().filter(|&&b| b == b'\n').count() as u32;
    let start = utf16_len(&text[line_start..range.start]);
    Span { line, start, end: start + utf16_len(&text[range]) }
}

/// Identifier text without Kotlin backticks.
pub(crate) fn ident_text(text: &str, node: Node) -> String {
    text[node.byte_range()].trim_matches('`').to_string()
}

/// Dotted path text without whitespace and backticks: `a . `b` .C` → `a.b.C`.
pub(crate) fn dotted_text(text: &str, node: Node) -> String {
    clean_dotted(&text[node.byte_range()])
}

pub(crate) fn clean_dotted(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace() && *c != '`').collect()
}

pub(crate) fn qualify(prefix: &str, name: &str) -> String {
    if prefix.is_empty() { name.to_string() } else { format!("{prefix}.{name}") }
}

pub(crate) fn named_child_of_kind<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).find(|c| c.kind() == kind)
}

/// Collects declarations of one file, building FQNs from the package and container chain.
pub(crate) struct DeclSink<'a> {
    pub text: &'a str,
    pub package: &'a str,
    pub decls: Vec<Decl>,
}

impl DeclSink<'_> {
    /// Adds a declaration named by `name_node` (or `fixed_name`) and returns its index.
    pub fn push(&mut self, name_node: Node, fixed_name: Option<&str>, kind: DeclKind, container: Option<u32>) -> u32 {
        let name = fixed_name.map_or_else(|| ident_text(self.text, name_node), str::to_string);
        self.add(name, span_of(self.text, name_node), kind, container)
    }

    /// Like `push`, for a name token at `text[range]` that has no tree node.
    pub fn push_range(&mut self, fixed_name: Option<&str>, range: Range<usize>, kind: DeclKind, container: Option<u32>) -> u32 {
        let name = fixed_name.map_or_else(|| self.text[range.clone()].trim_matches('`').to_string(), str::to_string);
        self.add(name, span_at(self.text, range), kind, container)
    }

    fn add(&mut self, name: String, name_range: Span, kind: DeclKind, container: Option<u32>) -> u32 {
        let prefix = container.map_or(self.package, |c| self.decls[c as usize].fqn.as_str());
        let fqn = qualify(prefix, &name);
        self.decls.push(Decl { name, fqn, kind, container, name_range });
        (self.decls.len() - 1) as u32
    }
}
