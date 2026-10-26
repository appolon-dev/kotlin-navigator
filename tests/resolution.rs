use kotlin_navigator::index::Index;
use kotlin_navigator::parse::summarize_text;
use kotlin_navigator::resolve::{resolve, Certainty::{self, *}, Current, Found};
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
