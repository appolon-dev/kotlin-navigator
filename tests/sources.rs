use kotlin_navigator::docs::{Documents, OpenDoc};
use kotlin_navigator::sources::Sources;
use std::sync::Arc;

#[test]
fn open_buffers_win_and_disk_files_are_parsed_once() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("A.kt"), dir.path().join("B.kt"));
    std::fs::write(&a, "package p\nclass OnDisk\n").unwrap();
    std::fs::write(&b, "package p\nclass Other\n").unwrap();
    let mut docs = Documents::default();
    docs.insert(OpenDoc::parse(a.clone(), "package p\nclass Unsaved\n".into(), None).unwrap());
    let sources = Sources::new(docs.snapshot());
    assert_eq!(sources.load(&a).unwrap().summary.decls[0].name, "Unsaved");
    assert!(Arc::ptr_eq(&sources.load(&b).unwrap(), &sources.load(&b).unwrap()));
    assert!(sources.load(&dir.path().join("Missing.kt")).is_none());
}

#[test]
fn load_containing_skips_files_without_the_needle() {
    let dir = tempfile::tempdir().unwrap();
    let c = dir.path().join("C.kt");
    std::fs::write(&c, "package p\nclass Other\n").unwrap();
    let sources = Sources::default();
    assert!(sources.load_containing(&c, "Unsaved").is_none());
    assert_eq!(sources.load_containing(&c, "Other").unwrap().summary.decls[0].name, "Other");
}
