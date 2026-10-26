mod support;
use kotlin_navigator::lsp::{self, State};
use serde_json::json;
use std::{fs, path::PathBuf, time::{Duration, Instant}};
use support::{uri, Client};

const DOMAIN_TEXT: &str = "package com.example.domain\nclass StakingRepository {}\n";
const FEATURE_TEXT: &str = "package com.example.feature\nimport com.example.domain.StakingRepository\nclass StakingViewModel(val repo: StakingRepository)\n";

fn workspace() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let domain = dir.path().join("domain/src/StakingRepository.kt");
    let feature = dir.path().join("feature/src/StakingViewModel.kt");
    for (p, t) in [(&domain, DOMAIN_TEXT), (&feature, FEATURE_TEXT)] {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, t).unwrap();
    }
    (dir, domain, feature)
}
fn symbols(c: &mut Client, q: &str) -> Vec<serde_json::Value> {
    c.request("workspace/symbol", json!({"query": q})).unwrap().as_array().unwrap().clone()
}
fn open(c: &Client, path: &std::path::Path, text: &str) {
    c.notify("textDocument/didOpen", json!({"textDocument": {"uri": uri(path), "languageId": "kotlin", "version": 1, "text": text}}));
}

#[test]
fn definition_across_modules() {
    let (dir, domain, feature) = workspace();
    let mut c = Client::start(dir.path());
    let res = c.request("textDocument/definition", json!({"textDocument": {"uri": uri(&feature)}, "position": {"line": 2, "character": 36}})).unwrap();
    assert_eq!(res[0]["uri"], uri(&domain));
    assert_eq!(res[0]["range"]["start"], json!({"line": 1, "character": 6}));
    c.shutdown();
}

#[test]
fn unsaved_buffer_wins_and_close_restores_disk() {
    let (dir, domain, _) = workspace();
    let mut c = Client::start(dir.path());
    open(&c, &domain, "package com.example.domain\nclass BrandNewName {}\n");
    assert_eq!(symbols(&mut c, "BrandNewName").len(), 1);
    assert_eq!(symbols(&mut c, "StakingRepository").len(), 0);
    c.notify("textDocument/didClose", json!({"textDocument": {"uri": uri(&domain)}}));
    assert_eq!(symbols(&mut c, "BrandNewName").len(), 0);
    assert_eq!(symbols(&mut c, "StakingRepository").len(), 1);
    c.shutdown();
}

#[test]
fn watched_files_add_and_delete() {
    let (dir, _, _) = workspace();
    let mut c = Client::start(dir.path());
    let new_file = dir.path().join("domain/src/Fresh.kt");
    fs::write(&new_file, "package com.example.domain\nclass Fresh\n").unwrap();
    c.notify("workspace/didChangeWatchedFiles", json!({"changes": [{"uri": uri(&new_file), "type": 1}]}));
    assert_eq!(symbols(&mut c, "Fresh").len(), 1);
    fs::remove_file(&new_file).unwrap();
    c.notify("workspace/didChangeWatchedFiles", json!({"changes": [{"uri": uri(&new_file), "type": 3}]}));
    assert_eq!(symbols(&mut c, "Fresh").len(), 0);
    c.shutdown();
}

#[test]
fn gradle_script_is_not_indexed_but_navigates() {
    let (dir, domain, _) = workspace();
    let mut c = Client::start(dir.path());
    let script = dir.path().join("build.gradle.kts");
    open(&c, &script, "import com.example.domain.StakingRepository\nclass ScriptLocal\nval r: StakingRepository? = null\n");
    assert_eq!(symbols(&mut c, "ScriptLocal").len(), 0);
    let res = c.request("textDocument/definition", json!({"textDocument": {"uri": uri(&script)}, "position": {"line": 2, "character": 8}})).unwrap();
    assert_eq!(res[0]["uri"], uri(&domain));
    c.shutdown();
}

#[test]
fn unknown_request_is_method_not_found() {
    let (dir, _, _) = workspace();
    let mut c = Client::start(dir.path());
    assert_eq!(c.request("textDocument/hover", json!({})), Err(-32601));
    c.shutdown();
}

#[test]
fn request_waits_for_index_then_answers_from_open_buffers() {
    let state = State::new(Duration::from_millis(50));
    let path = PathBuf::from("/ws/a/Use.kt");
    state.open_or_change(path.clone(), "package a\nclass Repo\nval r: Repo? = null\n".into());
    let started = Instant::now();
    let locs = lsp::definition(&state, &path, lsp_types::Position { line: 2, character: 8 });
    assert!(started.elapsed() >= Duration::from_millis(50));
    assert_eq!(locs.len(), 1);
    assert_eq!(locs[0].range.start.line, 1);
}
