use kotlin_navigator::model::{DeclKind::*, Import, Lang};
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
                         ("com.j.A.C.Y", EnumEntry), ("com.j.A.R", Class), ("com.j.A.Ann", Interface)]);
}

#[test]
fn java_comments_are_ignored() {
    let (_, s) = summarize_text(PathBuf::from("/ws/A.java"), "// class Fake\n/* interface Fake2 */\nclass Real {}\n").unwrap();
    assert_eq!(s.decls.iter().map(|d| d.fqn.as_str()).collect::<Vec<_>>(), vec!["Real"]);
}
