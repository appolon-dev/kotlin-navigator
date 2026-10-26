use kotlin_navigator::index::Index;
use kotlin_navigator::parse::summarize_text;
use kotlin_navigator::resolve::{definition, Current};
use kotlin_navigator::scan::build_index;
use std::path::{Path, PathBuf};

const CORE: &str = "core/src/commonMain/kotlin/com/fx/core";
const SCREEN: &str = "app/src/commonMain/kotlin/com/fx/app/Screen.kt";

fn root() -> PathBuf { Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/kmp") }

/// Targets from `needle` on `line` of `rel`, as "relative/path:line".
fn go(rel: &str, line: usize, needle: &str) -> Vec<String> {
    let (index, _) = build_index(&[root()]);
    hits(&index, &root(), &root().join(rel), line, needle)
}
fn hits(index: &Index, base: &Path, path: &Path, line: usize, needle: &str) -> Vec<String> {
    let text = std::fs::read_to_string(path).unwrap();
    let start: usize = text.split_inclusive('\n').take(line).map(str::len).sum();
    let byte = start + text.split_inclusive('\n').nth(line).unwrap().find(needle).unwrap();
    let (tree, parsed) = summarize_text(path.to_path_buf(), &text).unwrap();
    let summary = index.file(path).cloned().unwrap_or(parsed);
    definition(index, &Current { text: &text, tree: &tree, summary: &summary }, byte)
        .iter().map(|t| format!("{}:{}", t.path.strip_prefix(base).unwrap_or(&t.path).display(), t.range.line)).collect()
}
fn at(dir: &str, file: &str, line: u32) -> String { format!("{dir}/{file}:{line}") }

#[test] fn method_via_typed_constructor_property() { assert_eq!(go(SCREEN, 10, "load"), vec![at(CORE, "Repo.kt", 9)]); }
#[test] fn constructor_call_types_a_local() { assert_eq!(go(SCREEN, 12, "size"), vec![at(CORE, "Repo.kt", 7)]); }
#[test] fn class_call_goes_to_the_class() { assert_eq!(go(SCREEN, 11, "Repo"), vec![at(CORE, "Repo.kt", 7)]); }
#[test] fn companion_member_via_class_name() { assert_eq!(go(SCREEN, 13, "create"), vec![at(CORE, "Repo.kt", 13)]); }
#[test] fn extension_on_the_receiver_type() { assert_eq!(go(SCREEN, 14, "extra"), vec![at(CORE, "Ext.kt", 2)]); }
#[test] fn own_override_beats_the_supertype() { assert_eq!(go(SCREEN, 15, "ping"), vec![at(CORE, "Repo.kt", 8)]); }
#[test] fn supertype_member() { assert_eq!(go(SCREEN, 16, "describe"), vec![at(CORE, "Repo.kt", 4)]); }
#[test] fn extension_on_a_supertype() { assert_eq!(go(SCREEN, 17, "baseExt"), vec![at(CORE, "Ext.kt", 3)]); }
#[test] fn private_member_of_another_file_is_not_offered() { assert!(go(SCREEN, 18, "secret").is_empty()); }
#[test] fn parameter_and_local() {
    assert_eq!(go(SCREEN, 19, "count"), vec![format!("{SCREEN}:9")]);
    assert_eq!(go(SCREEN, 20, "local"), vec![format!("{SCREEN}:19")]);
    assert_eq!(go(SCREEN, 12, "made"), vec![format!("{SCREEN}:11")]);
}
#[test] fn plain_class_parameter_in_initializer() { assert_eq!(go(SCREEN, 8, "title"), vec![format!("{SCREEN}:7")]); }
#[test] fn implicit_it_goes_to_its_lambda() { assert_eq!(go(SCREEN, 21, "it"), vec![format!("{SCREEN}:21")]); }
#[test] fn expect_from_common_lists_the_actuals_after_it() {
    assert_eq!(go(SCREEN, 22, "platformName"), vec![at(CORE, "Platform.kt", 2),
        "core/src/androidMain/kotlin/com/fx/core/Platform.android.kt:2".to_string(), "core/src/iosMain/kotlin/com/fx/core/Platform.ios.kt:2".to_string()]);
}
#[test] fn expect_from_android_puts_the_android_actual_first() {
    assert_eq!(go("app/src/androidMain/kotlin/com/fx/app/AndroidScreen.kt", 7, "platformName"), vec![
        "core/src/androidMain/kotlin/com/fx/core/Platform.android.kt:2".to_string(), at(CORE, "Platform.kt", 2),
        "core/src/iosMain/kotlin/com/fx/core/Platform.ios.kt:2".to_string()]);
}
#[test] fn common_test_sees_common_main() {
    assert_eq!(go("core/src/commonTest/kotlin/com/fx/core/PlatformTest.kt", 3, "platformName"), vec![at(CORE, "Platform.kt", 2),
        "core/src/androidMain/kotlin/com/fx/core/Platform.android.kt:2".to_string(), "core/src/iosMain/kotlin/com/fx/core/Platform.ios.kt:2".to_string()]);
}
#[test] fn other_source_set_is_dropped() {
    let android = "app/src/androidMain/kotlin/com/fx/app/AndroidScreen.kt";
    assert_eq!(go(android, 6, "Helper"), vec!["core/src/androidMain/kotlin/com/fx/core/Helper.kt:2"]);
    assert_eq!(go(android, 6, "help"), vec!["core/src/androidMain/kotlin/com/fx/core/Helper.kt:3"]);
}
#[test] fn this_member() { assert_eq!(go(SCREEN, 23, "repo"), vec![format!("{SCREEN}:7")]); }
#[test] fn unknown_receiver_uses_ranked_name_search() { assert_eq!(go(SCREEN, 24, "load"), vec!["app/src/commonMain/kotlin/com/fx/app/Other.kt:3"]); }
#[test] fn safe_call_is_like_a_dot() { assert_eq!(go(SCREEN, 25, "load"), vec![at(CORE, "Repo.kt", 9)]); }
#[test] fn own_member_by_simple_name() { assert_eq!(go(SCREEN, 26, "render"), vec![format!("{SCREEN}:29")]); }
#[test] fn string_templates() {
    assert_eq!(go(SCREEN, 27, "size"), vec![at(CORE, "Repo.kt", 7)]);
    assert_eq!(go(SCREEN, 27, "local"), vec![format!("{SCREEN}:19")]);
}

