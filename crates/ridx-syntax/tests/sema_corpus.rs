/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Runs `sema_fn` (`params`, `param_uses`, `own_returns`, `own_try_ops`, `tail_expr`) over every
//! `Fn` across the workspace and all 7 benchmark corpora (`RIDX_BENCH_REPOS`), verifying zero panics
//! and exact slice/kind invariants on every returned range and node.

use ridx_syntax::sema_fn::{norm_ident, own_returns, own_try_ops, param_uses, params, tail_expr};
use ridx_syntax::{parse, SyntaxKind as K};
use std::path::{Path, PathBuf};
use std::time::Instant;

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name();
        let name = name.to_string_lossy();
        if p.is_dir() {
            if name != "target" && name != ".git" && name != "test_data" {
                rs_files(&p, out);
            }
        } else if name.ends_with(".rs") {
            out.push(p);
        }
    }
}

#[derive(Default)]
struct CorpusStats {
    files: usize,
    fns: usize,
    params: usize,
    uses: usize,
    macro_uses: usize,
    returns: usize,
    try_ops: usize,
    tails: usize,
    sema_secs: f64,
}

fn check_corpus(files: &[PathBuf]) -> CorpusStats {
    let mut st = CorpusStats::default();
    for f in files {
        let Ok(src) = std::fs::read_to_string(f) else { continue };
        let tree = parse(&src);
        st.files += 1;
        let t0 = Instant::now();
        for id in tree.descendants(tree.root()) {
            if tree.kind(id) != K::Fn {
                continue;
            }
            st.fns += 1;
            let ps = params(&tree, id);
            st.params += ps.len();
            for p in &ps {
                for &b in &p.bindings {
                    let u = param_uses(&tree, id, b);
                    for r in &u.uses {
                        assert_eq!(norm_ident(&src[r.clone()]), b, "{}: bad use {:?}", f.display(), r);
                    }
                    for r in &u.macro_uses {
                        assert_eq!(norm_ident(&src[r.clone()]), b, "{}: bad macro use {:?}", f.display(), r);
                    }
                    st.uses += u.uses.len();
                    st.macro_uses += u.macro_uses.len();
                }
            }
            let rets = own_returns(&tree, id);
            for &r in &rets {
                assert_eq!(tree.kind(r), K::ReturnExpr);
            }
            st.returns += rets.len();
            let tries = own_try_ops(&tree, id);
            for &tr in &tries {
                assert_eq!(tree.kind(tr), K::TryExpr);
            }
            st.try_ops += tries.len();
            if let Some(tail) = tail_expr(&tree, id) {
                let tk = tree.kind(tail);
                assert!(tk.is_expr() || tk == K::MacroCall);
                st.tails += 1;
            }
        }
        st.sema_secs += t0.elapsed().as_secs_f64();
    }
    st
}

#[test]
fn corpora_sema_fn_without_panics() {
    let mut roots: Vec<(String, PathBuf)> = vec![("workspace".into(), PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))];
    if let Ok(base) = std::env::var("RIDX_BENCH_REPOS") {
        for c in ["serde", "ripgrep", "tokio", "rust-analyzer", "bevy", "clap", "regex"] {
            roots.push((c.into(), PathBuf::from(&base).join(c)));
        }
    }
    let (mut tot_files, mut tot_fns, mut tot_secs) = (0usize, 0usize, 0.0f64);
    for (label, root) in &roots {
        let mut files = Vec::new();
        rs_files(root, &mut files);
        let st = check_corpus(&files);
        tot_files += st.files;
        tot_fns += st.fns;
        tot_secs += st.sema_secs;
        eprintln!(
            "[sema:{label:<14}] files={:<5} fns={:<6} params={:<6} uses={:<7} mac={:<6} ret={:<5} try={:<6} tails={:<6} {:.2} ms",
            st.files,
            st.fns,
            st.params,
            st.uses,
            st.macro_uses,
            st.returns,
            st.try_ops,
            st.tails,
            st.sema_secs * 1e3
        );
    }
    eprintln!("[sema:TOTAL] files={tot_files} fns={tot_fns} sema_ms={:.2}", tot_secs * 1e3);
}

#[test]
fn broken_inputs_sema_never_panics() {
    let broken = [
        "fn f(x: ) { let x = ; if { x } else { match x { A if => x } } }",
        "fn f(&mut self, (a, ): ) -> { for x in { return x?; |x| } }",
        "fn f(x: i32) { println!(\"{x",
        "fn f(r#x: i32) { try { x? } async { return x } const { x } }",
    ];
    for src in broken {
        let tree = parse(src);
        for id in tree.descendants(tree.root()) {
            if tree.kind(id) == K::Fn {
                let ps = params(&tree, id);
                for p in &ps {
                    for &b in &p.bindings {
                        let _ = param_uses(&tree, id, b);
                    }
                }
                let _ = own_returns(&tree, id);
                let _ = own_try_ops(&tree, id);
                let _ = tail_expr(&tree, id);
            }
        }
    }
}
