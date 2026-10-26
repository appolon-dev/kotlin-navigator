use kotlin_navigator::model::{DeclKind::*, DeclKind, FileSummary, Flags, Import, Span};
use kotlin_navigator::model::Lang;
use kotlin_navigator::parse::{parse_within, source_text, summarize_text};
use std::path::PathBuf;
use std::time::Duration;

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
    assert_eq!(fqns("// class Fake\nval s = \"class Fake2\"\n/* object Fake3 */\nfun f() {\n  class Local\n}\nclass Real\n"), v(&[("s", Property), ("f", Function), ("Real", Class)]));
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
    assert_eq!(fqns(text), v(&[("p.T", Class), ("p.T.A", Object), ("p.T.A.raw", Property), ("p.T.B", Object), ("p.T.B.raw", Property),
        ("p.T.Companion", Object), ("p.T.Companion.f", Function), ("p.After", Class)]));
    let s = summary(text);
    assert_eq!(decl(&s, "p.T.B").name_range, Span { line: 4, start: 16, end: 17 });
    assert_eq!(decl(&s, "p.T.Companion").name_range, Span { line: 5, start: 4, end: 13 });
    assert_eq!(decl(&s, "p.After").name_range, Span { line: 11, start: 6, end: 11 });
}

#[test]
fn whole_file_parsed_as_error_keeps_declarations() {
    // A parameter named after the soft keyword `override` makes the root itself an ERROR node.
    let text = "package p\ninternal object E {\n    fun f(s: Any, override: Any?): Any? = when (s) {\n        is String -> override as? String\n    }\n}\n";
    let s = summary(text);
    assert_eq!(s.package, "p");
    assert_eq!(fqns(text), v(&[("p.E", Object), ("p.E.f", Function)]));
}

#[test]
fn summary_records_source_set() {
    let (_, s) = summarize_text(PathBuf::from("/ws/m/src/iosMain/kotlin/A.kt"), "class A\n").unwrap();
    assert_eq!((s.source_set.as_deref(), s.module.clone()), (Some("iosMain"), None));
}

fn decl<'a>(s: &'a FileSummary, fqn: &str) -> &'a kotlin_navigator::model::Decl {
    s.decls.iter().find(|d| d.fqn == fqn).unwrap_or_else(|| panic!("no {fqn}"))
}

#[test]
fn functions_properties_and_their_details() {
    let text = "package p\nfun top() {}\nfun <T> Repo.ext(a: Int): T = TODO()\nval Repo.extProp: Int get() = 1\n\
private val typed: Map<String, Int>? = null\ninternal val made = Repo(1)\nval lambda: () -> Unit = {}\nexpect fun plat(): String\n\
class VM(private val repo: Repo, plain: Int) : Base(), a.b.Iface<T>, Del by d {\n    override fun load() { val local = 1 }\n\
    companion object { fun create(): VM = TODO() }\n}\n";
    assert_eq!(fqns(text), v(&[("p.top", Function), ("p.ext", Function), ("p.extProp", Property), ("p.typed", Property),
        ("p.made", Property), ("p.lambda", Property), ("p.plat", Function), ("p.VM", Class), ("p.VM.repo", Property),
        ("p.VM.load", Function), ("p.VM.Companion", Object), ("p.VM.Companion.create", Function)]));
    let s = summary(text);
    assert_eq!((decl(&s, "p.ext").receiver.as_deref(), decl(&s, "p.ext").type_hint.as_deref()), (Some("Repo"), None));
    assert_eq!((decl(&s, "p.extProp").receiver.as_deref(), decl(&s, "p.extProp").type_hint.as_deref()), (Some("Repo"), Some("Int")));
    assert_eq!((decl(&s, "p.typed").type_hint.as_deref(), decl(&s, "p.typed").flags), (Some("Map"), Flags::PRIVATE));
    assert_eq!((decl(&s, "p.made").type_hint.as_deref(), decl(&s, "p.made").flags), (Some("Repo"), Flags::INTERNAL | Flags::CALL_HINT));
    assert_eq!(decl(&s, "p.lambda").type_hint, None);
    assert_eq!(decl(&s, "p.plat").flags, Flags::EXPECT);
    assert_eq!(decl(&s, "p.VM").supertypes, vec!["Base", "a.b.Iface", "Del"]);
    assert_eq!((decl(&s, "p.VM.repo").type_hint.as_deref(), decl(&s, "p.VM.repo").flags), (Some("Repo"), Flags::PRIVATE));
    assert_eq!(decl(&s, "p.VM.load").flags, Flags::OVERRIDE);
    assert_eq!(decl(&s, "p.VM.Companion").flags, Flags::COMPANION);
    assert_eq!(decl(&s, "p.VM.load").name_range, Span { line: 9, start: 17, end: 21 });
}

