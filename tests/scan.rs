use kotlin_navigator::scan::{build_index, walk_sources};
use std::{fs, path::Path};

fn write(root: &Path, rel: &str, bytes: &[u8]) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, bytes).unwrap();
}

#[test]
fn walk_skips_ignored_hidden_build_and_large_files() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, "app/src/main/A.kt", b"class A");
    write(r, "app/src/main/B.java", b"class B {}");
    write(r, "app/build.gradle.kts", b"plugins {}");
    write(r, ".claude/worktrees/w1/app/A.kt", b"class A");
    for d in ["build", "out", "target", "bin", "node_modules", "generated"] {
        write(r, &format!("lib/{d}/X.kt"), b"class X");
    }
    write(r, ".gitignore", b"ignored/\n");
    write(r, "ignored/I.kt", b"class I");
    write(r, "big/Big.kt", &vec![b' '; 2 * 1024 * 1024 + 1]);
    let found: Vec<String> = walk_sources(&[r.to_path_buf()]).iter()
        .map(|p| p.strip_prefix(r).unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(found, vec!["app/src/main/A.kt", "app/src/main/B.java"]);
}

#[test]
fn build_index_skips_non_utf8_files() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, "a/A.kt", b"package a\nclass A\n");
    write(r, "a/Bad.kt", b"class \xff\xfe Bad\n");
    let (idx, stats) = build_index(&[r.to_path_buf()]);
    assert_eq!((stats.files, stats.skipped), (1, 1));
    assert_eq!(idx.lookup("A").len(), 1);
    assert_eq!(idx.file_count(), 1);
}

#[test]
fn git_tracked_sources_are_kept_even_when_ignored() {
    // Like `.git/info/exclude: keys` hitting a tracked `navigation/keys/` package.
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    let git = |args: &[&str]| assert!(std::process::Command::new("git").arg("-C").arg(r).args(args).output().unwrap().status.success());
    git(&["init", "-q"]);
    write(r, ".git/info/exclude", b"keys\n");
    write(r, "nav/keys/ScreenKey.kt", b"class ScreenKey");
    write(r, "nav/keys/Untracked.kt", b"class Untracked");
    git(&["add", "-f", "nav/keys/ScreenKey.kt"]);
    let found: Vec<String> = walk_sources(&[r.to_path_buf()]).iter()
        .map(|p| p.strip_prefix(r).unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(found, vec!["nav/keys/ScreenKey.kt"]);
}

#[test]
fn module_is_the_nearest_dir_with_a_build_script() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, "build.gradle.kts", b"");
    write(r, "core/build.gradle", b"");
    write(r, "core/src/commonMain/kotlin/a/A.kt", b"package a\nclass A\n");
    write(r, "loose/B.kt", b"class B\n");
    let roots = [r.to_path_buf()];
    assert_eq!(kotlin_navigator::scan::module_of(&r.join("core/src/commonMain/kotlin/a/A.kt"), &roots), Some(r.join("core")));
    assert_eq!(kotlin_navigator::scan::module_of(&r.join("loose/B.kt"), &roots), Some(r.to_path_buf()));
    assert_eq!(kotlin_navigator::scan::module_of(Path::new("/elsewhere/C.kt"), &roots), None);
    let (idx, _) = build_index(&roots);
    let a = idx.file(&r.join("core/src/commonMain/kotlin/a/A.kt")).unwrap();
    assert_eq!((a.module.clone(), a.source_set.as_deref()), (Some(r.join("core")), Some("commonMain")));
}
