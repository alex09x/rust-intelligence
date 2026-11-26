/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The dependencies the driver compiled an empty stand-in for, because they did not compile (see the driver's
//! `stand_in`): their dependents are indexed, but every use of such a crate is unresolved, and `ridx index` says so.

use std::fs;
use std::path::Path;

use ridx_format::STAND_IN_PREFIX;

/// The crates standing in under `cargo_target_dir`, sorted: a stand-in source whose dep-info (the crate's
/// `<crate><extra>.d` beside it) still names it. A later build of the real crate rewrote that dep-info.
pub fn active(cargo_target_dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    // <target>/[<triple>/]<profile>/deps
    let mut stack = vec![(cargo_target_dir.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        for path in fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path()) {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if path.is_dir() {
                if depth < 3 {
                    stack.push((path.clone(), depth + 1));
                }
                continue;
            }
            let Some(stem) = name.strip_prefix(STAND_IN_PREFIX).and_then(|s| s.strip_suffix(".rs")) else { continue };
            let dep_info = fs::read_to_string(dir.join(format!("{stem}.d"))).unwrap_or_default();
            if dep_info.contains(name) {
                // A crate name has no `-`; the extra filename starts with one.
                found.push(stem.split('-').next().unwrap_or(stem).to_string());
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stand_ins_are_listed_while_their_dep_info_names_them() {
        let root = std::env::temp_dir().join(format!("ridx_stand_ins_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let deps = root.join("debug").join("deps");
        fs::create_dir_all(&deps).unwrap();
        let stand_in = deps.join("ridx-stand-in-trybuild-0a1b2c.rs");
        fs::write(&stand_in, "").unwrap();
        fs::write(deps.join("trybuild-0a1b2c.d"), format!("{}: {}\n", deps.join("libtrybuild-0a1b2c.rmeta").display(), stand_in.display())).unwrap();
        // The real crate built since: its dep-info names its own sources again.
        fs::write(deps.join("ridx-stand-in-toml-99.rs"), "").unwrap();
        fs::write(deps.join("toml-99.d"), "libtoml-99.rmeta: /registry/toml/src/lib.rs\n").unwrap();
        assert_eq!(active(&root), vec!["trybuild".to_string()]);
        let _ = fs::remove_dir_all(&root);
    }
}
