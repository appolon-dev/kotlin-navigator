//! Declarations inside a Kotlin `ERROR` node. tree-sitter may leave the tokens after
//! an error out of the tree entirely, so the node's text is lexed: `class`, `interface`,
//! `object` or `typealias` followed by a name declares; the `{` after a declaration
//! header opens its body, any other `{` opens a block whose declarations are local and
//! skipped. Complete declarations and bodies inside the error are read from the tree.

use std::ops::Range;

use tree_sitter::Node;

use super::{DeclSink, declare, visit};
use crate::model::DeclKind;

/// Children of an `ERROR` node that the tree already got right.
const COMPLETE: [&str; 7] =
    ["class_declaration", "object_declaration", "companion_object", "type_alias", "enum_entry", "class_body", "enum_class_body"];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Tok<'a> {
    Word(&'a str),
    Open,
    Close,
    LParen,
    RParen,
    ColonColon,
}

pub(super) fn recover(sink: &mut DeclSink, node: Node, outer: Option<u32>) {
    let tokens = lex(sink.text, node.byte_range());
    let mut cursor = node.walk();
    let mut complete = node.named_children(&mut cursor).filter(|c| COMPLETE.contains(&c.kind())).peekable();
    // Some(decl): that declaration's body. None: a block; declarations inside are local.
    // Each scope also keeps the paren depth outside it, so an unbalanced `(` ends with its block.
    let mut scopes: Vec<(Option<u32>, usize)> = Vec::new();
    // A declaration header waiting for its `{`.
    let mut pending: Option<u32> = None;
    let mut parens = 0usize;
    let mut skip_until = 0;
    let word_at = |k: usize| match tokens.get(k) {
        Some((Tok::Word(w), r)) => Some((*w, r.clone())),
        _ => None,
    };
    for (i, (tok, range)) in tokens.iter().enumerate() {
        if range.start < skip_until {
            continue;
        }
        let scope = scopes.last().map(|s| s.0);
        let local = parens > 0 || scope == Some(None);
        let container = scope.unwrap_or(outer);
        while complete.next_if(|c| c.end_byte() <= range.start).is_some() {}
        if let Some(c) = complete.next_if(|c| c.start_byte() <= range.start) {
            let header = pending.take();
            if !local {
                match c.kind() {
                    "class_body" | "enum_class_body" => {
                        if let Some(owner) = header {
                            visit(sink, c, Some(owner));
                        }
                    }
                    _ => {
                        declare(sink, c, container);
                    }
                }
            }
            skip_until = c.end_byte();
            continue;
        }
        match tok {
            Tok::LParen => parens += 1,
            Tok::RParen => parens = parens.saturating_sub(1),
            Tok::Open => {
                let owner = if parens == 0 { pending.take() } else { None };
                scopes.push((owner, parens));
                parens = 0;
            }
            Tok::Close => parens = scopes.pop().map_or(0, |s| s.1),
            Tok::Word(word) if !local => {
                let after_colons = i > 0 && tokens[i - 1].0 == Tok::ColonColon; // `Foo::class`
                let prev_word = i.checked_sub(1).and_then(word_at).map(|(w, _)| w);
                let name = word_at(i + 1).filter(|(w, _)| !is_keyword(w));
                match *word {
                    "fun" | "val" | "var" | "init" | "constructor" => pending = None,
                    "class" | "interface" if !after_colons => {
                        let kind = match (*word, prev_word) {
                            ("interface", _) => DeclKind::Interface,
                            (_, Some("enum")) => DeclKind::Enum,
                            _ => DeclKind::Class,
                        };
                        pending = name.map(|(_, r)| sink.push_range(None, r, kind, container));
                    }
                    "object" => {
                        pending = match (name, prev_word) {
                            (Some((_, r)), _) => Some(sink.push_range(None, r, DeclKind::Object, container)),
                            (None, Some("companion")) => {
                                Some(sink.push_range(Some("Companion"), tokens[i - 1].1.clone(), DeclKind::Object, container))
                            }
                            (None, _) => None, // object expression
                        };
                    }
                    "typealias" => {
                        if let Some((_, r)) = name {
                            sink.push_range(None, r, DeclKind::TypeAlias, container);
                        }
                        pending = None;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

fn is_keyword(word: &str) -> bool {
    matches!(word, "class" | "interface" | "object" | "fun" | "val" | "var" | "typealias" | "companion")
}

/// Words, braces, parens and `::` of `text[region]`, skipping comments, strings and char literals.
fn lex(text: &str, region: Range<usize>) -> Vec<(Tok<'_>, Range<usize>)> {
    let bytes = text.as_bytes();
    let end = region.end.min(bytes.len());
    let mut out = Vec::new();
    let mut i = region.start;
    while i < end {
        let c = bytes[i];
        let rest = &bytes[i..end];
        if rest.starts_with(b"//") {
            i += rest.iter().position(|&b| b == b'\n').unwrap_or(rest.len());
        } else if rest.starts_with(b"/*") {
            i = skip_block_comment(bytes, i, end);
        } else if rest.starts_with(b"\"\"\"") {
            i = find(bytes, i + 3, end, b"\"\"\"").map_or(end, |j| j + 3);
        } else if c == b'"' {
            i = skip_quoted(bytes, i + 1, end, b'"');
        } else if c == b'\'' {
            i = skip_quoted(bytes, i + 1, end, b'\'');
        } else if c == b'`' {
            let close = rest[1..].iter().position(|&b| b == b'`' || b == b'\n').map_or(end, |j| i + 1 + j);
            out.push((Tok::Word(&text[i + 1..close]), i..(close + 1).min(end)));
            i = close + 1;
        } else if c == b'_' || c.is_ascii_alphabetic() || c >= 0x80 {
            let start = i;
            while i < end && (bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric() || bytes[i] >= 0x80) {
                i += 1;
            }
            out.push((Tok::Word(&text[start..i]), start..i));
        } else {
            let tok = match c {
                b'{' => Some(Tok::Open),
                b'}' => Some(Tok::Close),
                b'(' => Some(Tok::LParen),
                b')' => Some(Tok::RParen),
                b':' if rest.starts_with(b"::") => Some(Tok::ColonColon),
                _ => None,
            };
            let len = if tok == Some(Tok::ColonColon) { 2 } else { 1 };
            if let Some(tok) = tok {
                out.push((tok, i..i + len));
            }
            i += len;
        }
    }
    out
}

fn find(bytes: &[u8], from: usize, end: usize, needle: &[u8]) -> Option<usize> {
    (from..end.saturating_sub(needle.len() - 1)).find(|&j| bytes[j..end].starts_with(needle))
}

/// Kotlin block comments nest.
fn skip_block_comment(bytes: &[u8], mut i: usize, end: usize) -> usize {
    let mut depth = 0;
    while i < end {
        if bytes[i..end].starts_with(b"/*") {
            depth += 1;
            i += 2;
        } else if bytes[i..end].starts_with(b"*/") {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return i;
            }
        } else {
            i += 1;
        }
    }
    end
}

/// Skips a `"…"` or `'…'` body from just after the opening quote, including escapes and
/// `${…}` templates; returns the index after the closing quote, or the line end if unterminated.
fn skip_quoted(bytes: &[u8], mut i: usize, end: usize, quote: u8) -> usize {
    while i < end {
        match bytes[i] {
            b'\\' => i += 2,
            b'\n' => return i,
            b if b == quote => return i + 1,
            b'$' if quote == b'"' && bytes.get(i + 1) == Some(&b'{') => {
                let mut depth = 0;
                while i < end {
                    match bytes[i] {
                        b'{' => depth += 1,
                        b'}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    end
}
