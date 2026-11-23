/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `RIDX_NO_BUILD_SCRIPTS=1`: no code of the workspace or of its dependencies runs while it is indexed (prod-code,
//! m=20739: a workspace with `build_scripts = false`). A build script is compiled from `fn main() {}`, so that what
//! Cargo runs is that; a procedural macro crate is compiled from a stub declaring the same macros, so that what
//! rustc loads expands a derive to nothing, an attribute to the item as written and a function-like macro to
//! nothing. Code that needs what they would have produced does not compile, and is indexed as far as it does.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rustc_driver::Callbacks;
use rustc_span::source_map::{FileLoader, RealFileLoader};

use ridx_format::{NO_BUILD_DIR_PREFIX, NO_BUILD_ENV as ENV};

/// Set with [`ENV`] where only the workspace's own crates are indexed: another crate is compiled, not indexed.
const DEPS_UNINDEXED: &str = "RIDX_DEPS_UNINDEXED";

/// Whether this compile runs with no build script or procedural macro executed.
pub(crate) fn enabled() -> bool {
    std::env::var_os(ENV).is_some_and(|v| v == "1")
}

/// The compile of `args` in this mode, when it is not indexed as any other: a build script or a procedural macro
/// crate compiled from its stub (a local macro crate indexed first, as written, by a run of this driver without the
/// mode: compiling it runs none of it), a dependency compiled without an index, or a refusal of a procedural macro
/// not built in this mode (an index built before, with its macros, names it).
pub(crate) fn compile(args: &[String], local: bool) -> Option<Result<(), ()>> {
    let foreign = args.windows(2).filter(|w| w[0] == "--extern").filter_map(|w| w[1].split_once('=').map(|(_, p)| Path::new(p))).find(|p| {
        matches!(p.extension().and_then(|e| e.to_str()), Some("so" | "dylib" | "dll"))
            && !p.components().any(|c| c.as_os_str().to_str().is_some_and(|c| c.starts_with(NO_BUILD_DIR_PREFIX)))
    });
    if let Some(p) = foreign {
        eprintln!("error: ridx-driver: {ENV}=1 and the procedural macro {} was not built without running it", p.display());
        return Some(Err(()));
    }
    let primary = std::env::var_os(DEPS_UNINDEXED).is_none() || std::env::var_os("CARGO_PRIMARY_PACKAGE").is_some();
    match unit_of(args) {
        Unit::BuildScript => Some(compile_stub(args, |_| "fn main() {}\n".to_string())),
        Unit::ProcMacro => {
            if local && primary {
                let exe = std::env::current_exe().ok()?;
                let indexed = std::process::Command::new(exe).args(&args[1..]).env_remove(ENV).status();
                if !indexed.is_ok_and(|s| s.success()) {
                    eprintln!("warning: ridx-driver: the procedural macro crate was not indexed");
                }
            }
            Some(compile_stub(args, proc_macro_stub))
        }
        Unit::Other if !primary => Some(crate::run_with_callbacks(args, &mut crate::NoopCallbacks)),
        Unit::Other => None,
    }
}

/// What a compile is, for this mode.
#[derive(Debug, PartialEq)]
pub(crate) enum Unit {
    BuildScript,
    ProcMacro,
    Other,
}

/// The unit compiled by the rustc arguments `args`.
pub(crate) fn unit_of(args: &[String]) -> Unit {
    fn value<'a>(args: &'a [String], flag: &'a str) -> impl Iterator<Item = &'a str> {
        args.windows(2).filter(move |w| w[0] == flag).map(|w| w[1].as_str()).chain(args.iter().filter_map(move |a| a.strip_prefix(flag).and_then(|r| r.strip_prefix('='))))
    }
    let value = |flag| value(args, flag);
    if value("--crate-name").any(|n| n.starts_with("build_script_")) {
        Unit::BuildScript
    } else if value("--crate-type").any(|t| t.split(',').any(|t| t == "proc-macro")) {
        Unit::ProcMacro
    } else {
        Unit::Other
    }
}

/// The root of a procedural macro crate as a stub: each `#[proc_macro]`, `#[proc_macro_attribute]` and
/// `#[proc_macro_derive(..)]` function of `src` with its attribute as written and a body executing nothing of the
/// crate's. Comments, strings and characters are skipped; a macro declared otherwise (under `cfg_attr`, by a macro)
/// is not in it.
pub(crate) fn proc_macro_stub(src: &str) -> String {
    let code = blank_non_code(src);
    let mut out = String::from("extern crate proc_macro;\n");
    let mut at = 0;
    while let Some(i) = code[at..].find("#[").map(|i| at + i) {
        at = i + 2;
        let Some(close) = closing(&code, i + 1) else { break };
        let attr: String = code[i + 2..close].split_whitespace().collect::<Vec<_>>().join(" ");
        let kind = attr.split(['(', ' ']).next().unwrap_or("");
        if !matches!(kind, "proc_macro" | "proc_macro_attribute" | "proc_macro_derive") {
            continue;
        }
        let Some(name) = next_fn_name(&code[close + 1..]) else { continue };
        let body = match kind {
            "proc_macro_attribute" => format!("pub fn {name}(_: proc_macro::TokenStream, item: proc_macro::TokenStream) -> proc_macro::TokenStream {{ item }}"),
            _ => format!("pub fn {name}(_: proc_macro::TokenStream) -> proc_macro::TokenStream {{ proc_macro::TokenStream::new() }}"),
        };
        out.push_str(&format!("#[{attr}]\n{body}\n"));
        at = close + 1;
    }
    out
}

/// `src` with every comment, string and character literal blanked out, byte offsets kept.
fn blank_non_code(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| out[from..to].iter_mut().filter(|c| c.is_ascii()).for_each(|c| *c = b' ');
    while i < b.len() {
        let rest = &b[i..];
        let end = if rest.starts_with(b"//") {
            rest.iter().position(|&c| c == b'\n').map_or(b.len(), |p| i + p)
        } else if rest.starts_with(b"/*") {
            let mut depth = 0usize;
            let mut j = i;
            loop {
                if j + 1 >= b.len() {
                    break b.len();
                }
                if &b[j..j + 2] == b"/*" {
                    depth += 1;
                    j += 2;
                } else if &b[j..j + 2] == b"*/" {
                    depth -= 1;
                    j += 2;
                    if depth == 0 {
                        break j;
                    }
                } else {
                    j += 1;
                }
            }
        } else if let Some(hashes) = raw_string_start(rest) {
            let open = i + hashes + 2 + usize::from(rest[0] == b'b');
            let close = format!("\"{}", "#".repeat(hashes));
            src[open..].find(&close).map_or(b.len(), |p| open + p + close.len())
        } else if rest[0] == b'"' || rest.starts_with(b"b\"") {
            let mut j = i + if rest[0] == b'"' { 1 } else { 2 };
            while j < b.len() && b[j] != b'"' {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            (j + 1).min(b.len())
        } else if let Some(len) = (rest[0] == b'\'').then(|| char_literal_len(&src[i..])).flatten() {
            i + len
        } else {
            i += 1;
            continue;
        };
        blank(&mut out, i, end);
        i = end;
    }
    String::from_utf8(out).unwrap_or_default()
}

/// The number of `#` of a raw string literal starting `rest` (`r"`, `r#"`, `br#"`).
fn raw_string_start(rest: &[u8]) -> Option<usize> {
    let r = rest.strip_prefix(b"b").unwrap_or(rest).strip_prefix(b"r")?;
    let hashes = r.iter().take_while(|&&c| c == b'#').count();
    (r.get(hashes) == Some(&b'"')).then_some(hashes)
}

/// The length of a character literal starting `rest` (`'a'`, `'\n'`, `'\u{7b}'`), not a lifetime (`'a`).
fn char_literal_len(rest: &str) -> Option<usize> {
    let b = rest.as_bytes();
    if b.get(1) == Some(&b'\\') {
        // The escaped character itself may be a quote (`'\''`).
        return b.get(3..)?.iter().position(|&c| c == b'\'').map(|p| p + 4);
    }
    let c = rest[1..].chars().next()?;
    (b.get(1 + c.len_utf8()) == Some(&b'\'')).then_some(c.len_utf8() + 2)
}

/// The index of the `]` closing the `[` at `open`.
fn closing(code: &str, open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (j, c) in code[open..].char_indices() {
        match c {
            '[' | '(' | '{' => depth += 1,
            ']' | ')' | '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + j);
                }
            }
            _ => {}
        }
    }
    None
}

