#!/usr/bin/env python3
"""Tiny Kotlin/Java Go-to-Definition LSP for source navigation, not compilation.

Python 3.9+, standard library only. Does not run Gradle or a JVM.
"""
from __future__ import annotations

import json
import os
import re
import sys
import threading
import traceback
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import unquote, urlparse

PACKAGE = re.compile(r"(?m)^\s*package\s+([\w.]+)")
IMPORT = re.compile(r"(?m)^[ \t]*import[ \t]+(?:static[ \t]+)?([A-Za-z_]\w*(?:\.[A-Za-z_]\w*)*(?:\.\*)?)(?:[ \t]+as[ \t]+(\w+))?[ \t]*;?")
# Lightweight textual approximation. Handles common Kotlin class modifiers.
DECL = re.compile(
    r"(?m)^\s*(?:(?:public|internal|protected|private|open|abstract|data|sealed|enum|annotation|value|inner|expect|actual|final|external|inline|fun|static|strictfp|suspend)\s+)*"
    r"(class|interface|object|typealias|record|enum)\s+(`?[A-Za-z_][A-Za-z_0-9]*`?)\b"
)
IDENT = re.compile(r"[A-Za-z_][A-Za-z_0-9]*")
SKIP = {".git", ".hg", ".svn", ".idea", ".gradle", ".kotlin", ".venv", "build", "out", "target", "node_modules", "bin", "generated"}
MAX_FILE_SIZE = 2 * 1024 * 1024


def debug(message: str) -> None:
    if os.environ.get("KOTLIN_NAVIGATOR_DEBUG") == "1":
        print("navigator: " + message, file=sys.stderr, flush=True)


def file_from_uri(uri: str) -> Path:
    parsed = urlparse(uri)
    if parsed.scheme != "file":
        raise ValueError("Unsupported URI scheme")
    return Path(unquote(parsed.path)).resolve()


def as_uri(path: Path) -> str:
    return path.resolve().as_uri()


def utf16_to_py(line: str, offset: int) -> int:
    count = 0
    for i, ch in enumerate(line):
        count += 2 if ord(ch) > 0xFFFF else 1
        if count > offset:
            return i
        if count == offset:
            return i + 1
    return len(line)


def py_to_utf16(line: str, offset: int) -> int:
    return sum(2 if ord(ch) > 0xFFFF else 1 for ch in line[:offset])


@dataclass(frozen=True)
class Symbol:
    name: str
    package: str
    uri: str
    line: int
    column: int
    kind: int

    @property
    def fqn(self) -> str:
        return f"{self.package}.{self.name}" if self.package else self.name

    def location(self) -> dict:
        position = {"line": self.line, "character": self.column}
        end = {"line": self.line, "character": self.column + len(self.name)}
        return {"uri": self.uri, "range": {"start": position, "end": end}}


def get_package(text: str) -> str:
    found = PACKAGE.search(text)
    return found.group(1) if found else ""


def declarations(text: str, uri: str) -> list[Symbol]:
    package = get_package(text)
    output = []
    for match in DECL.finditer(text):
        name = match.group(2).strip("`")
        start = match.start(2)
        line = text.count("\n", 0, start)
        line_start = text.rfind("\n", 0, start) + 1
        col = py_to_utf16(text[line_start:start], len(text[line_start:start]))
        kind = 11 if match.group(1) in ("interface",) else 5  # LSP: Interface=11, Class=5
        output.append(Symbol(name, package, uri, line, col, kind))
    return output


def imports(text: str) -> tuple[dict[str, str], list[str]]:
    explicit, wildcard = {}, []
    for match in IMPORT.finditer(text):
        target, alias = match.group(1), match.group(2)
        if target.endswith(".*"):
            wildcard.append(target[:-2])
        else:
            explicit[alias or target.rsplit(".", 1)[-1]] = target
    return explicit, wildcard


def symbol_at(text: str, line_number: int, character: int) -> str | None:
    lines = text.splitlines()
    if line_number < 0 or line_number >= len(lines):
        return None
    line = lines[line_number]
    offset = utf16_to_py(line, character)
    for match in IDENT.finditer(line):
        if match.start() <= offset <= match.end():
            return match.group(0)
    return None


