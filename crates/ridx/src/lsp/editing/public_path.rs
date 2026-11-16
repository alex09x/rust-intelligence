/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The public path to an item of another crate, read from that crate's sources: a `pub use` re-export, or a chain of
//! `pub mod` (in files or inline). What `Implement default members` qualifies a name with and what a completion's
//! auto-import writes.

use super::action_parse::{is_ident_byte, mask_comments_and_strings};
use std::path::Path;

/// Whether byte `p` of a module's masked text is at that module's own level: inside no braces but a macro call's
/// (`cfg_rt! { pub mod task; }`), not in a nested `mod prelude { … }`, `fn` or `impl`.
fn at_module_level(masked: &str, p: usize) -> bool {
    let b = masked.as_bytes();
    let (mut depth, mut i) = (0i32, p);
    while i > 0 {
        i -= 1;
        match b[i] {
            b'}' => depth += 1,
            b'{' if depth > 0 => depth -= 1,
            b'{' if masked[..i].trim_end().ends_with('!') => {}
            b'{' => return false,
            _ => {}
        }
    }
    true
}

/// The body of the module `m` declared inline at the level of `text` (`pub mod m { … }`), between its braces.
fn inline_module_body(text: &str, m: &str) -> Option<String> {
    let masked = mask_comments_and_strings(text);
    let decl = format!("pub mod {m}");
    let b = masked.as_bytes();
    let at = masked.match_indices(&decl).map(|(p, _)| p).find(|&p| {
        let after = &masked[p + decl.len()..];
        (p == 0 || !is_ident_byte(b[p - 1])) && at_module_level(&masked, p) && after.trim_start().starts_with('{')
    })?;
    let open = at + decl.len() + masked[at + decl.len()..].find('{')?;
    let mut depth = 0usize;
    for (i, c) in masked[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(text[open + 1..open + i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// A path that reaches `full` (`krate::m1::m2::Item`) from another crate, read from the defining crate's sources rooted
/// at `root_file`: the crate root's `pub use` re-export of `Item` when it has one, else `full` itself when every
/// module on the way is declared `pub mod`; `None` when neither can be shown.
pub(crate) fn public_path(root_file: &Path, full: &str) -> Option<String> {
    let segs: Vec<&str> = full.split("::").collect();
    let (krate, item) = (*segs.first()?, *segs.last()?);
    if segs.len() <= 2 {
        return Some(full.to_string());
    }
    let declares = |text: &str, kw: &str, name: &str| {
        let masked = mask_comments_and_strings(text);
        masked.match_indices(kw).any(|(p, _)| {
            let rest = &masked[p + kw.len()..];
            let b = masked.as_bytes();
            (p == 0 || !is_ident_byte(b[p - 1]))
                && at_module_level(&masked, p)
                && (kw != "pub mod " || rest.starts_with(name) && !rest[name.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_'))
                && (kw != "pub use " || rest.split(';').next().is_some_and(|u| u.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|w| w == name)))
        })
    };
    let root_text = std::fs::read_to_string(root_file).ok()?;
    if declares(&root_text, "pub use ", item) {
        return Some(format!("{krate}::{item}"));
    }
    let (mut dir, mut text) = (root_file.parent()?.to_path_buf(), root_text);
    for (k, m) in segs[1..segs.len() - 1].iter().enumerate() {
        // A module that re-exports the item by name (`pub use join_set::JoinSet;`) gives the shortest public path,
        // whichever `cfg` makes the module below public (`mod join_set;` in one branch, `pub mod` in another).
        if k > 0 && declares(&text, "pub use ", item) {
            return Some(format!("{}::{item}", segs[..=k].join("::")));
        }
        if !declares(&text, "pub mod ", m) {
            // A private module whose items the module above re-exports (`mod access; pub use access::*;`, or by
            // name): the item is reached there.
            let masked = mask_comments_and_strings(&text);
            let glob = [format!("pub use {m}::*"), format!("pub use self::{m}::*")].iter().any(|g| masked.contains(g.as_str()));
            return (glob || declares(&text, "pub use ", item)).then(|| format!("{}::{item}", segs[..=k].join("::")));
        }
        // The module's text: its body when declared inline (`pub mod shown { … }`), else its file.
        if let Some(body) = inline_module_body(&text, m) {
            text = body;
            dir = dir.join(m);
            continue;
        }
        let (file, sub) = [(dir.join(format!("{m}.rs")), dir.join(m)), (dir.join(m).join("mod.rs"), dir.join(m))].into_iter().find(|(f, _)| f.is_file())?;
        text = std::fs::read_to_string(&file).ok()?;
        dir = sub;
    }
    Some(full.to_string())
}

#[cfg(test)]
mod tests {
    use super::public_path;

    #[test]
    fn test_public_path_through_pub_modules_or_root_reexport() {
        let dir = std::env::temp_dir().join(format!("ridx_pubpath_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src/de")).unwrap();
        std::fs::write(
            dir.join("src/lib.rs"),
            "mod sink;\npub mod de;\npub use crate::sink::{Sink, SinkError};\npub mod inline {\n    pub struct Shown;\n    mod deep { pub struct Deeper; }\n}\n",
        )
        .unwrap();
        std::fs::write(dir.join("src/sink.rs"), "pub struct Sink;\npub struct SinkError;\npub struct Hidden;\n").unwrap();
        std::fs::write(dir.join("src/de/mod.rs"), "pub struct Unexpected;\n").unwrap();
        let root = dir.join("src/lib.rs");
        assert_eq!(public_path(&root, "searcher::sink::SinkError").as_deref(), Some("searcher::SinkError"), "root re-export");
        assert_eq!(public_path(&root, "searcher::de::Unexpected").as_deref(), Some("searcher::de::Unexpected"), "pub module chain");
        assert_eq!(public_path(&root, "searcher::sink::Hidden"), None, "private module, no re-export");
        assert_eq!(public_path(&root, "searcher::inline::Shown").as_deref(), Some("searcher::inline::Shown"), "an inline pub module");
        assert_eq!(public_path(&root, "searcher::inline::deep::Deeper"), None, "a private module inside an inline one");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
