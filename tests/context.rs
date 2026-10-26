use kotlin_navigator::parse::summarize_text;
use kotlin_navigator::resolve::context::{reference_at, Receiver::{self, *}, Role::{self, *}};
use std::path::PathBuf;

/// The context of `needle` on `line`; the cursor is on the first byte of `needle`.
fn at(path: &str, text: &str, line: usize, needle: &str) -> Option<(String, Role, Receiver)> {
    let (tree, _) = summarize_text(PathBuf::from(path), text).unwrap();
    let start: usize = text.split_inclusive('\n').take(line).map(str::len).sum();
    let byte = start + text.split_inclusive('\n').nth(line).unwrap().find(needle).unwrap();
    reference_at(&tree, text, byte).map(|r| (r.name, r.role, r.receiver))
}
fn kt(line: usize, needle: &str) -> Option<(String, Role, Receiver)> {
    let text = "package a.b\nimport x.y.Repo\nfun f(repo: Repo) {\n  repo.load(); this.repo; super.ping(); Foo.create(); a.b.Foo.create()\n  foo().bar; repo?.load(); ::load; Foo::bar\n  val v: Outer.Inner = g(count = 1)\n  \"$v ${repo.size}\"; xs.map { it }\n}\n";
    at("/ws/A.kt", text, line, needle)
}
fn p(s: &str) -> Receiver { Path(s.into()) }

#[test]
fn kotlin_roles_and_receivers() {
    assert_eq!(kt(3, "load"), Some(("load".into(), Call, p("repo"))));
    assert_eq!(kt(3, "repo;"), Some(("repo".into(), Expr, This)));
    assert_eq!(kt(3, "ping"), Some(("ping".into(), Call, Super)));
    assert_eq!(kt(3, "create(); a"), Some(("create".into(), Call, p("Foo"))));
    assert_eq!(kt(3, "create()\n"), Some(("create".into(), Call, p("a.b.Foo")))); // the line ends with '\n': the last create
    assert_eq!(kt(4, "bar;"), Some(("bar".into(), Expr, Unknown)));
    assert_eq!(kt(4, "load(); ::"), Some(("load".into(), Call, p("repo"))));
    assert_eq!(kt(4, "load; Foo"), Some(("load".into(), CallableRef, Receiver::None)));
    assert_eq!(kt(4, "bar\n"), Some(("bar".into(), CallableRef, p("Foo"))));
    assert_eq!(kt(5, "Inner"), Some(("Inner".into(), Type, p("Outer"))));
    assert_eq!(kt(5, "Outer"), Some(("Outer".into(), Type, Receiver::None)));
    assert_eq!(kt(5, "count"), Some(("count".into(), Argument, Receiver::None))); // named argument label
    assert_eq!(kt(6, "v "), Some(("v".into(), Expr, Receiver::None)));  // "$v": string_content after "$"
    assert_eq!(kt(6, "size"), Some(("size".into(), Expr, p("repo"))));
    assert_eq!(kt(6, "it"), Some(("it".into(), Expr, Receiver::None)));
    assert_eq!(kt(1, "Repo"), Some(("Repo".into(), Import("x.y.Repo".into()), Receiver::None)));
    assert_eq!(kt(0, "b"), Option::None); // the package line
}

#[test]
fn java_member_receivers_are_unknown() {
    let text = "package j;\nclass U { Repo r; void f() { r.load(); r.size; } }\n";
    assert_eq!(at("/ws/U.java", text, 1, "Repo"), Some(("Repo".into(), Type, Receiver::None)));
    assert_eq!(at("/ws/U.java", text, 1, "load"), Some(("load".into(), Call, Unknown)));
    assert_eq!(at("/ws/U.java", text, 1, "size"), Some(("size".into(), Expr, Unknown)));
}