class Index:
    def __init__(self):
        self.roots: list[Path] = []
        self.by_uri: dict[str, list[Symbol]] = {}
        self.by_name: dict[str, list[Symbol]] = defaultdict(list)
        self.overlays: dict[str, str] = {}
        self.ready = threading.Event()
        self.lock = threading.RLock()

    def set_roots(self, roots: list[Path]):
        self.roots = roots
        self.ready.clear()
        threading.Thread(target=self.rebuild, daemon=True).start()

    def iter_files(self):
        visited = set()
        for root in self.roots:
            if not root.exists():
                continue
            for dirpath, dirs, files in os.walk(root):
                dirs[:] = [d for d in dirs if d not in SKIP and not d.startswith(".cache")]
                for filename in files:
                    if not filename.endswith((".kt", ".java")):
                        continue
                    path = Path(dirpath) / filename
                    try:
                        if path.stat().st_size > MAX_FILE_SIZE:
                            continue
                        resolved = path.resolve()
                        if resolved not in visited:
                            visited.add(resolved)
                            yield resolved
                    except OSError:
                        continue

    def rebuild(self):
        by_uri = {}
        scanned = 0
        for path in self.iter_files():
            uri = as_uri(path)
            try:
                with self.lock:
                    text = self.overlays.get(uri)
                if text is None:
                    text = path.read_text(encoding="utf-8", errors="replace")
                defs = declarations(text, uri)
                if defs:
                    by_uri[uri] = defs
                scanned += 1
            except OSError:
                continue
        with self.lock:
            # Open unsaved buffers might have changed while scanning.
            for uri, text in self.overlays.items():
                by_uri[uri] = declarations(text, uri)
            self.by_uri = by_uri
            self._reindex()
        debug(f"scanned {scanned} source files, {len(self.by_name)} names")
        self.ready.set()

    def _reindex(self):
        by_name = defaultdict(list)
        for group in self.by_uri.values():
            for item in group:
                by_name[item.name].append(item)
        self.by_name = by_name

    def update(self, uri: str, text: str):
        with self.lock:
            self.overlays[uri] = text
            self.by_uri[uri] = declarations(text, uri)
            self._reindex()

    def close(self, uri: str):
        with self.lock:
            self.overlays.pop(uri, None)
        self.reload(uri)

    def reload(self, uri: str):
        try:
            with self.lock:
                content = self.overlays.get(uri)
            if content is None:
                content = file_from_uri(uri).read_text(encoding="utf-8", errors="replace")
            defs = declarations(content, uri)
        except (OSError, ValueError):
            defs = []
        with self.lock:
            if defs:
                self.by_uri[uri] = defs
            else:
                self.by_uri.pop(uri, None)
            self._reindex()

    def content(self, uri: str) -> str:
        with self.lock:
            buffer = self.overlays.get(uri)
        if buffer is not None:
            return buffer
        return file_from_uri(uri).read_text(encoding="utf-8", errors="replace")

    def definition(self, uri: str, line: int, col: int) -> list[dict]:
        self.ready.wait(timeout=30)
        try:
            text = self.content(uri)
        except (OSError, ValueError):
            return []
        word = symbol_at(text, line, col)
        if not word:
            return []
        package = get_package(text)
        explicit, wildcard = imports(text)
        target_name = explicit.get(word, word).rsplit(".", 1)[-1]
        with self.lock:
            matches = list(self.by_name.get(target_name, []))
        if not matches:
            return []
        # Prefer explicit imports (including aliases), then own file/package, then wildcard imports.
        if word in explicit:
            selected = [s for s in matches if s.fqn == explicit[word]]
            if selected:
                return [s.location() for s in selected]
        own_file = [s for s in matches if s.uri == uri]
        if own_file:
            return [s.location() for s in own_file]
        same_package = [s for s in matches if s.package == package]
        if same_package:
            return [s.location() for s in same_package]
        star = [s for s in matches if s.package in wildcard]
        if star:
            return [s.location() for s in star]
        # Better to offer multiple candidates than silently select a wrong class.
        return [s.location() for s in matches[:50]]

    def workspace_symbols(self, term: str):
        self.ready.wait(timeout=30)
        with self.lock:
            matches = [s for group in self.by_name.values() for s in group if term.lower() in s.name.lower()]
        return [{"name": s.name, "kind": s.kind, "location": s.location(), "containerName": s.package} for s in matches[:100]]


