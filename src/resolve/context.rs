//! What the identifier under the cursor is: its role (type, call, plain expression…)
//! and, for a member access, what it is accessed on.

use std::ops::Range;

use tree_sitter::{Node, Tree};

use crate::parse::{clean_dotted, dotted_identifiers, ident_text, named_child_of_kind, type_name};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Role {
    /// Part of an import path: the path up to and including the identifier.
    Import(String),
    Type,
    Call,
    /// `::name` or `Type::name`.
    CallableRef,
    Expr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Receiver {
    /// Not a member access.
    None,
    This,
    Super,
    /// `a.b.Foo.name`: the dotted identifiers before the name.
    Path(String),
    /// A member access on something other than a path (`foo().bar`, Java members).
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    pub name: String,
    /// Bytes of the identifier.
    pub range: Range<usize>,
    pub role: Role,
    pub receiver: Receiver,
}

pub fn reference_at(tree: &Tree, text: &str, byte: usize) -> Option<Reference> {
    let root = tree.root_node();
    let java = tree.language().id_for_node_kind("program", true) != 0;
    // The cursor may sit right after the name, e.g. `Foo|)`.
    let (node, range) = name_at(root, text, byte, java).or_else(|| name_at(root, text, byte.checked_sub(1)?, java))?;
    if node.kind() == "string_content" {
        return Some(Reference { name: text[range.clone()].to_string(), range, role: Role::Expr, receiver: Receiver::None });
    }
    let (role, receiver) = if java { java_context(text, node)? } else { kotlin_context(text, node)? };
    Some(Reference { name: ident_text(text, node), range, role, receiver })
}

/// An identifier at `byte`, or in Kotlin the name of a `$name` template, with its bytes.
fn name_at<'t>(root: Node<'t>, text: &str, byte: usize, java: bool) -> Option<(Node<'t>, Range<usize>)> {
    let node = root.named_descendant_for_byte_range(byte, byte)?;
    match node.kind() {
        "identifier" | "type_identifier" => Some((node, node.byte_range())),
        // `"$v"` is `string_content "$"` followed by `string_content "v…"`.
        "string_content" if !java => {
            let dollar = node.prev_sibling().filter(|p| p.kind() == "string_content" && &text[p.byte_range()] == "$")?;
            let start = dollar.end_byte();
            let len = text[start..node.end_byte()].find(|c: char| !(c == '_' || c.is_alphanumeric())).unwrap_or(node.end_byte() - start);
            (len > 0 && byte < start + len).then_some((node, start..start + len))
        }
        _ => None,
    }
}

fn kotlin_context(text: &str, ident: Node) -> Option<(Role, Receiver)> {
    if let Some(path) = import_fqn(text, ident) {
        return Some((Role::Import(path), Receiver::None));
    }
    if has_ancestor(ident, "package_header") {
        return None;
    }
    let parent = ident.parent()?;
    let first = parent.named_child(0).is_some_and(|f| f.id() == ident.id());
    let is_name = parent.child_by_field_name("name").is_some_and(|n| n.id() == ident.id());
    Some(match parent.kind() {
        "user_type" => {
            let mut cursor = parent.walk();
            let before: Vec<String> = parent
                .named_children(&mut cursor)
                .filter(|c| c.kind() == "identifier" && c.start_byte() < ident.start_byte())
                .map(|c| ident_text(text, c))
                .collect();
            (Role::Type, if before.is_empty() { Receiver::None } else { Receiver::Path(before.join(".")) })
        }
        "navigation_expression" if !first => {
            let receiver = parent.named_child(0).map_or(Receiver::Unknown, |r| receiver_of(text, r));
            let callable = child_of_kind(parent, "::").is_some();
            let called = parent.parent().is_some_and(|c| {
                c.kind() == "call_expression" && c.named_child(0).is_some_and(|f| f.id() == parent.id())
            });
            let role = if callable { Role::CallableRef } else if called { Role::Call } else { Role::Expr };
            (role, receiver)
        }
        "callable_reference" => {
            let mut cursor = parent.walk();
            let owner = parent.named_children(&mut cursor).find(|c| c.end_byte() <= ident.start_byte());
            (Role::CallableRef, owner.and_then(|t| type_name(text, t)).map_or(Receiver::None, Receiver::Path))
        }
        "call_expression" if first => (Role::Call, Receiver::None),
        "value_argument" if ident.next_sibling().is_some_and(|n| n.kind() == "=") => (Role::Expr, Receiver::Unknown),
        "class_declaration" | "object_declaration" if is_name => (Role::Type, Receiver::None),
        "function_declaration" if is_name => (Role::Call, Receiver::None),
        "this_expression" | "super_expression" => return None,
        _ => (Role::Expr, Receiver::None),
    })
}

fn receiver_of(text: &str, node: Node) -> Receiver {
    match node.kind() {
        "this_expression" => Receiver::This,
        "super_expression" => Receiver::Super,
        _ => dotted_identifiers(text, node).map_or(Receiver::Unknown, Receiver::Path),
    }
}

fn java_context(text: &str, ident: Node) -> Option<(Role, Receiver)> {
    if let Some(path) = import_fqn(text, ident) {
        return Some((Role::Import(path), Receiver::None));
    }
    if has_ancestor(ident, "package_declaration") {
        return None;
    }
    let parent = ident.parent()?;
    let field_is = |field: &str| parent.child_by_field_name(field).is_some_and(|n| n.id() == ident.id());
    Some(match (ident.kind(), parent.kind()) {
        ("type_identifier", "scoped_type_identifier") if parent.named_child(0).is_some_and(|q| q.id() != ident.id()) => {
            let qualifier = parent.named_child(0).and_then(|q| type_name(text, q));
            (Role::Type, qualifier.map_or(Receiver::None, Receiver::Path))
        }
        ("type_identifier", _) => (Role::Type, Receiver::None),
        (_, "method_invocation") if field_is("name") => (Role::Call, Receiver::Unknown),
        (_, "field_access") if field_is("field") => (Role::Expr, Receiver::Unknown),
        (_, "method_declaration") if field_is("name") => (Role::Call, Receiver::None),
        _ => (Role::Expr, Receiver::None),
    })
}

fn has_ancestor(node: Node, kind: &str) -> bool {
    let mut current = node.parent();
    while let Some(n) = current {
        if n.kind() == kind {
            return true;
        }
        current = n.parent();
    }
    false
}

fn child_of_kind<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).find(|c| c.kind() == kind)
}

/// For an identifier inside an import path: the path up to and including it.
pub(crate) fn import_fqn(text: &str, ident: Node) -> Option<String> {
    let mut current = ident.parent();
    while let Some(node) = current {
        let path = match node.kind() {
            "import" => named_child_of_kind(node, "qualified_identifier"),
            "import_declaration" => {
                let mut cursor = node.walk();
                node.named_children(&mut cursor).find(|c| matches!(c.kind(), "scoped_identifier" | "identifier"))
            }
            _ => {
                current = node.parent();
                continue;
            }
        };
        let path = path?;
        // An alias (`import a.B as C`) lies outside the path: it means the whole path.
        let inside = path.byte_range().contains(&ident.start_byte());
        let end = if inside { ident.end_byte() } else { path.end_byte() };
        return Some(clean_dotted(&text[path.start_byte()..end]));
    }
    None
}
