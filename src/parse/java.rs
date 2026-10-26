//! Java summaries for the `tree-sitter-java` grammar (`declaration` and
//! `statement` are supertypes, so declarations are direct children of bodies).

use std::path::PathBuf;

use tree_sitter::{Node, Tree};

use super::{DeclSink, dotted_text, named_child_of_kind, type_name};
use crate::model::{DeclKind, FileSummary, Flags, Import, Lang};
use crate::source_set::source_set_of;

pub(crate) fn summarize(path: PathBuf, text: &str, tree: &Tree) -> FileSummary {
    let root = tree.root_node();
    let mut package = String::new();
    let mut imports = Vec::new();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        match child.kind() {
            "package_declaration" => {
                if let Some(name) = path_child(child) {
                    package = dotted_text(text, name);
                }
            }
            "import_declaration" => {
                if let Some(name) = path_child(child) {
                    let mut c = child.walk();
                    let wildcard = child.named_children(&mut c).any(|n| n.kind() == "asterisk");
                    imports.push(Import { path: dotted_text(text, name), alias: None, wildcard });
                }
            }
            _ => {}
        }
    }
    let mut sink = DeclSink { text, package: &package, decls: Vec::new() };
    visit(&mut sink, root, None);
    let source_set = source_set_of(&path);
    FileSummary {
        path,
        lang: Lang::Java,
        package: package.clone(),
        imports,
        decls: sink.decls,
        has_errors: root.has_error(),
        module: None,
        source_set,
    }
}

fn path_child(node: Node) -> Option<Node> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).find(|c| matches!(c.kind(), "scoped_identifier" | "identifier"))
}

fn visit(sink: &mut DeclSink, node: Node, container: Option<u32>) {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.named_children(&mut cursor).collect();
    for child in children {
        let kind = match child.kind() {
            "class_declaration" | "record_declaration" => DeclKind::Class,
            "interface_declaration" | "annotation_type_declaration" => DeclKind::Interface,
            "enum_declaration" => DeclKind::Enum,
            "enum_constant" => DeclKind::EnumEntry,
            "method_declaration" => DeclKind::Function,
            "field_declaration" => {
                let mut c = child.walk();
                for declarator in child.children_by_field_name("declarator", &mut c) {
                    member(sink, child, declarator, container);
                }
                continue;
            }
            "enum_body_declarations" | "ERROR" => {
                visit(sink, child, container);
                continue;
            }
            _ => continue,
        };
        let Some(name) = child.child_by_field_name("name") else { continue };
        let index = sink.push(name, None, kind, container);
        sink.decls[index as usize].flags = flags(sink.text, child);
        sink.decls[index as usize].supertypes = supertypes(sink.text, child);
        if kind == DeclKind::Function {
            sink.decls[index as usize].type_hint = result_type(sink.text, child);
        }
        if let Some(params) = child.child_by_field_name("parameters").filter(|_| child.kind() == "record_declaration") {
            let mut c = params.walk();
            for param in params.named_children(&mut c).filter(|p| p.kind() == "formal_parameter") {
                member(sink, param, param, Some(index));
            }
        }
        if !matches!(kind, DeclKind::EnumEntry | DeclKind::Function) {
            if let Some(body) = child.child_by_field_name("body") {
                visit(sink, body, Some(index));
            }
        }
    }
}

/// A field declarator or record component: a `Property` typed by `typed`'s `type` field.
fn member(sink: &mut DeclSink, typed: Node, named: Node, container: Option<u32>) {
    let Some(name) = named.child_by_field_name("name") else { return };
    let index = sink.push(name, None, DeclKind::Property, container) as usize;
    sink.decls[index].type_hint = typed.child_by_field_name("type").and_then(|t| type_name(sink.text, t));
    sink.decls[index].flags = flags(sink.text, typed);
}

fn flags(text: &str, node: Node) -> Flags {
    let private = named_child_of_kind(node, "modifiers").is_some_and(|m| text[m.byte_range()].split_whitespace().any(|w| w == "private"));
    if private { Flags::PRIVATE } else { Flags::default() }
}

/// `extends Base implements I, a.K<T>` → `Base`, `I`, `a.K`.
fn supertypes(text: &str, node: Node) -> Vec<String> {
    let mut cursor = node.walk();
    let mut out = Vec::new();
    for clause in node.named_children(&mut cursor) {
        let list = match clause.kind() {
            "superclass" => Some(clause),
            "super_interfaces" | "extends_interfaces" => named_child_of_kind(clause, "type_list"),
            _ => None,
        };
        if let Some(list) = list {
            let mut c = list.walk();
            out.extend(list.named_children(&mut c).filter_map(|t| type_name(text, t)));
        }
    }
    out
}

/// The declared result type of a method, unless it is one of the method's own type parameters.
fn result_type(text: &str, method: Node) -> Option<String> {
    let name = type_name(text, method.child_by_field_name("type")?)?;
    let generic = named_child_of_kind(method, "type_parameters").is_some_and(|tps| {
        let mut c = tps.walk();
        tps.named_children(&mut c)
            .filter(|p| p.kind() == "type_parameter")
            .any(|p| named_child_of_kind(p, "type_identifier").is_some_and(|i| &text[i.byte_range()] == name))
    });
    (!generic).then_some(name)
}
