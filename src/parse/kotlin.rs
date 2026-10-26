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
use super::{DeclSink, dotted_identifiers, dotted_text, ident_text, named_child_of_kind, parse, type_name};
use crate::model::{DeclKind, FileSummary, Flags, Import, Lang};
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
        } else if let Some((word, body)) = swallowed_companion(sink.text, child).filter(|_| node.kind() == "lambda_literal") {
            let index = sink.push(word, Some("Companion"), DeclKind::Object, container);
            sink.decls[index as usize].flags = Flags::COMPANION;
            visit(sink, body, Some(index));
        } else {
            declare(sink, child, container);
        }
    }
}

/// Adds `node` and its body members if it is a declaration; returns whether it was.
/// Bodies of functions, accessors, initializers and lambdas are not entered.
fn declare(sink: &mut DeclSink, node: Node, container: Option<u32>) -> bool {
    match node.kind() {
        "function_declaration" => return function(sink, node, container),
        "property_declaration" => return property(sink, node, container),
        _ => {}
    }
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
    let mut flags = modifier_flags(sink.text, node);
    if node.kind() == "companion_object" {
        flags.insert(Flags::COMPANION);
    }
    let decl = &mut sink.decls[index as usize];
    decl.flags = flags;
    decl.supertypes = supertypes(sink.text, node);
    let params = named_child_of_kind(node, "primary_constructor").and_then(|p| named_child_of_kind(p, "class_parameters"));
    if let Some(params) = params {
        let mut cursor = params.walk();
        for param in params.named_children(&mut cursor).filter(|p| p.kind() == "class_parameter") {
            let is_property = anonymous_child(param, "val").or_else(|| anonymous_child(param, "var")).is_some();
            if let (true, Some(name)) = (is_property, named_child_of_kind(param, "identifier")) {
                let p = sink.push(name, None, DeclKind::Property, Some(index));
                sink.decls[p as usize].type_hint = type_child(sink.text, param);
                sink.decls[p as usize].flags = modifier_flags(sink.text, param);
            }
        }
    }
    let mut has_body = false;
    for body in ["class_body", "enum_class_body"] {
        if let Some(body) = named_child_of_kind(node, body) {
            visit(sink, body, Some(index));
            has_body = true;
        }
    }
    if let Some(lambda) = swallowed_body(node).filter(|_| !has_body) {
        visit(sink, lambda, Some(index));
    }
    true
}

fn function(sink: &mut DeclSink, node: Node, container: Option<u32>) -> bool {
    let Some(name) = node.child_by_field_name("name") else { return false };
    let receiver = receiver_before(sink.text, node, name.start_byte());
    let index = sink.push(name, None, DeclKind::Function, container) as usize;
    sink.decls[index].receiver = receiver;
    sink.decls[index].flags = modifier_flags(sink.text, node);
    true
}

/// One `Property` per declared variable; `val (a, b) = …` declares two.
fn property(sink: &mut DeclSink, node: Node, container: Option<u32>) -> bool {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    let Some(at) = children.iter().position(|c| matches!(c.kind(), "variable_declaration" | "multi_variable_declaration"))
    else {
        return false;
    };
    let receiver = receiver_before(sink.text, node, children[at].start_byte());
    let flags = modifier_flags(sink.text, node);
    // `val r = Repo(x)`: the initializer is the named node right after `=`.
    let call_hint = match children[at + 1..] {
        [eq, init, ..] if eq.kind() == "=" && children[at].kind() == "variable_declaration" => call_type(sink.text, init),
        _ => None,
    };
    let vars = if children[at].kind() == "variable_declaration" {
        vec![children[at]]
    } else {
        let mut cursor = children[at].walk();
        children[at].named_children(&mut cursor).filter(|v| v.kind() == "variable_declaration").collect()
    };
    for var in vars {
        let Some(name) = named_child_of_kind(var, "identifier") else { continue };
        let index = sink.push(name, None, DeclKind::Property, container) as usize;
        let decl = &mut sink.decls[index];
        decl.receiver = receiver.clone();
        decl.flags = flags;
        decl.type_hint = type_child(sink.text, var);
        if decl.type_hint.is_none() && call_hint.is_some() {
            decl.type_hint = call_hint.clone();
            decl.flags.insert(Flags::CALL_HINT);
        }
    }
    true
}