/// Scenarios without the fixture: one temporary index from (path, text).
fn mem(files: &[(&str, &str)], from: &str, line: usize, needle: &str) -> Vec<String> {
    let mut index = Index::default();
    for (p, t) in files { index.upsert(summarize_text(PathBuf::from(p), t).unwrap().1); }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cur.kt");
    std::fs::write(&path, files.iter().find(|(p, _)| *p == from).unwrap().1).unwrap();
    index.upsert(summarize_text(path.clone(), &std::fs::read_to_string(&path).unwrap()).unwrap().1);
    index.remove(Path::new(from));
    hits(&index, dir.path(), &path, line, needle)
}

#[test] fn local_shadows_the_property_and_gives_its_type() {
    let files = [("/ws/R.kt", "package p\nclass R1 { fun go() {} }\nclass R2 { fun go() {} }\n"),
                 ("/ws/U.kt", "package p\nclass A(val repo: R1) {\n  fun f() {\n    val repo = R2()\n    repo.go()\n  }\n}\n")];
    assert_eq!(mem(&files, "/ws/U.kt", 4, "go"), vec!["/ws/R.kt:2"]);
}
#[test] fn cyclic_supertypes_terminate() {
    let files = [("/ws/U.kt", "package p\nopen class X : Y()\nopen class Y : X()\nfun f(x: X) {\n  x.nothing()\n}\n")];
    assert!(mem(&files, "/ws/U.kt", 4, "nothing").is_empty());
}
#[test] fn declaration_name_resolves_to_itself() {
    let files = [("/ws/O.kt", "package p\nclass O { fun load() {} }\n"), ("/ws/U.kt", "package p\nclass R {\n  fun load() {}\n}\n")];
    assert_eq!(mem(&files, "/ws/U.kt", 2, "load"), vec!["cur.kt:2"]);
}
#[test] fn nested_type_through_its_outer() {
    let files = [("/ws/O.kt", "package p\nclass Outer { class Inner }\n"), ("/ws/Q.kt", "package q\nclass Inner\n"),
                 ("/ws/U.kt", "package u\nimport p.Outer\nval i: Outer.Inner? = null\n")];
    assert_eq!(mem(&files, "/ws/U.kt", 2, "Inner"), vec!["/ws/O.kt:1"]);
}
#[test] fn call_prefers_the_function() {
    let files = [("/ws/U.kt", "package p\nval format = 1\nfun format() {}\nfun g() { format() }\n")];
    assert_eq!(mem(&files, "/ws/U.kt", 3, "format"), vec!["cur.kt:2"]);
}
