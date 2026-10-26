//! Java summaries for the `tree-sitter-java` grammar (`declaration` and
//! `statement` are supertypes, so declarations are direct children of bodies).

use std::path::PathBuf;

use tree_sitter::{Node, Tree};

use super::{DeclSink, dotted_text};
use crate::model::{DeclKind, FileSummary, Import, Lang};
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
            "enum_body_declarations" | "ERROR" => {
                visit(sink, child, container);
                continue;
            }
            _ => continue,
        };
        let Some(name) = child.child_by_field_name("name") else { continue };
        let index = sink.push(name, None, kind, container);
        if kind != DeclKind::EnumEntry {
            if let Some(body) = child.child_by_field_name("body") {
                visit(sink, body, Some(index));
            }
        }
    }
}
