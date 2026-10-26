use kotlin_navigator::index::Index;
use kotlin_navigator::parse::summarize_text;
use kotlin_navigator::resolve::typedef::type_definition;
use kotlin_navigator::resolve::{resolve, target_at, Certainty::{self, *}, Current, Found};
use kotlin_navigator::scan::build_index;
use kotlin_navigator::sources::Sources;
use std::path::{Path, PathBuf};

const SCREEN: &str = "app/src/commonMain/kotlin/com/fx/app/Screen.kt";

fn root() -> PathBuf { Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/kmp") }
fn v(items: &[&str]) -> Vec<String> { items.iter().map(|s| s.to_string()).collect() }
fn byte_of(text: &str, line: usize, needle: &str) -> usize {
    let start: usize = text.split_inclusive('\n').take(line).map(str::len).sum();
    start + text.split_inclusive('\n').nth(line).unwrap().find(needle).unwrap()
}
/// Targets as "File.kt:line", with " local" for declarations the index does not hold.
fn show(found: &[Found]) -> Vec<String> {
    found.iter().map(|f| {
        let t = f.target();
        let local = if matches!(f, Found::Local(_)) { " local" } else { "" };
        format!("{}:{}{local}", t.path.file_name().unwrap().to_string_lossy(), t.range.line)
    }).collect()
}
/// How the reference at `needle` on `line` of fixture file `rel` resolves.
fn how(rel: &str, line: usize, needle: &str) -> Option<(Certainty, Vec<String>)> {
    let (index, _) = build_index(&[root()]);
    let sources = Sources::default();
    let doc = sources.load(&root().join(rel)).unwrap();
    let current = Current::of(&index, &doc);
    resolve(&index, &current, &sources, byte_of(&doc.text, line, needle)).map(|r| (r.certainty, show(&r.found)))
}
/// Like `how`, through `target_at`.
fn target(rel: &str, line: usize, needle: &str) -> Option<(Certainty, Vec<String>)> {
    let (index, _) = build_index(&[root()]);
    let sources = Sources::default();
    let doc = sources.load(&root().join(rel)).unwrap();
    let current = Current::of(&index, &doc);
    target_at(&index, &current, &sources, byte_of(&doc.text, line, needle)).map(|r| (r.certainty, show(&r.found)))
}
/// The same for files that exist only in memory.
fn mem(files: &[(&str, &str)], from: &str, line: usize, needle: &str) -> Option<(Certainty, Vec<String>)> {
    let mut index = Index::default();
    for (p, t) in files { index.upsert(summarize_text(PathBuf::from(p), t).unwrap().1); }
    let text = files.iter().find(|(p, _)| *p == from).unwrap().1;
    let (tree, summary) = summarize_text(PathBuf::from(from), text).unwrap();
    let current = Current { text, tree: &tree, summary: &summary };
    resolve(&index, &current, &Sources::default(), byte_of(text, line, needle)).map(|r| (r.certainty, show(&r.found)))
}

#[test]
fn reliable_paths_are_exact() {
    assert_eq!(how(SCREEN, 2, "Repo"), Some((Exact, v(&["Repo.kt:7"]))));        // import line
    assert_eq!(how(SCREEN, 10, "load"), Some((Exact, v(&["Repo.kt:9"]))));       // typed property
    assert_eq!(how(SCREEN, 11, "Repo"), Some((Exact, v(&["Repo.kt:7"]))));       // explicit import
    assert_eq!(how(SCREEN, 13, "create"), Some((Exact, v(&["Repo.kt:13"]))));    // class name
    assert_eq!(how(SCREEN, 14, "extra"), Some((Exact, v(&["Ext.kt:2"]))));       // extension by receiver type
    assert_eq!(how(SCREEN, 19, "count"), Some((Exact, v(&["Screen.kt:9 local"]))));
    assert_eq!(how(SCREEN, 12, "made"), Some((Exact, v(&["Screen.kt:11 local"]))));
    assert_eq!(how(SCREEN, 26, "render"), Some((Exact, v(&["Screen.kt:29"]))));  // implicit receiver
}

#[test]
fn name_search_is_probable() {
    assert_eq!(how(SCREEN, 24, "load"), Some((Probable, v(&["Other.kt:3"]))));
    assert_eq!(how(SCREEN, 24, "unknownThing"), Some((Probable, vec![])));
    let files = [("/ws/a/U.kt", "package a\nfun f(s: String) { s.length }\n")];
    assert_eq!(mem(&files, "/ws/a/U.kt", 1, "length"), Some((Probable, vec![]))); // type not in the project
    assert_eq!(mem(&files, "/ws/a/U.kt", 1, " {"), None);
}

#[test]
fn a_member_missing_from_a_type_that_leaves_the_project_is_probable() {
    let files = [("/ws/p/Toast.kt", "package p\nimport android.content.Context\nfun Context.toast() {}\n"),
                 ("/ws/p/MyActivity.kt", "package p\nimport androidx.appcompat.app.AppCompatActivity\nclass MyActivity : AppCompatActivity() {\n  fun f() {\n    this.toast()\n  }\n}\nfun use(a: MyActivity) {\n  a.toast()\n}\n")];
    // Nothing to go to, as before, but the library supertype may have it.
    assert_eq!(mem(&files, "/ws/p/MyActivity.kt", 8, "toast"), Some((Probable, vec![])));
    assert_eq!(mem(&files, "/ws/p/MyActivity.kt", 4, "toast"), Some((Probable, vec![])));
    let known = [("/ws/p/A.kt", "package p\nopen class Base\nclass A : Base()\nfun use(a: A) {\n  a.toast()\n}\n")];
    assert_eq!(mem(&known, "/ws/p/A.kt", 4, "toast"), Some((Exact, vec![]))); // the whole chain is in the project
}

#[test]
fn top_level_without_a_matching_level_is_probable() {
    let other = [("/ws/b/Repo.kt", "package b\nclass Repo\n"), ("/ws/c/Repo.kt", "package c\nclass Repo\n"), ("/ws/a/Use.kt", "package a\nval r: Repo? = null\n")];
    assert_eq!(mem(&other, "/ws/a/Use.kt", 1, "Repo"), Some((Probable, v(&["Repo.kt:1", "Repo.kt:1"]))));
    let same = [("/ws/a/Repo.kt", "package a\nclass Repo\n"), ("/ws/a/Use.kt", "package a\nval r: Repo? = null\n")];
    assert_eq!(mem(&same, "/ws/a/Use.kt", 1, "Repo"), Some((Exact, v(&["Repo.kt:1"]))));
}

#[test]
fn java_is_probable_except_imports() {
    let files = [("/ws/k/Repo.kt", "package k\nclass Repo\n"), ("/ws/j/Use.java", "package j;\nimport k.Repo;\nclass Use { Repo r; }\n")];
    assert_eq!(mem(&files, "/ws/j/Use.java", 2, "Repo"), Some((Probable, v(&["Repo.kt:1"]))));
    assert_eq!(mem(&files, "/ws/j/Use.java", 1, "Repo"), Some((Exact, v(&["Repo.kt:1"]))));
}

const USAGES: &str = "app/src/commonMain/kotlin/com/fx/app/Usages.kt";

#[test]
fn label_goes_to_the_parameter() {
    assert_eq!(how(USAGES, 11, "limit"), Some((Exact, v(&["Repo.kt:9 local"]))));
    assert_eq!(how(USAGES, 10, "size"), Some((Exact, v(&["Repo.kt:7"])))); // `val` parameter: the property
}

#[test]
fn labels_of_local_functions_constructors_and_missing_parameters() {
    let files = [("/ws/a/U.kt", "package a\nclass P(plain: Int)\nfun f(a: Int) {}\nfun g() {\n  fun inner(x: Int) {}\n  inner(x = 1); P(plain = 2); f(b = 3); f(a = 4)\n}\n")];
    assert_eq!(mem(&files, "/ws/a/U.kt", 5, "x ="), Some((Exact, v(&["U.kt:4 local"]))));
    assert_eq!(mem(&files, "/ws/a/U.kt", 5, "plain ="), Some((Exact, v(&["U.kt:1 local"]))));
    assert_eq!(mem(&files, "/ws/a/U.kt", 5, "a ="), Some((Exact, v(&["U.kt:2 local"]))));
    assert_eq!(mem(&files, "/ws/a/U.kt", 5, "b ="), Some((Exact, vec![])));
}

#[test]
fn parameters_are_not_seen_before_their_list() {
    let fun = [("/ws/p/T.kt", "package p\nclass Label\ninterface T {\n  fun title(title: Label): T\n}\n")];
    assert_eq!(mem(&fun, "/ws/p/T.kt", 3, "title"), Some((Exact, v(&["T.kt:3"]))));
    assert_eq!(mem(&fun, "/ws/p/T.kt", 3, "title:"), Some((Exact, v(&["T.kt:3 local"]))));
    assert_eq!(mem_type(&fun, "/ws/p/T.kt", 3, "title"), ["T.kt:2"]); // the result type, not the parameter's
    let class = [("/ws/p/C.kt", "package p\nconst val tag = \"t\"\n@Named(tag) class C(tag: String) {\n  val t = tag\n}\n")];
    assert_eq!(mem(&class, "/ws/p/C.kt", 2, "tag"), Some((Exact, v(&["C.kt:1"]))));
    assert_eq!(mem(&class, "/ws/p/C.kt", 3, "tag"), Some((Exact, v(&["C.kt:2 local"]))));
}

#[test]
fn on_a_declaration_the_target_is_that_declaration() {
    let platform = "core/src/commonMain/kotlin/com/fx/core/Platform.kt";
    assert_eq!(how(platform, 2, "platformName").map(|(_, t)| t.len()), Some(3)); // definition lists the actuals too
    assert_eq!(target(platform, 2, "platformName"), Some((Exact, v(&["Platform.kt:2"]))));
    assert_eq!(target(SCREEN, 10, "load"), Some((Exact, v(&["Repo.kt:9"]))));
}

fn file_lines(found: Vec<kotlin_navigator::resolve::Target>) -> Vec<String> {
    found.iter().map(|t| format!("{}:{}", t.path.file_name().unwrap().to_string_lossy(), t.range.line)).collect()
}
fn type_of(rel: &str, line: usize, needle: &str) -> Vec<String> {
    let (index, _) = build_index(&[root()]);
    let sources = Sources::default();
    let doc = sources.load(&root().join(rel)).unwrap();
    let current = Current::of(&index, &doc);
    file_lines(type_definition(&index, &current, &sources, byte_of(&doc.text, line, needle)))
}
fn mem_type(files: &[(&str, &str)], from: &str, line: usize, needle: &str) -> Vec<String> {
    let mut index = Index::default();
    for (p, t) in files { index.upsert(summarize_text(PathBuf::from(p), t).unwrap().1); }
    let text = files.iter().find(|(p, _)| *p == from).unwrap().1;
    let (tree, summary) = summarize_text(PathBuf::from(from), text).unwrap();
    let current = Current { text, tree: &tree, summary: &summary };
    file_lines(type_definition(&index, &current, &Sources::default(), byte_of(text, line, needle)))
}

#[test]
fn type_of_variables_calls_and_classes() {
    assert_eq!(type_of(SCREEN, 12, "made"), ["Repo.kt:7"]);    // local typed by a constructor call
    assert_eq!(type_of(SCREEN, 23, "repo"), ["Repo.kt:7"]);    // property
    assert_eq!(type_of(USAGES, 7, "base"), ["Repo.kt:2"]);     // parameter
    assert_eq!(type_of(SCREEN, 13, "create"), ["Repo.kt:7"]);  // function result
    assert_eq!(type_of(SCREEN, 11, "Repo"), ["Repo.kt:7"]);    // a class is its own type
    assert!(type_of(SCREEN, 19, "count").is_empty());          // Int is not in the project
}

#[test]
fn type_parameters_and_generic_arguments_lead_nowhere() {
    let files = [("/ws/a/U.kt", "package a\nclass Box\nfun <T> pick(): T = TODO()\nfun g(xs: List<Box>, b: Box) {\n  pick<Int>()\n  xs\n  b\n}\n")];
    assert!(mem_type(&files, "/ws/a/U.kt", 4, "pick").is_empty());
    assert!(mem_type(&files, "/ws/a/U.kt", 5, "xs").is_empty()); // List<Box> leads to List, not to Box
    assert_eq!(mem_type(&files, "/ws/a/U.kt", 6, "b"), ["U.kt:1"]);
}

#[test]
fn type_definition_respects_the_declarations_nested_scope() {
    let files = [("/ws/R.kt", "package p\nclass Right\n"),
                 ("/ws/U.kt", "package p\nclass Outer {\n  class Right\n  val item: Right? = null\n  fun f(x: Right) { x }\n  fun make(): Right = TODO()\n}\nfun use(o: Outer) { o.item; o.make() }\n")];
    assert_eq!(mem_type(&files, "/ws/U.kt", 4, "x }"), ["U.kt:2"]);
    assert_eq!(mem_type(&files, "/ws/U.kt", 7, "item"), ["U.kt:2"]);
    assert_eq!(mem_type(&files, "/ws/U.kt", 7, "make"), ["U.kt:2"]);
}
