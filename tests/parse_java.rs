use kotlin_navigator::model::{DeclKind::*, Flags, Import, Lang};
use kotlin_navigator::parse::summarize_text;
use std::path::PathBuf;

#[test]
fn java_declarations_and_imports() {
    let text = "package com.j;\nimport java.util.List;\nimport static com.x.Util.*;\npublic class A {\n  interface B {}\n  enum C { X, Y }\n  record R(int a) {}\n  @interface Ann {}\n}\n";
    let (_, s) = summarize_text(PathBuf::from("/ws/A.java"), text).unwrap();
    assert_eq!(s.lang, Lang::Java);
    assert_eq!(s.package, "com.j");
    assert_eq!(s.imports, vec![
        Import { path: "java.util.List".into(), alias: None, wildcard: false },
        Import { path: "com.x.Util".into(), alias: None, wildcard: true },
    ]);
    let got: Vec<_> = s.decls.iter().map(|d| (d.fqn.as_str(), d.kind)).collect();
    assert_eq!(got, vec![("com.j.A", Class), ("com.j.A.B", Interface), ("com.j.A.C", Enum), ("com.j.A.C.X", EnumEntry),
                         ("com.j.A.C.Y", EnumEntry), ("com.j.A.R", Class), ("com.j.A.R.a", Property), ("com.j.A.Ann", Interface)]);
}

#[test]
fn java_comments_are_ignored() {
    let (_, s) = summarize_text(PathBuf::from("/ws/A.java"), "// class Fake\n/* interface Fake2 */\nclass Real {}\n").unwrap();
    assert_eq!(s.decls.iter().map(|d| d.fqn.as_str()).collect::<Vec<_>>(), vec!["Real"]);
}

#[test]
fn java_members_and_supertypes() {
    let text = "package j;\npublic class J extends Base implements I, a.K<T> {\n  private static final int C = 1, D = 2;\n  Repo repo;\n  public J(int x) {}\n  void load(Repo r) {}\n}\nrecord R(int a, String b) {}\n";
    let (_, s) = summarize_text(PathBuf::from("/ws/J.java"), text).unwrap();
    let got: Vec<_> = s.decls.iter().map(|d| (d.fqn.as_str(), d.kind, d.type_hint.as_deref())).collect();
    assert_eq!(got, vec![("j.J", Class, None), ("j.J.C", Property, None), ("j.J.D", Property, None), ("j.J.repo", Property, Some("Repo")),
        ("j.J.load", Function, None), ("j.R", Class, None), ("j.R.a", Property, None), ("j.R.b", Property, Some("String"))]);
    assert_eq!(s.decls[0].supertypes, vec!["Base", "I", "a.K"]);
    assert!(s.decls[1].flags.contains(Flags::PRIVATE));
}

#[test]
fn java_method_return_types() {
    let text = "package j;\nclass J {\n  <T> T get() { return null; }\n  int n() { return 0; }\n  java.util.List<String> l() { return null; }\n  void v() {}\n  Repo r() { return null; }\n}\n";
    let (_, s) = summarize_text(PathBuf::from("/ws/J.java"), text).unwrap();
    let hints: Vec<_> = s.decls.iter().skip(1).map(|d| (d.name.as_str(), d.type_hint.as_deref())).collect();
    assert_eq!(hints, vec![("get", None), ("n", None), ("l", Some("java.util.List")), ("v", None), ("r", Some("Repo"))]);
}
