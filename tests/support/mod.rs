//! In-memory LSP client driving `lsp::run` on a background thread.

use std::path::Path;
use std::thread::JoinHandle;
use std::time::Duration;

use kotlin_navigator::{docs::path_to_uri, lsp};
use lsp_server::{Connection, Message, Notification, Request, RequestId};
use serde_json::{Value, json};

pub struct Client {
    conn: Connection,
    next_id: i32,
    server: Option<JoinHandle<anyhow::Result<()>>>,
}

pub fn uri(path: &Path) -> String {
    path_to_uri(path).as_str().to_string()
}

impl Client {
    pub fn start(root: &Path) -> Client {
        let (server, client) = Connection::memory();
        let handle = std::thread::spawn(move || lsp::run(server));
        let mut c = Client { conn: client, next_id: 0, server: Some(handle) };
        c.request("initialize", json!({"capabilities": {}, "rootUri": uri(root)})).unwrap();
        c.notify("initialized", json!({}));
        c
    }

    /// Sends a request and waits for its response, skipping anything else the server sends.
    pub fn request(&mut self, method: &str, params: Value) -> Result<Value, i32> {
        self.next_id += 1;
        let id = RequestId::from(self.next_id);
        self.conn.sender.send(Message::Request(Request::new(id.clone(), method.to_string(), params))).unwrap();
        loop {
            match self.conn.receiver.recv_timeout(Duration::from_secs(20)).expect("server response") {
                Message::Response(r) if r.id == id => {
                    return r.response_result.map_err(|e| e.code);
                }
                _ => continue,
            }
        }
    }

    pub fn notify(&self, method: &str, params: Value) {
        self.conn.sender.send(Message::Notification(Notification::new(method.to_string(), params))).unwrap();
    }

    pub fn shutdown(mut self) {
        assert_eq!(self.request("shutdown", Value::Null), Ok(Value::Null));
        self.notify("exit", Value::Null);
        self.server.take().unwrap().join().unwrap().unwrap();
    }
}
