//! Kotlin summaries for the `tree-sitter-kotlin-ng` grammar. `declaration`,
//! `statement` and `class_member_declaration` are supertypes, so declarations
//! are direct children of `source_file`, `class_body` and `enum_class_body`.

use std::path::PathBuf;

use tree_sitter::{Node, Tree};

use super::{dotted_text, ident_text, named_child_of_kind, qualify, span_of};
use crate::model::{Decl, DeclKind, FileSummary, Import, Lang};

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
    let mut walker = Walker { text, package: &package, decls: Vec::new() };
    walker.visit(root, None);
    FileSummary {
        path,
        lang: Lang::Kotlin,
        package: package.clone(),
        imports,
        decls: walker.decls,
        has_errors: root.has_error(),
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

struct Walker<'a> {
    text: &'a str,
    package: &'a str,
    decls: Vec<Decl>,
}

impl Walker<'_> {
    fn visit(&mut self, node: Node, container: Option<u32>) {
        let mut cursor = node.walk();
        let children: Vec<Node> = node.named_children(&mut cursor).collect();
        for child in children {
            let found = match child.kind() {
                "class_declaration" => child.child_by_field_name("name").map(|n| (n, None, class_kind(self.text, child))),
                "object_declaration" => child.child_by_field_name("name").map(|n| (n, None, DeclKind::Object)),
                "companion_object" => match child.child_by_field_name("name") {
                    Some(n) => Some((n, None, DeclKind::Object)),
                    None => anonymous_child(child, "companion").map(|n| (n, Some("Companion"), DeclKind::Object)),
                },
                "type_alias" => child.child_by_field_name("type").map(|n| (n, None, DeclKind::TypeAlias)),
                "enum_entry" => named_child_of_kind(child, "identifier").map(|n| (n, None, DeclKind::EnumEntry)),
                "ERROR" => {
                    self.visit(child, container);
                    None
                }
                _ => None,
            };
            let Some((name_node, fixed_name, kind)) = found else { continue };
            let index = self.push(name_node, fixed_name, kind, container);
            for body in ["class_body", "enum_class_body"] {
                if let Some(body) = named_child_of_kind(child, body) {
                    self.visit(body, Some(index));
                }
            }
        }
    }

    fn push(&mut self, name_node: Node, fixed_name: Option<&str>, kind: DeclKind, container: Option<u32>) -> u32 {
        let name = fixed_name.map_or_else(|| ident_text(self.text, name_node), str::to_string);
        let prefix = container.map_or(self.package, |c| self.decls[c as usize].fqn.as_str());
        let fqn = qualify(prefix, &name);
        self.decls.push(Decl { name, fqn, kind, container, name_range: span_of(self.text, name_node) });
        (self.decls.len() - 1) as u32
    }
}

fn class_kind(text: &str, node: Node) -> DeclKind {
    if anonymous_child(node, "interface").is_some() {
        return DeclKind::Interface;
    }
    let is_enum = named_child_of_kind(node, "modifiers").is_some_and(|m| {
        let mut cursor = m.walk();
        m.named_children(&mut cursor).any(|c| c.kind() == "class_modifier" && &text[c.byte_range()] == "enum")
    });
    if is_enum { DeclKind::Enum } else { DeclKind::Class }
}

fn anonymous_child<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).find(|c| !c.is_named() && c.kind() == kind)
}