def read_message(stdin) -> dict | None:
    headers = {}
    while True:
        line = stdin.readline()
        if not line:
            return None
        if line in (b"\r\n", b"\n"):
            break
        if b":" in line:
            name, value = line.split(b":", 1)
            headers[name.decode("ascii").strip().lower()] = value.decode("ascii").strip()
    length = int(headers.get("content-length", "0"))
    if length < 1 or length > 20 * 1024 * 1024:
        return None
    body = stdin.read(length)
    if len(body) != length:
        return None
    return json.loads(body)


def send_message(stdout, message: dict):
    body = json.dumps(message, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    stdout.write(f"Content-Length: {len(body)}\r\n\r\n".encode("ascii") + body)
    stdout.flush()


class Server:
    def __init__(self, stdin=None, stdout=None):
        self.stdin = stdin or sys.stdin.buffer
        self.stdout = stdout or sys.stdout.buffer
        self.index = Index()
        self.running = True

    def process(self, msg: dict):
        method, params = msg.get("method"), msg.get("params") or {}
        msg_id = msg.get("id")
        if method == "initialize":
            roots = []
            for folder in params.get("workspaceFolders") or []:
                try:
                    roots.append(file_from_uri(folder["uri"]))
                except (ValueError, KeyError):
                    continue
            if not roots and params.get("rootUri"):
                try:
                    roots = [file_from_uri(params["rootUri"])]
                except ValueError:
                    pass
            if not roots and params.get("rootPath"):
                roots = [Path(params["rootPath"]).resolve()]
            self.index.set_roots(roots)
            return {"capabilities": {"textDocumentSync": 1, "definitionProvider": True, "workspaceSymbolProvider": True},
                    "serverInfo": {"name": "kotlin-navigator", "version": "0.1.0"}}
        if method == "textDocument/didOpen":
            doc = params.get("textDocument", {})
            self.index.update(doc["uri"], doc.get("text", ""))
        elif method == "textDocument/didChange":
            doc = params.get("textDocument", {})
            changes = params.get("contentChanges", [])
            if changes and "range" not in changes[-1]:
                self.index.update(doc["uri"], changes[-1]["text"])
        elif method == "textDocument/didClose":
            self.index.close(params["textDocument"]["uri"])
        elif method == "textDocument/didSave":
            self.index.reload(params["textDocument"]["uri"])
        elif method == "workspace/didChangeWatchedFiles":
            for change in params.get("changes", []):
                self.index.reload(change["uri"])
        elif method == "textDocument/definition":
            pos = params["position"]
            return self.index.definition(params["textDocument"]["uri"], pos["line"], pos["character"])
        elif method == "workspace/symbol":
            return self.index.workspace_symbols(params.get("query", ""))
        elif method == "shutdown":
            return None
        elif method == "exit":
            self.running = False
        elif msg_id is not None:  # Unknown request: safely return an empty/unsupported response.
            return None
        return None

    def run(self):
        while self.running:
            msg = read_message(self.stdin)
            if msg is None:
                break
            # Responses from client lack method; ignore them.
            if not msg.get("method"):
                continue
            try:
                result = self.process(msg)
                if "id" in msg:
                    send_message(self.stdout, {"jsonrpc": "2.0", "id": msg["id"], "result": result})
            except Exception as exc:
                debug(traceback.format_exc())
                if "id" in msg:
                    send_message(self.stdout, {"jsonrpc": "2.0", "id": msg["id"], "error": {"code": -32603, "message": str(exc)}})


if __name__ == "__main__":
    Server().run()
