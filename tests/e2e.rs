use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use lsp_server::{Message, Notification, Request, RequestId, Response};
use serde_json::{Value, json};

#[test]
fn version_flag_prints_name_and_version() {
    let out = Command::new(env!("CARGO_BIN_EXE_kotlin-navigator")).arg("--version").output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "kotlin-navigator 0.2.0\n");
}

fn send(stdin: &mut impl Write, msg: Message) {
    msg.write(stdin).unwrap();
}

fn response(stdout: &mut impl BufRead, id: i32) -> Response {
    loop {
        match Message::read(stdout).unwrap().expect("server message") {
            Message::Response(r) if r.id == RequestId::from(id) => return r,
            _ => continue,
        }
    }
}

fn request(id: i32, method: &str, params: Value) -> Message {
    Message::Request(Request::new(RequestId::from(id), method.to_string(), params))
}

fn notification(method: &str, params: Value) -> Message {
    Message::Notification(Notification::new(method.to_string(), params))
}

#[test]
fn stdio_handshake_and_definition() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("feature")).unwrap();
    std::fs::create_dir_all(root.join("domain")).unwrap();
    let from_file = root.join("feature/ViewModel.kt");
    let to_file = root.join("domain/Repository.kt");
    std::fs::write(&from_file, "package a.feature\nimport a.domain.Repository\nclass VM(val repository: Repository)\n").unwrap();
    std::fs::write(&to_file, "package a.domain\nclass Repository\n").unwrap();
    let uri = |p: &std::path::Path| kotlin_navigator::docs::path_to_uri(p).as_str().to_string();

    let mut child = Command::new(env!("CARGO_BIN_EXE_kotlin-navigator"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    send(&mut stdin, request(1, "initialize", json!({"capabilities": {}, "rootUri": uri(root)})));
    assert_eq!(response(&mut stdout, 1).response_result.unwrap()["serverInfo"]["name"], "kotlin-navigator");
    send(&mut stdin, notification("initialized", json!({})));
    send(&mut stdin, request(2, "textDocument/definition",
        json!({"textDocument": {"uri": uri(&from_file)}, "position": {"line": 2, "character": 28}})));
    assert_eq!(response(&mut stdout, 2).response_result.unwrap()[0]["uri"], uri(&to_file));
    send(&mut stdin, request(3, "shutdown", Value::Null));
    assert_eq!(response(&mut stdout, 3).response_result.unwrap(), Value::Null);
    send(&mut stdin, notification("exit", Value::Null));
    drop(stdin);

    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "server did not exit within 5 s");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success());
}

#[test]
fn bench_prints_report() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("A.kt"), "class A\nclass B\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_kotlin-navigator")).arg("--bench").arg(dir.path()).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.starts_with("files: 1\nskipped: 0\ndecls: 2\nparse_error_files: 0\ncold_index_ms: "));
    assert!(text.contains("\npeak_rss_mb: "));
    assert!(text.contains("\ndefinition_samples: 1000\n"));
    assert!(text.contains("\ndefinition_one_pct: "));
    assert!(text.contains("\ndefinition_p95_us: "));
}
