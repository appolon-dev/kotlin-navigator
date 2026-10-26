use kotlin_navigator::index::Index;
use kotlin_navigator::model::FileSummary;
use kotlin_navigator::parse::summarize_text;
use kotlin_navigator::resolve::context::Role;
use kotlin_navigator::resolve::rank::{prefer, rank};
use std::path::PathBuf;

fn file(path: &str, text: &str, module: Option<&str>) -> FileSummary {
    let mut s = summarize_text(PathBuf::from(path), text).unwrap().1;
    s.module = module.map(PathBuf::from);
    s
}
fn ranked(files: Vec<FileSummary>, current: FileSummary, name: &str) -> Vec<String> {
    let mut index = Index::default();
    files.into_iter().for_each(|f| index.upsert(f));
    rank(&index, &current, index.lookup(name)).iter().map(|(f, d)| format!("{}:{}", f.path.display(), d.name_range.line)).collect()
}

#[test]
fn private_of_another_file_is_dropped() {
    let a = file("/ws/a/A.kt", "package a\nprivate fun f() {}\n", None);
    let b = file("/ws/b/B.kt", "package b\nfun f() {}\n", None);
    assert_eq!(ranked(vec![a, b], file("/ws/a/Use.kt", "package a\n", None), "f"), vec!["/ws/b/B.kt:1"]);
}

#[test]
fn incompatible_source_set_is_dropped_unless_alone() {
    let android = file("/ws/m/src/androidMain/kotlin/H.kt", "package h\nclass H\n", None);
    let ios = file("/ws/m/src/iosMain/kotlin/H.kt", "package h\nclass H\n", None);
    let from_android = file("/ws/m/src/androidMain/kotlin/U.kt", "package u\n", None);
    assert_eq!(ranked(vec![android, ios.clone()], from_android.clone(), "H"), vec!["/ws/m/src/androidMain/kotlin/H.kt:1"]);
    assert_eq!(ranked(vec![ios], from_android, "H"), vec!["/ws/m/src/iosMain/kotlin/H.kt:1"]);
}

#[test]
fn same_module_first_and_foreign_internal_last() {
    let core = file("/ws/core/C.kt", "package x\ninternal fun f() {}\n", Some("/ws/core"));
    let app = file("/ws/app/A.kt", "package y\nfun f() {}\n", Some("/ws/app"));
    let lib = file("/ws/lib/L.kt", "package z\nfun f() {}\n", Some("/ws/lib"));
    let current = file("/ws/app/U.kt", "package u\n", Some("/ws/app"));
    assert_eq!(ranked(vec![core, app, lib], current, "f"), vec!["/ws/app/A.kt:1", "/ws/lib/L.kt:1", "/ws/core/C.kt:1"]);
}

#[test]
fn expect_brings_its_actuals_compatible_first() {
    let expect = file("/ws/m/src/commonMain/kotlin/P.kt", "package p\nexpect fun plat(): String\n", None);
    let android = file("/ws/m/src/androidMain/kotlin/P.android.kt", "package p\nactual fun plat() = \"a\"\n", None);
    let ios = file("/ws/m/src/iosMain/kotlin/P.ios.kt", "package p\nactual fun plat() = \"i\"\n", None);
    let all = vec![expect, android, ios];
    assert_eq!(ranked(all.clone(), file("/ws/m/src/androidMain/kotlin/U.kt", "package p\n", None), "plat"),
        vec!["/ws/m/src/androidMain/kotlin/P.android.kt:1", "/ws/m/src/commonMain/kotlin/P.kt:1", "/ws/m/src/iosMain/kotlin/P.ios.kt:1"]);
    assert_eq!(ranked(all, file("/ws/m/src/commonMain/kotlin/U.kt", "package p\n", None), "plat"),
        vec!["/ws/m/src/commonMain/kotlin/P.kt:1", "/ws/m/src/androidMain/kotlin/P.android.kt:1", "/ws/m/src/iosMain/kotlin/P.ios.kt:1"]);
}

#[test]
fn prefer_follows_role_tiers() {
    let s = file("/ws/a/A.kt", "package a\nclass Format\nfun Format() {}\nval Format = 1\n", None);
    let mut index = Index::default();
    index.upsert(s);
    let kinds = |role: Role| prefer(&role, index.lookup("Format")).iter().map(|(_, d)| d.name_range.line).collect::<Vec<_>>();
    assert_eq!(kinds(Role::Call), vec![2]);
    assert_eq!(kinds(Role::Type), vec![1]);
    assert_eq!(kinds(Role::Expr), vec![1, 3]);
}
