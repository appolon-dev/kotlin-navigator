//! tree-sitter parsing: source text → `Tree` → `FileSummary`.

mod kotlin;

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use tree_sitter::{Language, Node, Parser, Tree};

use crate::docs::utf16_len;
use crate::model::{FileSummary, Lang, Span};

thread_local! {
    static KOTLIN: RefCell<Parser> = RefCell::new(new_parser(&tree_sitter_kotlin_ng::LANGUAGE.into()));
}

fn new_parser(language: &Language) -> Parser {
    let mut parser = Parser::new();
    parser.set_language(language).expect("grammar ABI matches tree-sitter");
    parser
}

pub fn lang_of(path: &Path) -> Option<Lang> {
    match path.extension()?.to_str()? {
        "kt" | "kts" => Some(Lang::Kotlin),
        _ => None,
    }
}

pub fn parse(lang: Lang, text: &str) -> Tree {
    let parser = match lang {
        Lang::Kotlin => &KOTLIN,
        Lang::Java => unimplemented!("Java parsing"),
    };
    // `parse` only returns None without a language, a timeout or a cancellation flag; none are set.
    parser.with_borrow_mut(|p| p.parse(text, None)).expect("parser has a language")
}

pub fn summarize(path: PathBuf, lang: Lang, text: &str, tree: &Tree) -> FileSummary {
    match lang {
        Lang::Kotlin => kotlin::summarize(path, text, tree),
        Lang::Java => unimplemented!("Java parsing"),
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

/// Identifier text without Kotlin backticks.
pub(crate) fn ident_text(text: &str, node: Node) -> String {
    text[node.byte_range()].trim_matches('`').to_string()
}

/// Dotted path text without whitespace and backticks: `a . `b` .C` → `a.b.C`.
pub(crate) fn dotted_text(text: &str, node: Node) -> String {
    text[node.byte_range()].chars().filter(|c| !c.is_whitespace() && *c != '`').collect()
}

pub(crate) fn qualify(prefix: &str, name: &str) -> String {
    if prefix.is_empty() { name.to_string() } else { format!("{prefix}.{name}") }
}

pub(crate) fn named_child_of_kind<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).find(|c| c.kind() == kind)
}
