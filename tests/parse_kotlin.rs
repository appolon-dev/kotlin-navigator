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

#[test]
fn annotated_top_level_soft_keyword_declarations() {
    // tree-sitter-kotlin-ng reads these as `annotated_expression` without any ERROR node.
    let text = "package p\n@Retention(AnnotationRetention.BINARY)\nannotation class A\n@Serializable\ndata object D\n@Serializable\nenum class E { X }\n";
    assert_eq!(fqns(text), v(&[("p.A", Class), ("p.D", Object), ("p.E", Enum), ("p.E.X", EnumEntry)]));
    assert_eq!(summary(text).decls[0].name_range, Span { line: 2, start: 17, end: 18 });
}

#[test]
fn grammar_error_inside_class_body_keeps_class_and_fqns() {
    // Two `data object X : T() { override ... }` members in a row make the grammar emit one flat ERROR node.
    let text = "package p\nsealed class T {\n    data object A : T() { override val raw = \"a\" }\n\n    data object B : T() { override val raw = \"b\" }\n    companion object {\n        fun f() {\n            class Local\n        }\n    }\n}\nclass After\n";
    assert_eq!(fqns(text), v(&[("p.T", Class), ("p.T.A", Object), ("p.T.B", Object), ("p.T.Companion", Object), ("p.After", Class)]));
    let spans: Vec<Span> = summary(text).decls.iter().map(|d| d.name_range).collect();
    assert_eq!(spans[2..], [Span { line: 4, start: 16, end: 17 }, Span { line: 5, start: 4, end: 13 }, Span { line: 11, start: 6, end: 11 }]);
}

#[test]
fn whole_file_parsed_as_error_keeps_declarations() {
    // A parameter named after the soft keyword `override` makes the root itself an ERROR node.
    let text = "package p\ninternal object E {\n    fun f(s: Any, override: Any?): Any? = when (s) {\n        is String -> override as? String\n    }\n}\n";
    let s = summary(text);
    assert_eq!(s.package, "p");
    assert_eq!(fqns(text), v(&[("p.E", Object)]));
}

#[test]
fn summary_records_source_set() {
    let (_, s) = summarize_text(PathBuf::from("/ws/m/src/iosMain/kotlin/A.kt"), "class A\n").unwrap();
    assert_eq!((s.source_set.as_deref(), s.module.clone()), (Some("iosMain"), None));
}
