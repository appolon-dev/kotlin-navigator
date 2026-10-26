use kotlin_navigator::cancel::{Cancel, Cancelled};
use kotlin_navigator::index::Index;
use kotlin_navigator::model::Flags;
use kotlin_navigator::parse::summarize_text;
use kotlin_navigator::resolve::family::{family, implementation, implementations, subtypes};
use kotlin_navigator::resolve::rank::DeclRef;
use kotlin_navigator::resolve::Current;
use kotlin_navigator::scan::build_index;
use kotlin_navigator::sources::Sources;
use std::path::{Path, PathBuf};

fn root() -> PathBuf { Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/kmp") }
fn fixture() -> Index { build_index(&[root()]).0 }
fn decl<'a>(index: &'a Index, fqn: &str) -> DeclRef<'a> {
    index.decls().find(|(_, d)| d.fqn == fqn).unwrap_or_else(|| panic!("no {fqn}"))
}
fn names(list: Vec<DeclRef>) -> Vec<String> {
    list.iter().map(|(f, d)| format!("{}:{}", f.path.file_name().unwrap().to_string_lossy(), d.name_range.line)).collect()
}
fn lines(list: Vec<DeclRef>) -> Vec<u32> { list.iter().map(|(_, d)| d.name_range.line).collect() }
fn byte_of(text: &str, line: usize, needle: &str) -> usize {
    let start: usize = text.split_inclusive('\n').take(line).map(str::len).sum();
    start + text.split_inclusive('\n').nth(line).unwrap().find(needle).unwrap()
}
fn impls(rel: &str, line: usize, needle: &str) -> Vec<String> {
    let index = fixture();
    let sources = Sources::default();
    let doc = sources.load(&root().join(rel)).unwrap();
    let current = Current::of(&index, &doc);
    implementation(&index, &current, &sources, byte_of(&doc.text, line, needle), &Cancel::default()).unwrap()
        .iter().map(|t| format!("{}:{}", t.path.file_name().unwrap().to_string_lossy(), t.range.line)).collect()
}
/// The same over files that exist only in memory.
fn mem_impls(files: &[(&str, &str)], from: &str, line: usize, needle: &str) -> Vec<String> {
    let mut index = Index::default();
    for (p, t) in files { index.upsert(summarize_text(PathBuf::from(p), t).unwrap().1); }
    let text = files.iter().find(|(p, _)| *p == from).unwrap().1;
    let (tree, summary) = summarize_text(PathBuf::from(from), text).unwrap();
    let current = Current { text, tree: &tree, summary: &summary };
    implementation(&index, &current, &Sources::default(), byte_of(text, line, needle), &Cancel::default()).unwrap()
        .iter().map(|t| format!("{}:{}", t.path.file_name().unwrap().to_string_lossy(), t.range.line)).collect()
}

#[test]
fn subtypes_are_direct_and_indirect() {
    let index = fixture();
    assert_eq!(names(subtypes(&index, decl(&index, "com.fx.core.Base"), &Cancel::default()).unwrap()), ["Cache.kt:2", "Repo.kt:7"]);
}

#[test]
fn family_goes_up_and_down_the_overrides() {
    let index = fixture();
    let none = Cancel::default();
    assert_eq!(names(family(&index, decl(&index, "com.fx.core.Repo.ping"), &none).unwrap()), ["Cache.kt:3", "Repo.kt:3", "Repo.kt:8"]);
    assert_eq!(names(family(&index, decl(&index, "com.fx.core.Repo.load"), &none).unwrap()), ["Repo.kt:9"]);
    assert_eq!(names(family(&index, decl(&index, "com.fx.core.Repo"), &none).unwrap()), ["Repo.kt:7"]); // subtypes are not family
}

#[test]
fn expect_and_its_actuals_are_one_family() {
    let index = fixture();
    let none = Cancel::default();
    let expect = index.decls().find(|(_, d)| d.name == "platformName" && d.flags.contains(Flags::EXPECT)).unwrap();
    assert_eq!(names(family(&index, expect, &none).unwrap()), ["Platform.android.kt:2", "Platform.kt:2", "Platform.ios.kt:2"]);
    assert_eq!(names(implementations(&index, expect, &none).unwrap()), ["Platform.android.kt:2", "Platform.ios.kt:2"]);
}

#[test]
fn implementations_of_classes_and_members() {
    let index = fixture();
    let none = Cancel::default();
    assert_eq!(names(implementations(&index, decl(&index, "com.fx.core.Base"), &none).unwrap()), ["Cache.kt:2", "Repo.kt:7"]);
    assert_eq!(names(implementations(&index, decl(&index, "com.fx.core.Base.ping"), &none).unwrap()), ["Cache.kt:3", "Repo.kt:8"]);
    assert!(implementations(&index, decl(&index, "com.fx.core.Repo.load"), &none).unwrap().is_empty());
}

#[test]
fn deep_and_cyclic_hierarchies() {
    let mut index = Index::default();
    let text = "package a\ninterface I {\n  fun run()\n}\nopen class A : I\nopen class B : A() {\n  override fun run() {}\n}\nclass C : B()\nopen class X : Y()\nopen class Y : X()\n";
    index.upsert(summarize_text(PathBuf::from("/ws/a/H.kt"), text).unwrap().1);
    let none = Cancel::default();
    assert_eq!(lines(subtypes(&index, decl(&index, "a.I"), &none).unwrap()), [4, 5, 8]);
    assert_eq!(lines(implementations(&index, decl(&index, "a.I.run"), &none).unwrap()), [6]);
    assert_eq!(lines(subtypes(&index, decl(&index, "a.X"), &none).unwrap()), [10]);
}

#[test]
fn implementation_from_a_reference() {
    assert_eq!(impls("app/src/commonMain/kotlin/com/fx/app/Usages.kt", 7, "ping"), ["Cache.kt:3", "Repo.kt:8"]);
    assert_eq!(impls("app/src/commonMain/kotlin/com/fx/app/Screen.kt", 11, "Repo"), ["Cache.kt:2"]);
}

#[test]
fn implementation_of_a_function_named_like_its_parameter() {
    let files = [("/ws/p/T.kt", "package p\ninterface T {\n  fun title(title: String): T\n}\nclass B : T {\n  override fun title(title: String): T = this\n}\n")];
    assert_eq!(mem_impls(&files, "/ws/p/T.kt", 2, "title"), ["T.kt:5"]);
}

#[test]
fn members_of_an_expect_class_are_implemented_by_their_actuals() {
    let files = [("/ws/core/src/commonMain/kotlin/p/P.kt", "package p\nexpect class P {\n  fun ping()\n}\n"),
                 ("/ws/core/src/androidMain/kotlin/p/P.android.kt", "package p\nactual class P {\n  actual fun ping() {}\n}\n"),
                 ("/ws/core/src/iosMain/kotlin/p/P.ios.kt", "package p\nactual class P {\n  actual fun ping() {}\n}\n")];
    assert_eq!(mem_impls(&files, "/ws/core/src/commonMain/kotlin/p/P.kt", 2, "ping"), ["P.android.kt:2", "P.ios.kt:2"]);
}

#[test]
fn a_set_flag_stops_the_walk() {
    let index = fixture();
    let cancel = Cancel::default();
    cancel.clone().set();
    assert_eq!(subtypes(&index, decl(&index, "com.fx.core.Base"), &cancel), Err(Cancelled));
}
