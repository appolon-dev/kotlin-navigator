//! Declarations visible from a point in the current Kotlin tree that the index does not
//! hold: parameters and locals, and the implicit receivers (`this`) around the point.

use std::ops::Range;

use tree_sitter::{Node, Tree};

use crate::model::Span;
use crate::parse::{declared, named_child_of_kind, receiver_before, span_of, type_child, variables};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalKind {
    Value,
    Function,
    Class,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Local {
    /// Bytes of the declared name; for an implicit `it`, the lambda's `{`.
    pub range: Range<usize>,
    pub kind: LocalKind,
    pub type_name: Option<String>,
    /// `type_name` comes from an initializer `Repo(x)`.
    pub from_call: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Implicit {
    /// Inside `fun Repo.ext()`: the receiver type name.
    Extension(String),
    /// Inside a class-like declaration: the bytes of its name (`companion` for an unnamed companion).
    Container(Range<usize>),
}

/// Local declarations of `name` visible at `at`, nearest first.
pub fn find_local(tree: &Tree, text: &str, at: usize, name: &str) -> Vec<Local> {
    let mut found = Vec::new();
    // Plain class parameters are visible in initializers, not inside members' bodies.
    let mut in_body = false;
    let mut current = tree.root_node().descendant_for_byte_range(at, at);
    while let Some(node) = current {
        let mut here = Vec::new();
        match node.kind() {
            "block" => statements(text, node, at, name, &mut here),
            "lambda_literal" => {
                statements(text, node, at, name, &mut here);
                match named_child_of_kind(node, "lambda_parameters") {
                    Some(params) => {
                        let mut cursor = params.walk();
                        for param in params.named_children(&mut cursor) {
                            values(text, declared(param), name, &mut here);
                        }
                    }
                    None if name == "it" => here.push(value(node.child(0).map_or(node.byte_range(), |b| b.byte_range()), None)),
                    None => {}
                }
            }
            "function_declaration" | "secondary_constructor" => {
                in_body = true;
                if let Some(params) = named_child_of_kind(node, "function_value_parameters") {
                    let mut cursor = params.walk();
                    for param in params.named_children(&mut cursor).filter(|p| p.kind() == "parameter") {
                        parameter(text, param, name, &mut here);
                    }
                }
            }
            "getter" => in_body = true,
            "setter" => {
                in_body = true;
                parameter(text, node, name, &mut here);
            }
            "for_statement" => {
                let mut cursor = node.walk();
                let vars = node.named_children(&mut cursor).find(|c| matches!(c.kind(), "variable_declaration" | "multi_variable_declaration"));
                values(text, vars.map(declared).unwrap_or_default(), name, &mut here);
            }
            "catch_block" => parameter(text, node, name, &mut here),
            "when_expression" => {
                if let Some(subject) = named_child_of_kind(node, "when_subject") {
                    for var in variables(text, subject).into_iter().filter(|v| &text[v.name.byte_range()] == name) {
                        here.push(Local { range: var.name.byte_range(), kind: LocalKind::Value, type_name: var.type_name, from_call: var.from_call });
                    }
                }
            }
            "class_declaration" if !in_body => {
                let params = named_child_of_kind(node, "primary_constructor").and_then(|p| named_child_of_kind(p, "class_parameters"));
                if let Some(params) = params {
                    let mut cursor = params.walk();
                    for param in params.named_children(&mut cursor).filter(|p| p.kind() == "class_parameter" && !is_property(*p)) {
                        parameter(text, param, name, &mut here);
                    }
                }
            }
            _ => {}
        }
        found.extend(here);
        current = node.parent();
    }
    found
}

/// Declarations among `scope`'s statements before `at`, last first. A variable is visible
/// after its declaration, not in its own initializer; local functions and classes from
/// their start, for recursion.
fn statements(text: &str, scope: Node, at: usize, name: &str, out: &mut Vec<Local>) {
    let mut cursor = scope.walk();
    let before: Vec<Node> = scope.named_children(&mut cursor).filter(|s| s.start_byte() < at).collect();
    for statement in before.into_iter().rev() {
        match statement.kind() {
            "property_declaration" => {
                let vars = variables(text, statement);
                let declared = statement.end_byte() <= at;
                let seen = vars.into_iter().rev().filter(|v| declared || v.name.byte_range().contains(&at));
                for var in seen.filter(|v| &text[v.name.byte_range()] == name) {
                    out.push(Local { range: var.name.byte_range(), kind: LocalKind::Value, type_name: var.type_name, from_call: var.from_call });
                }
            }
            "function_declaration" | "class_declaration" | "object_declaration" => {
                let kind = if statement.kind() == "function_declaration" { LocalKind::Function } else { LocalKind::Class };
                if let Some(n) = statement.child_by_field_name("name").filter(|n| &text[n.byte_range()] == name) {
                    out.push(Local { range: n.byte_range(), kind, type_name: None, from_call: false });
                }
            }
            _ => {}
        }
    }
}

/// `variable_declaration`s named `name`, typed by what is written on them.
fn values(text: &str, vars: Vec<Node>, name: &str, out: &mut Vec<Local>) {
    for var in vars {
        if let Some(n) = named_child_of_kind(var, "identifier").filter(|n| &text[n.byte_range()] == name) {
            out.push(value(n.byte_range(), type_child(text, var)));
        }
    }
}

/// A node holding an `identifier` and an optional type: a parameter, a catch or a setter.
fn parameter(text: &str, node: Node, name: &str, out: &mut Vec<Local>) {
    if let Some(n) = named_child_of_kind(node, "identifier").filter(|n| &text[n.byte_range()] == name) {
        out.push(value(n.byte_range(), type_child(text, node)));
    }
}

fn value(range: Range<usize>, type_name: Option<String>) -> Local {
    Local { range, kind: LocalKind::Value, type_name, from_call: false }
}

pub(super) fn is_property(param: Node) -> bool {
    let mut cursor = param.walk();
    param.children(&mut cursor).any(|c| !c.is_named() && matches!(c.kind(), "val" | "var"))
}

/// Name spans of the functions and properties called `name` whose declaration contains `at`.
pub fn enclosing_declarations(tree: &Tree, text: &str, at: usize, name: &str) -> Vec<Span> {
    let mut out = Vec::new();
    let mut current = tree.root_node().descendant_for_byte_range(at, at);
    while let Some(node) = current {
        let names = match node.kind() {
            "function_declaration" => node.child_by_field_name("name").into_iter().collect(),
            "property_declaration" => variables(text, node).into_iter().map(|v| v.name).collect(),
            _ => Vec::new(),
        };
        out.extend(names.into_iter().filter(|n| &text[n.byte_range()] == name).map(|n| span_of(text, n)));
        current = node.parent();
    }
    out
}

/// Implicit receivers at `at`, innermost first.
pub fn implicit_receivers(tree: &Tree, text: &str, at: usize) -> Vec<Implicit> {
    let mut out = Vec::new();
    let mut current = tree.root_node().descendant_for_byte_range(at, at);
    while let Some(node) = current {
        match node.kind() {
            "function_declaration" => {
                let name = node.child_by_field_name("name").map_or(node.end_byte(), |n| n.start_byte());
                out.extend(receiver_before(text, node, name).map(Implicit::Extension));
            }
            "class_declaration" | "object_declaration" => out.extend(node.child_by_field_name("name").map(|n| Implicit::Container(n.byte_range()))),
            "companion_object" => {
                let mut cursor = node.walk();
                let keyword = node.children(&mut cursor).find(|c| !c.is_named() && c.kind() == "companion");
                out.extend(node.child_by_field_name("name").or(keyword).map(|n| Implicit::Container(n.byte_range())));
            }
            _ => {}
        }
        current = node.parent();
    }
    out
}
