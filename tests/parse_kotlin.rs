use kotlin_navigator::model::{DeclKind::*, DeclKind, FileSummary, Import, Span};
use kotlin_navigator::parse::summarize_text;
use std::path::PathBuf;

fn summary(text: &str) -> FileSummary {
    summarize_text(PathBuf::from("/ws/A.kt"), text).unwrap().1
}
fn fqns(text: &str) -> Vec<(String, DeclKind)> {
    summary(text).decls.into_iter().map(|d| (d.fqn, d.kind)).collect()
}
fn v(items: &[(&str, DeclKind)]) -> Vec<(String, DeclKind)> {
    items.iter().map(|(f, k)| (f.to_string(), *k)).collect()
}

#[test]
fn class_like_declarations() {
    let text = "package com.a\ninternal data class Wallet {}\nfun interface Clicker {}\nobject Single\ntypealias Handler = () -> Unit\n";
    assert_eq!(fqns(text), v(&[("com.a.Wallet", Class), ("com.a.Clicker", Interface), ("com.a.Single", Object), ("com.a.Handler", TypeAlias)]));
    assert_eq!(summary(text).package, "com.a");
}

#[test]
fn nested_companion_and_enum() {
    let text = "package p\nclass Outer {\n  class Inner\n  companion object {}\n}\nenum class Color { RED, GREEN }\n";
    assert_eq!(fqns(text), v(&[("p.Outer", Class), ("p.Outer.Inner", Class), ("p.Outer.Companion", Object),
                               ("p.Color", Enum), ("p.Color.RED", EnumEntry), ("p.Color.GREEN", EnumEntry)]));
    assert_eq!(summary(text).decls[1].container, Some(0));
}

#[test]
fn comments_strings_and_locals_are_not_indexed() {
    assert_eq!(fqns("// class Fake\nval s = \"class Fake2\"\n/* object Fake3 */\nfun f() {\n  class Local\n}\nclass Real\n"), v(&[("Real", Class)]));
}

#[test]
fn imports_alias_wildcard_nested() {
    assert_eq!(summary("import com.foo.RealName as Alias\nimport com.bar.*\nimport a.Outer.Inner\n").imports, vec![
        Import { path: "com.foo.RealName".into(), alias: Some("Alias".into()), wildcard: false },
        Import { path: "com.bar".into(), alias: None, wildcard: true },
        Import { path: "a.Outer.Inner".into(), alias: None, wildcard: false },
    ]);
}

#[test]
fn name_range_is_utf16_and_backticks_are_stripped() {
    let s = summary("/*😀*/ class `Foo`\n");
    assert_eq!(s.decls[0].name, "Foo");
    assert_eq!(s.decls[0].name_range, Span { line: 0, start: 13, end: 18 });
}

#[test]
fn crlf_lines() {
    assert_eq!(summary("package p\r\n\r\nclass A\r\n").decls[0].name_range, Span { line: 2, start: 6, end: 7 });
}

#[test]
fn broken_syntax_keeps_recognized_decls() {
    let s = summary("class Good {}\nclass Broken {\n  fun (\n");
    assert!(s.has_errors);
    assert!(s.decls.iter().any(|d| d.fqn == "Good"));
}
