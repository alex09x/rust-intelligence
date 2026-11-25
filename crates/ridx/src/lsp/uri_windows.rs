/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Windows file URIs and paths. A client names `C:\Users\a\lib.rs` as `file:///c%3A/Users/a/lib.rs` (VS Code) or
//! `file:///C:/Users/a/lib.rs`; the compiler, the index and the file system name it `C:\Users\a\lib.rs`. The first
//! Windows LSP session answered hover and references with nothing: the URI's path `/c:/Users/...` matched no
//! indexed file. The conversions are plain string work, so they are tested on every platform.

use std::path::Path;

/// `/c:/Users/a/lib.rs`, the path of a Windows file URI, as Windows names the file: `C:\Users\a\lib.rs`. Other paths
/// are left as they are.
pub fn windows_path_of_uri_path(path: &str) -> String {
    let b = path.as_bytes();
    if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':' && b.get(3).is_none_or(|c| *c == b'/' || *c == b'\\') {
        format!("{}{}", (b[1] as char).to_ascii_uppercase(), path[2..].replace('/', "\\"))
    } else {
        path.to_string()
    }
}

/// A Windows path (`C:\a\b.rs`, or canonical `\\?\C:\a\b.rs`) as the path of a file URI: `/C:/a/b.rs`; `None` for a
/// path without a drive.
pub fn uri_path_of_windows_path(path: &str) -> Option<String> {
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    let b = path.as_bytes();
    (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':').then(|| format!("/{}{}", (b[0] as char).to_ascii_uppercase(), path[1..].replace('\\', "/")))
}

/// The file URI of a Windows path, absolute or made so through the file system: `file:///C:/a/b.rs`, the drive's
/// colon as it is (RFC 8089), as every client reads it.
pub fn windows_file_uri(file: &str) -> Option<String> {
    let uri_path = uri_path_of_windows_path(file).or_else(|| Path::new(file).canonicalize().ok().and_then(|c| uri_path_of_windows_path(&c.to_string_lossy())))?;
    Some(format!("file://{}", super::protocol::percent_encode_path(&uri_path).replacen("%3A", ":", 1)))
}

/// The raw file URI of `path` (no percent-encoding: a `%` in a name stays the name), as compiler diagnostics are
/// published: `file:///home/a/lib.rs`, on Windows `file:///C:/Users/a/lib.rs` (not `file://C:\Users\a\lib.rs`).
pub fn raw_file_uri(path: &Path) -> String {
    let text = path.to_string_lossy();
    match uri_path_of_windows_path(&text).filter(|_| cfg!(windows)) {
        Some(uri_path) => format!("file://{uri_path}"),
        None => format!("file://{text}"),
    }
}

/// The path a [`raw_file_uri`] names.
pub fn path_of_raw_file_uri(uri: &str) -> std::path::PathBuf {
    let path = uri.strip_prefix("file://").unwrap_or(uri);
    std::path::PathBuf::from(if cfg!(windows) { windows_path_of_uri_path(path) } else { path.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_a_windows_uri_names_the_file_as_windows_does() {
        assert_eq!(windows_path_of_uri_path("/c:/Users/a/src/lib.rs"), r"C:\Users\a\src\lib.rs", "VS Code: lower-case drive");
        assert_eq!(windows_path_of_uri_path("/C:/Users/a/src/lib.rs"), r"C:\Users\a\src\lib.rs");
        assert_eq!(windows_path_of_uri_path("/d:"), "D:");
        assert_eq!(windows_path_of_uri_path("/home/a/src/lib.rs"), "/home/a/src/lib.rs", "not a drive");
        assert_eq!(windows_path_of_uri_path("/ab:/x"), "/ab:/x");
    }

    #[test]
    fn test_a_windows_path_becomes_a_file_uri() {
        assert_eq!(uri_path_of_windows_path(r"C:\Users\a\src\lib.rs").as_deref(), Some("/C:/Users/a/src/lib.rs"));
        assert_eq!(uri_path_of_windows_path(r"\\?\c:\Users\a b\lib.rs").as_deref(), Some("/C:/Users/a b/lib.rs"), "canonical form");
        assert_eq!(uri_path_of_windows_path("/home/a/lib.rs"), None);
        assert_eq!(windows_file_uri(r"C:\Users\a b\lib.rs").as_deref(), Some("file:///C:/Users/a%20b/lib.rs"));
        // A round trip through the client's form.
        let path = windows_path_of_uri_path(&crate::lsp::protocol::percent_decode("/c%3A/Users/a%20b/lib.rs"));
        assert_eq!(windows_file_uri(&path).as_deref(), Some("file:///C:/Users/a%20b/lib.rs"));
    }
}