#[test]
fn destructuring_and_qualified_constructor_calls() {
    let s = summary("val (a, b: Repo) = pair\nval x = a.b.Foo(1)\nval y = foo(1)\nval z = a?.Foo()\nval w = Foo::Bar()\n");
    assert_eq!(fqns("val (a, b: Repo) = pair\n"), v(&[("a", Property), ("b", Property)]));
    assert_eq!((decl(&s, "b").type_hint.as_deref(), decl(&s, "b").flags), (Some("Repo"), Flags::default()));
    assert_eq!((decl(&s, "x").type_hint.as_deref(), decl(&s, "x").flags), (Some("a.b.Foo"), Flags::CALL_HINT));
    for name in ["a", "y", "z", "w"] {
        assert_eq!(decl(&s, name).type_hint, None, "{name}");
    }
}

#[test]
fn error_regions_recover_functions_and_properties() {
    // The root is an ERROR (a parameter named `override`). tree-sitter drops the file's tail; names come from the text.
    let text = "package p\ninternal object E {\n    fun f(s: Any, override: Any?): Any? = when (s) {\n        is String -> override as? String\n    }\n    val after = 1\n    fun <T> Repo.ext() {}\n}\n";
    assert_eq!(fqns(text), v(&[("p.E", Object), ("p.E.f", Function), ("p.E.after", Property), ("p.E.ext", Function)]));
    // Enum members after `;` land in an ERROR inside enum_class_body.
    assert_eq!(fqns("package p\nenum class C { Q; fun g() {} }\n"), v(&[("p.C", Enum), ("p.C.Q", EnumEntry), ("p.C.g", Function)]));
}

/// Runs `f` on its own thread; panics when it has not finished within `secs` seconds.
fn within<T: Send + 'static>(secs: u64, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(Duration::from_secs(secs)).expect("finished in time")
}

#[test]
fn annotation_at_the_end_of_input_does_not_hang() {
    let s = within(10, || summary("package a\nval r: Repo? = null\n@"));
    assert_eq!(s.decls[0].name, "r");
}

#[test]
fn runaway_parse_gives_up() {
    // A lone backtick sends the grammar's error recovery into minutes of work.
    let text = include_str!("fixtures/slow_backtick.kt");
    assert!(within(10, move || summarize_text(PathBuf::from("/ws/A.kt"), text).is_none()));
}

#[test]
fn parser_recovers_after_giving_up() {
    let big = "fun f() { g(1, 2) }\n".repeat(5000);
    assert!(parse_within(Lang::Kotlin, &big, Duration::ZERO).is_none());
    assert_eq!(summary("class B\n").decls[0].name, "B");
}

#[test]
fn source_text_ends_with_a_newline() {
    assert_eq!(source_text("class A".to_string()), "class A\n");
    assert_eq!(source_text("class A\n".to_string()), "class A\n");
}

#[test]
fn function_return_types() {
    let s = summary("package p\nfun a(): Repo? = null\nfun <T> b(x: T): T = x\nfun c(): () -> Unit = {}\n\
fun Repo.d(): List<Repo> = TODO()\nfun e() {}\nfun <T> f(): Box<T> = TODO()\n");
    let hints: Vec<_> = s.decls.iter().map(|d| (d.name.as_str(), d.type_hint.as_deref())).collect();
    assert_eq!(hints, vec![("a", Some("Repo")), ("b", None), ("c", None), ("d", Some("List")), ("e", None), ("f", Some("Box"))]);
    assert_eq!(s.decls[3].receiver.as_deref(), Some("Repo"));
}
