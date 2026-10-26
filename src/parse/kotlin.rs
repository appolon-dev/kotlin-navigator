//! Kotlin summaries for the `tree-sitter-kotlin-ng` grammar. `declaration`,
//! `statement` and `class_member_declaration` are supertypes, so declarations
//! are direct children of `source_file`, `class_body` and `enum_class_body`.
//!
//! Two grammar weaknesses are worked around: an annotation line before a
//! soft-keyword declaration at top level (`@Serializable\ndata object D`) is
//! read as an expression, and some valid class bodies collapse into one
//! `ERROR` node (see `recover`).

mod recover;

use std::path::PathBuf;

use tree_sitter::{Node, Tree};

use self::recover::recover;
use super::{DeclSink, dotted_text, ident_text, named_child_of_kind, parse};
use crate::model::{DeclKind, FileSummary, Import, Lang};
use crate::source_set::source_set_of;

pub(crate) fn summarize(path: PathBuf, text: &str, tree: &Tree) -> FileSummary {
    let root = tree.root_node();
    let mut package = String::new();
    let mut imports = Vec::new();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        match child.kind() {
            "package_header" => {
                if let Some(q) = named_child_of_kind(child, "qualified_identifier") {
                    package = dotted_text(text, q);
                }
            }
            "import" => imports.extend(import(text, child)),
            _ => {}
        }
    }
    // Annotations never declare anything; blanking them (offsets unchanged) lets the
    // grammar see the declarations. Spans are still computed against the original text.
    let reparsed = needs_reparse(root).then(|| parse(Lang::Kotlin, &blank_annotations(text, root)));
    let root = reparsed.as_ref().map_or(root, Tree::root_node);
    let mut sink = DeclSink { text, package: &package, decls: Vec::new() };
    if root.kind() == "ERROR" {
        recover(&mut sink, root, None);
    } else {
        visit(&mut sink, root, None);
    }
    let source_set = source_set_of(&path);
    FileSummary {
        path,
        lang: Lang::Kotlin,
        package: package.clone(),
        imports,
        decls: sink.decls,
        has_errors: root.has_error(),
        module: None,
        source_set,
    }
}

fn import(text: &str, node: Node) -> Option<Import> {
    let (mut path, mut alias, mut wildcard) = (None, None, false);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "qualified_identifier" => path = Some(dotted_text(text, child)),
            "identifier" => alias = Some(ident_text(text, child)),
            "*" => wildcard = true,
            _ => {}
        }
    }
    Some(Import { path: path?, alias, wildcard })
}

fn needs_reparse(root: Node) -> bool {
    let mut cursor = root.walk();
    root.has_error() || root.named_children(&mut cursor).any(|c| c.kind() == "annotated_expression")
}

/// The text with every annotation replaced by spaces (newlines kept, so byte offsets and rows stay put).
fn blank_annotations(text: &str, root: Node) -> String {
    let mut bytes = text.as_bytes().to_vec();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "annotation" {
            for b in &mut bytes[node.byte_range()] {
                if *b != b'\n' {
                    *b = b' ';
                }
            }
            continue;
        }
        let mut cursor = node.walk();
        stack.extend(node.named_children(&mut cursor));
    }
    String::from_utf8(bytes).expect("only whole annotations are replaced, by ASCII spaces")
}

fn visit(sink: &mut DeclSink, node: Node, container: Option<u32>) {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.named_children(&mut cursor).collect();
    for child in children {
        if child.kind() == "ERROR" {
            recover(sink, child, container);
        } else {
            declare(sink, child, container);
        }
    }
}

/// Adds `node` and its body members if it is a declaration; returns whether it was.
fn declare(sink: &mut DeclSink, node: Node, container: Option<u32>) -> bool {
    let found = match node.kind() {
        "class_declaration" => node.child_by_field_name("name").map(|n| (n, None, class_kind(sink.text, node))),
        "object_declaration" => node.child_by_field_name("name").map(|n| (n, None, DeclKind::Object)),
        "companion_object" => match node.child_by_field_name("name") {
            Some(n) => Some((n, None, DeclKind::Object)),
            None => anonymous_child(node, "companion").map(|n| (n, Some("Companion"), DeclKind::Object)),
        },
        "type_alias" => node.child_by_field_name("type").map(|n| (n, None, DeclKind::TypeAlias)),
        "enum_entry" => named_child_of_kind(node, "identifier").map(|n| (n, None, DeclKind::EnumEntry)),
        _ => None,
    };
    let Some((name_node, fixed_name, kind)) = found else { return false };
    let index = sink.push(name_node, fixed_name, kind, container);
    for body in ["class_body", "enum_class_body"] {
        if let Some(body) = named_child_of_kind(node, body) {
            visit(sink, body, Some(index));
        }
    }
    true
}

fn class_kind(text: &str, node: Node) -> DeclKind {
    if anonymous_child(node, "interface").is_some() {
        return DeclKind::Interface;
    }
    if named_child_of_kind(node, "modifiers").is_some_and(|m| is_enum(text, m)) { DeclKind::Enum } else { DeclKind::Class }
}

/// Whether `modifiers` contains the `enum` class modifier.
fn is_enum(text: &str, modifiers: Node) -> bool {
    if modifiers.kind() != "modifiers" {
        return false;
    }
    let mut cursor = modifiers.walk();
    modifiers.named_children(&mut cursor).any(|c| c.kind() == "class_modifier" && &text[c.byte_range()] == "enum")
}

fn anonymous_child<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).find(|c| !c.is_named() && c.kind() == kind)
}
