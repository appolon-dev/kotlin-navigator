use kotlin_navigator::{docs::byte_offset, index::Index, parse::summarize_text, resolve::{definition, reference_at}};
use std::path::PathBuf;

/// Возвращает "путь:строка" для каждого кандидата.
fn goto(files: &[(&str, &str)], from: &str, line: u32, col: u32) -> Vec<String> {
    let mut index = Index::default();
    let mut current = None;
    for (path, text) in files {
        let (tree, summary) = summarize_text(PathBuf::from(path), text).unwrap();
        if *path == from { current = Some((tree, summary.clone(), text.to_string())); }
        index.upsert(summary);
    }
    let (tree, summary, text) = current.unwrap();
    let Some(r) = reference_at(&tree, &text, byte_offset(&text, line, col).unwrap()) else { return vec![] };
    definition(&index, &summary, &r).iter().map(|(f, d)| format!("{}:{}", f.path.display(), d.name_range.line)).collect()
}

const DOMAIN: (&str, &str) = ("/ws/domain/src/StakingRepository.kt", "package com.example.domain\nclass StakingRepository {}\n");
const FEATURE: (&str, &str) = ("/ws/feature/src/StakingViewModel.kt",
    "package com.example.feature\nimport com.example.domain.StakingRepository\nclass StakingViewModel(val repo: StakingRepository)\n");
const DOMAIN_HIT: &str = "/ws/domain/src/StakingRepository.kt:1";

#[test] fn cross_module_via_import() { assert_eq!(goto(&[DOMAIN, FEATURE], FEATURE.0, 2, 36), vec![DOMAIN_HIT]); }
#[test] fn cursor_right_after_identifier() { assert_eq!(goto(&[DOMAIN, FEATURE], FEATURE.0, 2, 50), vec![DOMAIN_HIT]); }
#[test] fn cursor_on_whitespace_finds_nothing() { assert!(goto(&[DOMAIN, FEATURE], FEATURE.0, 2, 32).is_empty()); }
#[test] fn click_on_import_line() { assert_eq!(goto(&[DOMAIN, FEATURE], FEATURE.0, 1, 30), vec![DOMAIN_HIT]); }

#[test]
fn alias_import() {
    let files = [("/ws/foo/RealName.kt", "package com.foo\nclass RealName\n"), ("/ws/bar/RealName.kt", "package com.bar\nclass RealName\n"),
                 ("/ws/app/Use.kt", "package app\nimport com.foo.RealName as Alias\nval x: Alias? = null\n")];
    assert_eq!(goto(&files, "/ws/app/Use.kt", 2, 8), vec!["/ws/foo/RealName.kt:1"]);
}

#[test]
fn own_file_beats_same_package() {
    let files = [("/ws/a/Repo.kt", "package a\nclass Repo\n"), ("/ws/a/Use.kt", "package a\nclass Repo\nval r: Repo? = null\n")];
    assert_eq!(goto(&files, "/ws/a/Use.kt", 2, 8), vec!["/ws/a/Use.kt:1"]);
}

#[test]
fn same_package_beats_other_packages() {
    let files = [("/ws/a/Repo.kt", "package a\nclass Repo\n"), ("/ws/b/Repo.kt", "package b\nclass Repo\n"), ("/ws/a/Use.kt", "package a\nval r: Repo? = null\n")];
    assert_eq!(goto(&files, "/ws/a/Use.kt", 1, 8), vec!["/ws/a/Repo.kt:1"]);
}

#[test]
fn wildcard_import() {
    let files = [("/ws/b/Repo.kt", "package b\nclass Repo\n"), ("/ws/c/Repo.kt", "package c\nclass Repo\n"), ("/ws/a/Use.kt", "package a\nimport b.*\nval r: Repo? = null\n")];
    assert_eq!(goto(&files, "/ws/a/Use.kt", 2, 8), vec!["/ws/b/Repo.kt:1"]);
}

#[test]
fn nested_class_import() {
    let files = [("/ws/p/Outer.kt", "package p\nclass Outer {\n  class Inner\n}\n"), ("/ws/q/Inner.kt", "package q\nclass Inner\n"),
                 ("/ws/a/Use.kt", "package a\nimport p.Outer.Inner\nval i: Inner? = null\n")];
    assert_eq!(goto(&files, "/ws/a/Use.kt", 2, 8), vec!["/ws/p/Outer.kt:2"]);
}

#[test]
fn ambiguous_name_returns_all_sorted() {
    let files = [("/ws/b/Repo.kt", "package b\nclass Repo\n"), ("/ws/a/Repo.kt", "package a\nclass Repo\n"), ("/ws/c/Use.kt", "package c\nval r: Repo? = null\n")];
    assert_eq!(goto(&files, "/ws/c/Use.kt", 1, 8), vec!["/ws/a/Repo.kt:1", "/ws/b/Repo.kt:1"]);
}

#[test]
fn candidates_are_capped_at_50() {
    let texts: Vec<(String, String)> = (0..51).map(|i| (format!("/ws/p{i}/Dup.kt"), format!("package p{i}\nclass Dup\n"))).collect();
    let mut files: Vec<(&str, &str)> = texts.iter().map(|(p, t)| (p.as_str(), t.as_str())).collect();
    files.push(("/ws/use/Use.kt", "package use\nval d: Dup? = null\n"));
    assert_eq!(goto(&files, "/ws/use/Use.kt", 1, 8).len(), 50);
}

#[test]
fn crlf_source() {
    let files = [("/ws/a/Repo.kt", "package a\r\nclass Repo\r\n"), ("/ws/a/Use.kt", "package a\r\n\r\nval r: Repo? = null\r\n")];
    assert_eq!(goto(&files, "/ws/a/Use.kt", 2, 8), vec!["/ws/a/Repo.kt:1"]);
}

#[test]
fn java_reference_to_kotlin_class() {
    let files = [("/ws/k/Repo.kt", "package k\nclass Repo\n"), ("/ws/j/Use.java", "package j;\nimport k.Repo;\nclass Use { Repo r; }\n")];
    assert_eq!(goto(&files, "/ws/j/Use.java", 2, 13), vec!["/ws/k/Repo.kt:1"]);
}

#[test]
fn explicit_import_of_unindexed_class_finds_nothing() {
    // `State` comes from a library; a same-named project class is never the target.
    let files = [("/ws/a/State.kt", "package a\nclass State\n"), ("/ws/b/Use.kt", "package b\nimport androidx.compose.runtime.State\nval s: State? = null\n")];
    assert!(goto(&files, "/ws/b/Use.kt", 2, 8).is_empty());
}
