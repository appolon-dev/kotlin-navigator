use kotlin_navigator::cancel::{Cancel, Cancelled};
use kotlin_navigator::resolve::{Certainty, Current};
use kotlin_navigator::scan::build_index;
use kotlin_navigator::sources::Sources;
use kotlin_navigator::usages::references;
use std::path::{Path, PathBuf};

const SCREEN: &str = "app/src/commonMain/kotlin/com/fx/app/Screen.kt";
const USAGES: &str = "app/src/commonMain/kotlin/com/fx/app/Usages.kt";
const REPO: &str = "core/src/commonMain/kotlin/com/fx/core/Repo.kt";
const TAGS: &str = "core/src/commonMain/kotlin/com/fx/core/Tags.kt";
const PLATFORM: &str = "core/src/commonMain/kotlin/com/fx/core/Platform.kt";

fn root() -> PathBuf { Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/kmp") }
fn v(items: &[&str]) -> Vec<String> { items.iter().map(|s| s.to_string()).collect() }
fn byte_of(text: &str, line: usize, needle: &str) -> usize {
    let start: usize = text.split_inclusive('\n').take(line).map(str::len).sum();
    start + text.split_inclusive('\n').nth(line).unwrap().find(needle).unwrap()
}
/// Usages of the symbol at `needle` on `line` of `rel` under `base`, as "File.kt:line:col E|P".
fn uses_under(base: &Path, rel: &str, line: usize, needle: &str, include_declaration: bool) -> Vec<String> {
    let (index, _) = build_index(&[base.to_path_buf()]);
    let sources = Sources::default();
    let doc = sources.load(&base.join(rel)).unwrap();
    let current = Current::of(&index, &doc);
    let found = references(&index, &current, &sources, byte_of(&doc.text, line, needle), include_declaration, &Cancel::default()).unwrap();
    found.iter().map(|u| {
        let mark = if u.certainty == Certainty::Exact { "E" } else { "P" };
        format!("{}:{}:{} {mark}", u.path.file_name().unwrap().to_string_lossy(), u.range.line, u.range.start)
    }).collect()
}
fn uses(rel: &str, line: usize, needle: &str, include_declaration: bool) -> Vec<String> {
    uses_under(&root(), rel, line, needle, include_declaration)
}
/// The same over files written to a temporary workspace.
fn uses_in(files: &[(&str, &str)], rel: &str, line: usize, needle: &str, include_declaration: bool) -> Vec<String> {
    let dir = tempfile::tempdir().unwrap();
    for (p, t) in files {
        let path = dir.path().join(p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, t).unwrap();
    }
    uses_under(dir.path(), rel, line, needle, include_declaration)
}

#[test]
fn method_usages_include_overrides_and_calls_through_the_base() {
    assert_eq!(uses(REPO, 8, "ping", false), v(&["Screen.kt:15:13 E", "Usages.kt:7:9 E", "Usages.kt:8:9 E", "Usages.kt:9:12 P", "Cache.kt:3:17 E", "Repo.kt:3:8 E"]));
    assert!(uses(REPO, 8, "ping", true).contains(&"Repo.kt:8:17 E".to_string()));
}

#[test]
fn calls_found_by_name_are_probable() {
    assert_eq!(uses(SCREEN, 10, "load", false), v(&["JavaUse.java:5:38 P", "Screen.kt:10:13 E", "Screen.kt:24:23 P", "Screen.kt:25:14 E", "Usages.kt:11:9 E"]));
}

#[test]
fn property_usages_include_templates_labels_and_typed_locals() {
    assert_eq!(uses(REPO, 7, "size", false), v(&["Screen.kt:12:13 E", "Screen.kt:27:16 E", "Usages.kt:10:20 E", "Usages.kt:12:21 E", "Repo.kt:9:36 E"]));
}

#[test]
fn local_usages_stay_in_their_file() {
    assert_eq!(uses(SCREEN, 19, "local", false), v(&["Screen.kt:20:8 E", "Screen.kt:27:23 E"]));
}

#[test]
fn private_and_internal_limit_the_files() {
    assert_eq!(uses(TAGS, 3, "hidden", false), v(&["Tags.kt:5:31 E"]));
    assert_eq!(uses(TAGS, 2, "tag", false), v(&["Tags.kt:5:23 E"]));
    assert_eq!(uses(USAGES, 18, "tag", false), v(&["Usages.kt:14:12 E", "Usages.kt:15:22 P"]));
}

#[test]
fn expect_usages_cover_every_source_set() {
    assert_eq!(uses(PLATFORM, 2, "platformName", false), v(&["AndroidScreen.kt:3:19 E", "AndroidScreen.kt:7:4 E", "Screen.kt:5:19 E",
        "Screen.kt:22:8 E", "Platform.android.kt:2:11 E", "PlatformTest.kt:3:4 E", "Platform.ios.kt:2:11 E"]));
}

#[test]
fn java_usages_of_a_kotlin_class() {
    let found = uses(REPO, 7, "Repo", false);
    assert!(found.contains(&"JavaUse.java:2:19 E".to_string()));
    assert!(found.contains(&"JavaUse.java:5:13 P".to_string()));
}

#[test]
fn a_shadowing_local_is_not_a_property_usage() {
    let files = [("p/A.kt", "package p\nclass A(val repo: Int) {\n  fun f() {\n    val repo = 2\n    repo\n  }\n  fun g() = repo\n}\n")];
    assert_eq!(uses_in(&files, "p/A.kt", 1, "repo", false), v(&["A.kt:6:12 E"]));
}

#[test]
fn overloads_share_their_calls() {
    let files = [("p/R.kt", "package p\nclass R {\n  fun go(a: Int) {}\n  fun go(a: String) {}\n}\nfun call(r: R) { r.go(1) }\n")];
    assert_eq!(uses_in(&files, "p/R.kt", 2, "go", false), v(&["R.kt:5:19 E"]));
    assert_eq!(uses_in(&files, "p/R.kt", 3, "go", false), v(&["R.kt:5:19 E"]));
}

#[test]
fn parameter_usages_include_the_labels_of_calls() {
    assert_eq!(uses(REPO, 9, "limit", false), v(&["Usages.kt:11:14 E", "Repo.kt:9:43 E"]));
    assert_eq!(uses(USAGES, 11, "limit", true), v(&["Usages.kt:11:14 E", "Repo.kt:9:13 E", "Repo.kt:9:43 E"]));
}

#[test]
fn constructor_parameters_include_their_labels() {
    let files = [("p/S.kt", "package p\nclass S(private val repo: Int, title: String) {\n  val label = title\n  fun f() = repo\n}\n"),
                 ("q/U.kt", "package q\nimport p.S\nfun make() = S(repo = 1, title = \"t\")\n")];
    assert_eq!(uses_in(&files, "p/S.kt", 1, "title", false), v(&["S.kt:2:14 E", "U.kt:2:25 E"]));
    assert_eq!(uses_in(&files, "p/S.kt", 1, "repo", false), v(&["S.kt:3:12 E", "U.kt:2:15 E"]));
}

#[test]
fn labels_of_a_function_named_like_its_parameter() {
    let files = [("p/B.kt", "package p\nclass B {\n  fun title(title: String) = title\n}\n"),
                 ("q/U.kt", "package q\nimport p.B\nfun use(b: B) = b.title(title = \"x\")\n")];
    assert!(uses_in(&files, "p/B.kt", 2, "title:", false).contains(&"U.kt:2:24 E".to_string()));
}

#[test]
fn a_set_flag_cancels_the_search() {
    let (index, _) = build_index(&[root()]);
    let sources = Sources::default();
    let doc = sources.load(&root().join(REPO)).unwrap();
    let current = Current::of(&index, &doc);
    let cancel = Cancel::default();
    cancel.set();
    assert_eq!(references(&index, &current, &sources, byte_of(&doc.text, 7, "Repo"), false, &cancel), Err(Cancelled));
}

fn marks(rel: &str, line: usize, needle: &str) -> Vec<(u32, u32, bool)> {
    let (index, _) = build_index(&[root()]);
    let sources = Sources::default();
    let doc = sources.load(&root().join(rel)).unwrap();
    let current = Current::of(&index, &doc);
    kotlin_navigator::highlight::highlight(&index, &current, &sources, byte_of(&doc.text, line, needle))
        .iter().map(|h| (h.range.line, h.range.start, h.write)).collect()
}

#[test]
fn highlight_tells_writes_from_reads() {
    assert_eq!(marks(USAGES, 13, "total"), vec![(12, 8, true), (13, 4, true), (14, 4, true), (14, 27, false)]);
    assert_eq!(marks(REPO, 9, "limit"), vec![(9, 13, true), (9, 43, false)]);
}

#[test]
fn highlight_leaves_out_probable_places() {
    assert_eq!(marks(SCREEN, 10, "load"), vec![(10, 13, false), (25, 14, false)]);
}

#[test]
fn highlight_marks_member_assignment_as_write() {
    let dir = tempfile::tempdir().unwrap();
    let text = "class A { var n = 0 }\nfun f(a: A) {\n    a.n = 1\n    a.n += 2\n    println(a.n)\n}\n";
    std::fs::write(dir.path().join("A.kt"), text).unwrap();
    let (index, _) = build_index(&[dir.path().to_path_buf()]);
    let sources = Sources::default();
    let doc = sources.load(&dir.path().join("A.kt")).unwrap();
    let current = Current::of(&index, &doc);
    let found = kotlin_navigator::highlight::highlight(&index, &current, &sources, byte_of(text, 2, "n ="));
    let got: Vec<_> = found.iter().map(|h| (h.range.line, h.range.start, h.write)).collect();
    assert_eq!(got, vec![(0, 14, true), (2, 6, true), (3, 6, true), (4, 14, false)]);
}