const TYPES: [&str; 4] = ["user_type", "nullable_type", "parenthesized_type", "function_type"];

/// The name of the first type node among `node`'s children.
fn type_child(text: &str, node: Node) -> Option<String> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).find(|c| TYPES.contains(&c.kind())).and_then(|t| type_name(text, t))
}

/// The receiver type of an extension: a type child before the declared name.
fn receiver_before(text: &str, node: Node, name_start: usize) -> Option<String> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .take_while(|c| c.start_byte() < name_start)
        .find(|c| TYPES.contains(&c.kind()))
        .and_then(|t| type_name(text, t))
}

/// `Repo(x)` or `a.b.Repo(x)`: the called name when it looks like a class (capitalized last segment).
fn call_type(text: &str, init: Node) -> Option<String> {
    if init.kind() != "call_expression" {
        return None;
    }
    let name = dotted_identifiers(text, init.named_child(0)?)?;
    let last = name.rsplit('.').next()?;
    last.starts_with(|c: char| c.is_uppercase()).then_some(name)
}

fn modifier_flags(text: &str, node: Node) -> Flags {
    let mut flags = Flags::default();
    let Some(modifiers) = named_child_of_kind(node, "modifiers") else { return flags };
    let mut cursor = modifiers.walk();
    for m in modifiers.named_children(&mut cursor) {
        match (m.kind(), &text[m.byte_range()]) {
            ("visibility_modifier", "private") => flags.insert(Flags::PRIVATE),
            ("visibility_modifier", "internal") => flags.insert(Flags::INTERNAL),
            ("platform_modifier", "expect") => flags.insert(Flags::EXPECT),
            ("platform_modifier", "actual") => flags.insert(Flags::ACTUAL),
            ("member_modifier", "override") => flags.insert(Flags::OVERRIDE),
            _ => {}
        }
    }
    flags
}

/// `: Base(), a.b.Iface<T>, Del by d` → `Base`, `a.b.Iface`, `Del`.
fn supertypes(text: &str, node: Node) -> Vec<String> {
    let Some(specifiers) = named_child_of_kind(node, "delegation_specifiers") else { return Vec::new() };
    let mut cursor = specifiers.walk();
    specifiers
        .named_children(&mut cursor)
        .filter_map(|s| s.named_child(0))
        .filter_map(|s| match s.kind() {
            "constructor_invocation" | "explicit_delegation" => s.named_child(0),
            _ => Some(s),
        })
        .filter_map(|t| type_name(text, t))
        .collect()
}

/// In `class A : I by d { … }` the grammar reads the body as a trailing lambda of `d`.
fn swallowed_body(node: Node) -> Option<Node> {
    let specifiers = named_child_of_kind(node, "delegation_specifiers")?;
    let delegation = last_named(specifiers)?.named_child(0).filter(|d| d.kind() == "explicit_delegation")?;
    let call = last_named(delegation)?;
    let lambda = last_named(call)?;
    (call.kind() == "call_expression" && lambda.kind() == "annotated_lambda")
        .then(|| named_child_of_kind(lambda, "lambda_literal"))
        .flatten()
}

fn last_named(node: Node) -> Option<Node> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).last()
}

/// `companion object { … }` inside a swallowed body reads as `companion object {…}` infix call.
fn swallowed_companion<'t>(text: &str, node: Node<'t>) -> Option<(Node<'t>, Node<'t>)> {
    let words = (node.named_child(0)?, node.named_child(1)?, node.named_child(2)?);
    let is = |n: Node, w: &str| n.kind() == "identifier" && &text[n.byte_range()] == w;
    (node.kind() == "infix_expression" && is(words.0, "companion") && is(words.1, "object") && words.2.kind() == "lambda_literal")
        .then_some((words.0, words.2))
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