/// The name of the function an attribute is on: the identifier after the first `fn` (past other attributes and
/// `pub`).
fn next_fn_name(code: &str) -> Option<String> {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut at = 0;
    loop {
        let i = at + code[at..].find("fn")?;
        at = i + 2;
        let before = code[..i].chars().next_back().is_none_or(|c| !word(c) && c != '#');
        let rest = &code[at..];
        if before && rest.starts_with(char::is_whitespace) {
            let rest = rest.trim_start();
            let rest = rest.strip_prefix("r#").unwrap_or(rest);
            let name: String = rest.chars().take_while(|&c| word(c)).collect();
            return (!name.is_empty()).then_some(name);
        }
    }
}

/// Serves `text` for the crate root `root` and every other file from disk.
struct StubLoader {
    root: PathBuf,
    text: String,
}

impl StubLoader {
    fn is_root(&self, path: &Path) -> bool {
        let abs = |p: &Path| std::env::current_dir().map(|d| d.join(p)).ok().and_then(|p| p.canonicalize().ok());
        path == self.root || abs(path).is_some_and(|p| Some(p) == abs(&self.root))
    }
}

impl FileLoader for StubLoader {
    fn file_exists(&self, path: &Path) -> bool {
        self.is_root(path) || RealFileLoader.file_exists(path)
    }

    fn read_file(&self, path: &Path) -> io::Result<String> {
        if self.is_root(path) {
            return Ok(self.text.clone());
        }
        RealFileLoader.read_file(path)
    }

    fn read_binary_file(&self, path: &Path) -> io::Result<Arc<[u8]>> {
        if self.is_root(path) {
            return Ok(self.text.as_bytes().into());
        }
        RealFileLoader.read_binary_file(path)
    }

    fn current_directory(&self) -> io::Result<PathBuf> {
        RealFileLoader.current_directory()
    }
}

/// Compiles the unit of `args` from `stub(root text)` in place of its root file, indexing nothing.
pub(crate) fn compile_stub(args: &[String], stub: impl Fn(&str) -> String + Send) -> Result<(), ()> {
    struct Stub<F>(F);
    impl<F: Fn(&str) -> String + Send> Callbacks for Stub<F> {
        fn config(&mut self, config: &mut rustc_interface::Config) {
            if let rustc_session::config::Input::File(root) = &config.input {
                let text = (self.0)(&std::fs::read_to_string(root).unwrap_or_default());
                config.file_loader = Some(Box::new(StubLoader { root: root.clone(), text }));
            }
        }
    }
    crate::run_with_callbacks(args, &mut Stub(stub))
}

#[cfg(test)]
#[path = "tests/no_build_tests.rs"]
mod tests;
