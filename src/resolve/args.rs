//! Named arguments: the call a label belongs to and the parameter it names.

use tree_sitter::{Node, Tree};

use crate::docs::byte_offset;
use crate::model::Span;
use crate::parse::named_child_of_kind;

/// The callee of the call whose argument `label` names: `f(a = 1)` and `x.f(a = 1)` → `f`.
pub fn label_callee<'t>(label: Node<'t>) -> Option<Node<'t>> {
    let argument = label.parent().filter(|p| p.kind() == "value_argument")?;
    let call = argument.parent()?.parent().filter(|c| c.kind() == "call_expression")?;
    let callee = call.named_child(0)?;
    match callee.kind() {
        "identifier" => Some(callee),
        "navigation_expression" => {
            let mut cursor = callee.walk();
            callee.named_children(&mut cursor).filter(|c| c.kind() == "identifier").last()
        }
        _ => None,
    }
}

/// The identifier of parameter `name` of the function or class whose name is at `decl`.
pub fn parameter<'t>(tree: &'t Tree, text: &str, decl: Span, name: &str) -> Option<Node<'t>> {
    let byte = byte_offset(text, decl.line, decl.start)?;
    let owner = tree.root_node().descendant_for_byte_range(byte, byte)?.parent()?;
    let list = match owner.kind() {
        "function_declaration" => named_child_of_kind(owner, "function_value_parameters")?,
        "class_declaration" => named_child_of_kind(named_child_of_kind(owner, "primary_constructor")?, "class_parameters")?,
        _ => return None,
    };
    let mut cursor = list.walk();
    list.named_children(&mut cursor)
        .filter_map(|p| named_child_of_kind(p, "identifier"))
        .find(|id| &text[id.byte_range()] == name)
}

/// The name of the function or class a parameter name belongs to; `None` for the parameters
/// of lambdas, `catch`, setters and secondary constructors.
pub fn parameter_owner<'t>(name: Node<'t>) -> Option<Node<'t>> {
    let list = name.parent()?.parent()?;
    let owner = list.parent()?;
    match (list.kind(), owner.kind()) {
        ("function_value_parameters", "function_declaration") => owner.child_by_field_name("name"),
        ("class_parameters", "primary_constructor") => owner.parent()?.child_by_field_name("name"),
        _ => None,
    }
}
