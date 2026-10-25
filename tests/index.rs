use kotlin_navigator::index::Index;
use kotlin_navigator::model::FileSummary;
use kotlin_navigator::parse::summarize_text;
use std::path::{Path, PathBuf};

fn file(path: &str, text: &str) -> FileSummary {
    summarize_text(PathBuf::from(path), text).unwrap().1
}

#[test]
fn upsert_replaces_previous_names_of_the_file() {
    let mut idx = Index::default();
    idx.upsert(file("/ws/A.kt", "class Old\nclass Keep\n"));
    idx.upsert(file("/ws/A.kt", "class New\nclass Keep\n"));
    assert!(idx.lookup("Old").is_empty());
    assert_eq!(idx.lookup("New").len(), 1);
    assert_eq!(idx.lookup("Keep").len(), 1);
    assert_eq!((idx.file_count(), idx.decl_count()), (1, 2));
}

#[test]
fn remove_drops_file_and_names() {
    let mut idx = Index::default();
    idx.upsert(file("/ws/A.kt", "class A\n"));
    idx.remove(Path::new("/ws/A.kt"));
    assert!(idx.lookup("A").is_empty());
    assert!(idx.file(Path::new("/ws/A.kt")).is_none());
}

#[test]
fn lookup_is_sorted_by_path_then_line() {
    let mut idx = Index::default();
    idx.upsert(file("/ws/b.kt", "class Repo\n"));
    idx.upsert(file("/ws/a.kt", "\nclass Repo\n"));
    let paths: Vec<_> = idx.lookup("Repo").iter().map(|(f, _)| f.path.clone()).collect();
    assert_eq!(paths, vec![PathBuf::from("/ws/a.kt"), PathBuf::from("/ws/b.kt")]);
}
