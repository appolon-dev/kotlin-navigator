mod support;
use kotlin_navigator::lsp::{self, State};
use serde_json::json;
use std::{fs, path::PathBuf, sync::Arc, time::{Duration, Instant}};
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
    let state = State::new(Vec::new(), Duration::from_millis(50));
    let path = PathBuf::from("/ws/a/Use.kt");
    state.open_or_change(path.clone(), "package a\nclass Repo\nval r: Repo? = null\n".into());
    let started = Instant::now();
    let locs = lsp::definition(&state, &path, lsp_types::Position { line: 2, character: 8 });
    assert!(started.elapsed() >= Duration::from_millis(50));
    assert_eq!(locs.len(), 1);
    assert_eq!(locs[0].range.start.line, 1);
}

#[test]
fn buffers_where_the_scan_does_not_look_navigate_but_stay_out_of_the_index() {
    let (dir, domain, feature) = workspace();
    let mut c = Client::start(dir.path());
    let worktree_copy = dir.path().join(".claude/worktrees/w1/domain/src/StakingRepository.kt");
    let generated_copy = dir.path().join("domain/build/generated/StakingRepository.kt");
    let worktree_feature = dir.path().join(".claude/worktrees/w1/feature/src/StakingViewModel.kt");
    for (p, t) in [(&worktree_copy, DOMAIN_TEXT), (&generated_copy, DOMAIN_TEXT), (&worktree_feature, FEATURE_TEXT)] {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, t).unwrap();
        open(&c, p, t);
    }
    assert_eq!(symbols(&mut c, "StakingRepository").len(), 1);
    let at = |p: &PathBuf| json!({"textDocument": {"uri": uri(p)}, "position": {"line": 2, "character": 36}});
    for from in [&feature, &worktree_feature] {
        let res = c.request("textDocument/definition", at(from)).unwrap();
        assert_eq!(res.as_array().unwrap().len(), 1);
        assert_eq!(res[0]["uri"], uri(&domain));
    }
    for p in [&worktree_copy, &generated_copy] {
        c.notify("textDocument/didClose", json!({"textDocument": {"uri": uri(p)}}));
    }
    assert_eq!(symbols(&mut c, "StakingRepository").len(), 1);
    // A new file in an ordinary source directory is indexed as soon as it is opened.
    open(&c, &dir.path().join("domain/src/Fresh.kt"), "package com.example.domain\nclass Fresh\n");
    assert_eq!(symbols(&mut c, "Fresh").len(), 1);
    c.shutdown();
}

fn summary(path: &str, text: &str) -> kotlin_navigator::model::FileSummary {
    kotlin_navigator::parse::summarize_text(PathBuf::from(path), text).unwrap().1
}

#[test]
fn after_the_timeout_requests_use_the_published_part() {
    let state = State::new(Vec::new(), Duration::from_millis(50));
    state.publish(vec![summary("/ws/a/Repo.kt", "package a\nclass Repo\n")]);
    let path = PathBuf::from("/ws/a/Use.kt");
    state.open_or_change(path.clone(), "package a\nval r: Repo? = null\n".into());
    let locs = lsp::definition(&state, &path, lsp_types::Position { line: 1, character: 8 });
    assert_eq!(locs.len(), 1);
    assert!(locs[0].uri.as_str().ends_with("/ws/a/Repo.kt"));
}

#[test]
fn published_file_keeps_its_open_buffer() {
    let state = State::new(Vec::new(), Duration::from_millis(10));
    state.open_or_change(PathBuf::from("/ws/a/Repo.kt"), "package a\nclass Fresh\n".into());
    state.publish(vec![summary("/ws/a/Repo.kt", "package a\nclass Repo\n")]);
    assert_eq!((lsp::workspace_symbols(&state, "Fresh").len(), lsp::workspace_symbols(&state, "Repo").len()), (1, 0));
}

#[test]
fn indexer_panic_still_marks_ready() {
    let state = State::new(Vec::new(), Duration::from_secs(10));
    lsp::index_in_background(Arc::clone(&state), |publish| {
        publish(vec![summary("/ws/a/Repo.kt", "package a\nclass Repo\n")]);
        panic!("boom");
    }).join().unwrap();
    let started = Instant::now();
    assert!(state.wait_ready());
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(lsp::workspace_symbols(&state, "Repo").len(), 1);
}

#[test]
fn member_definition_over_lsp() {
    let dir = tempfile::tempdir().unwrap();
    let domain = dir.path().join("domain/src/Repo.kt");
    let feature = dir.path().join("feature/src/Vm.kt");
    for (p, t) in [(&domain, "package d\nclass Repo {\n  fun stake() {}\n}\n"),
                   (&feature, "package f\nimport d.Repo\nclass Vm(val repo: Repo) {\n  fun go() { repo.stake() }\n}\n")] {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, t).unwrap();
    }
    let mut c = Client::start(dir.path());
    let res = c.request("textDocument/definition", json!({"textDocument": {"uri": uri(&feature)}, "position": {"line": 3, "character": 18}})).unwrap();
    assert_eq!(res[0]["uri"], uri(&domain));
    assert_eq!(res[0]["range"]["start"], json!({"line": 2, "character": 6}));
    c.shutdown();
}
