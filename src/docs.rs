//! Positions (LSP UTF-16 columns <-> byte offsets) and file URIs.

use std::path::{Path, PathBuf};

use lsp_types::Uri;

pub fn utf16_len(s: &str) -> u32 {
    s.chars().map(|c| c.len_utf16() as u32).sum()
}

/// Byte offset of an LSP position. A column past the end of the line is clamped
/// to the line end; a line past the end of the text yields `None`.
pub fn byte_offset(text: &str, line: u32, utf16_col: u32) -> Option<usize> {
    let mut line_start = 0;
    for _ in 0..line {
        line_start += text[line_start..].find('\n')? + 1;
    }
    let rest = &text[line_start..];
    let line_text = rest.split('\n').next().unwrap_or("");
    let line_text = line_text.strip_suffix('\r').unwrap_or(line_text);
    let mut units = 0;
    for (i, c) in line_text.char_indices() {
        units += c.len_utf16() as u32;
        if units > utf16_col {
            return Some(line_start + i);
        }
    }
    Some(line_start + line_text.len())
}

pub fn path_to_uri(path: &Path) -> Uri {
    let url = url::Url::from_file_path(path).expect("absolute file path");
    url.as_str().parse().expect("file URL is a valid URI")
}

pub fn uri_to_path(uri: &Uri) -> Option<PathBuf> {
    let url = url::Url::parse(uri.as_str()).ok()?;
    if url.scheme() != "file" {
        return None;
    }
    url.to_file_path().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn utf16_len_counts_surrogate_pairs() {
        assert_eq!(utf16_len("😀 Foo"), 6);
    }

    #[test]
    fn byte_offset_maps_utf16_columns() {
        assert_eq!(byte_offset("😀 Foo", 0, 3), Some(5));
        assert_eq!(byte_offset("a\n😀 Foo", 1, 4), Some(8));
        assert_eq!(byte_offset("abc", 0, 99), Some(3)); // колонка за концом строки прижимается к концу
        assert_eq!(byte_offset("abc", 5, 0), None); // строки нет
    }

    #[test]
    fn byte_offset_handles_crlf() {
        assert_eq!(byte_offset("ab\r\ncd", 1, 1), Some(5));
        assert_eq!(byte_offset("ab\r\ncd", 0, 99), Some(2)); // \r не входит в строку
    }

    #[test]
    fn uri_roundtrip_with_spaces_and_cyrillic() {
        let path = PathBuf::from("/Users/me/My Projects/Проект/A.kt");
        let uri = path_to_uri(&path);
        assert_eq!(uri.as_str(), "file:///Users/me/My%20Projects/%D0%9F%D1%80%D0%BE%D0%B5%D0%BA%D1%82/A.kt");
        assert_eq!(uri_to_path(&uri), Some(path));
    }

    #[test]
    fn non_file_uri_is_rejected() {
        let uri: lsp_types::Uri = "untitled:Untitled-1".parse().unwrap();
        assert_eq!(uri_to_path(&uri), None);
    }
}
