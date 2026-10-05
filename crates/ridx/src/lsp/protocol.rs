//! LSP wire protocol helpers, URI conversion, symbol mappings, and position caching.

use ridx_format::DefKind;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{self, Write};
use std::path::Path;

pub fn lsp_symbol_kind(kind: DefKind) -> u32 {
    match kind {
        DefKind::Fn => 12,                          // Function
        DefKind::AssocFn => 6,                      // Method
        DefKind::Struct => 23,                      // Struct
        DefKind::Enum => 10,                        // Enum
        DefKind::Variant => 22,                     // EnumMember
        DefKind::Trait => 11,                       // Interface
        DefKind::Mod => 2,                          // Module
        DefKind::Const | DefKind::AssocConst => 14, // Constant
        DefKind::Static => 13,                      // Variable
        DefKind::Local => 13,                       // Variable
        DefKind::Field => 8,                        // Field
        DefKind::TyAlias => 26,                     // TypeParameter
        _ => 13,
    }
}

pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                decoded.push(byte);
                i += 3;
                continue;
            }
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&decoded).to_string()
}

pub fn percent_encode_path(s: &str) -> String {
    let mut encoded = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                encoded.push(b as char);
            }
            _ => {
                use std::fmt::Write;
                let _ = write!(encoded, "%{:02X}", b);
            }
        }
    }
    encoded
}

pub fn uri_to_path(uri: &str) -> String {
    let stripped = uri
        .strip_prefix("file://localhost")
        .or_else(|| uri.strip_prefix("file://"))
        .unwrap_or(uri);
    percent_decode(stripped)
}

pub fn path_to_uri(file: &str) -> String {
    let abs_path = if file.starts_with('/') {
        file.to_string()
    } else if let Ok(canon) = Path::new(file).canonicalize() {
        canon.to_string_lossy().to_string()
    } else {
        format!("/{}", file)
    };
    format!("file://{}", percent_encode_path(&abs_path))
}

/// Request-scoped line and position converter.
///
/// Converts between 1-based character/column coordinates and LSP 0-based UTF-16 code units.
/// Caches read lines per file on-demand during the lifetime of a single request to avoid
/// repeated full-file disk reads when formatting high-fanout reference/symbol responses,
/// and drops all buffers when the request completes to maintain near-zero resident memory.
#[derive(Default)]
pub struct PositionConverter {
    cache: HashMap<String, Vec<String>>,
}

impl PositionConverter {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    fn get_line(&mut self, file: &str, line_1based: u32) -> Option<&str> {
        let lines = self.cache.entry(file.to_string()).or_insert_with(|| {
            if let Ok(content) = std::fs::read_to_string(file) {
                content.lines().map(|s| s.to_string()).collect()
            } else {
                Vec::new()
            }
        });

        if line_1based == 0 || (line_1based as usize) > lines.len() {
            None
        } else {
            Some(&lines[(line_1based - 1) as usize])
        }
    }

    /// Converts 1-based char column (1-indexed code points) to LSP 0-based character (UTF-16 code units).
    pub fn char_col_to_utf16_col(
        &mut self,
        file: &str,
        line_1based: u32,
        char_col_1based: u32,
    ) -> u32 {
        if char_col_1based <= 1 {
            return 0;
        }
        if let Some(line) = self.get_line(file, line_1based) {
            let mut utf16_count = 0u32;
            for (char_idx, ch) in line.chars().enumerate() {
                if (char_idx as u32) + 1 >= char_col_1based {
                    break;
                }
                utf16_count += ch.len_utf16() as u32;
            }
            utf16_count
        } else {
            char_col_1based.saturating_sub(1)
        }
    }

    /// Converts LSP 0-based character (UTF-16 code units) to 1-based char column (1-indexed code points).
    pub fn utf16_col_to_char_col(
        &mut self,
        file: &str,
        line_1based: u32,
        utf16_col_0based: u32,
    ) -> u32 {
        if utf16_col_0based == 0 {
            return 1;
        }
        if let Some(line) = self.get_line(file, line_1based) {
            let mut current_utf16 = 0u32;
            for (char_idx, ch) in line.chars().enumerate() {
                if current_utf16 >= utf16_col_0based {
                    return (char_idx as u32) + 1;
                }
                current_utf16 += ch.len_utf16() as u32;
            }
            (line.chars().count() as u32) + 1
        } else {
            utf16_col_0based + 1
        }
    }
}

pub fn send_response<W: Write>(writer: &mut W, response: &Value) -> io::Result<()> {
    let body = serde_json::to_string(response)?;
    write!(writer, "Content-Length: {}\r\n\r\n{}", body.len(), body)?;
    writer.flush()?;
    Ok(())
}
