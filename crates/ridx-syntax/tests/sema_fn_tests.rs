/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Focused unit tests for every rule in `ridx_syntax::sema_fn`.

use ridx_syntax::sema_fn::{fn_body, own_returns, own_try_ops, param_uses, params, tail_expr, SelfKind};
use ridx_syntax::{parse, NodeId, SyntaxKind as K, SyntaxTree};

fn first_fn(tree: &SyntaxTree<'_>) -> NodeId {
    tree.descendants(tree.root()).find(|&n| tree.kind(n) == K::Fn).expect("expected a Fn node")
}

fn slices<'s>(src: &'s str, ranges: &[std::ops::Range<usize>]) -> Vec<&'s str> {
    ranges.iter().map(|r| &src[r.clone()]).collect()
}

#[test]
fn test_params_self_variants_and_patterns() {
    let cases = [
        ("fn a(self) {}", Some(SelfKind::Value)),
        ("fn b(mut self) {}", Some(SelfKind::MutValue)),
        ("fn c(&self) {}", Some(SelfKind::Ref { lifetime: None })),
        ("fn d(&mut self) {}", Some(SelfKind::MutRef { lifetime: None })),
        ("fn e<'a>(&'a self) {}", Some(SelfKind::Ref { lifetime: Some("'a") })),
        ("fn g<'a>(&'a mut self) {}", Some(SelfKind::MutRef { lifetime: Some("'a") })),
        ("fn h(mut self: Box<Self>) {}", Some(SelfKind::Typed { mutability: true, ty: "Box<Self>" })),
    ];
    for (src, expected) in cases {
        let t = parse(src);
        let p = params(&t, first_fn(&t));
        assert_eq!(p.len(), 1, "{src}");
        assert_eq!(p[0].self_kind, expected, "{src}");
        assert_eq!(p[0].bindings, vec!["self"], "{src}");
    }

    let complex = "fn k(&self, (a, mut b): (i32, u8), Point { x, y: z }: Point, (Ok(r) | Err(r)): Result<u8, u8>, _: u8) {}";
    let t = parse(complex);
    let ps = params(&t, first_fn(&t));
    assert_eq!(ps.len(), 5);
    assert_eq!(ps[1].pat_text, Some("(a, mut b)"));
    assert_eq!(ps[1].bindings, vec!["a", "b"]);
    assert_eq!(ps[1].ty_text, Some("(i32, u8)"));
    assert_eq!(ps[2].bindings, vec!["x", "z"]);
    assert_eq!(ps[2].ty_text, Some("Point"));
    assert_eq!(ps[3].bindings, vec!["r"]);
    assert_eq!(ps[4].bindings, Vec::<&str>::new());
}

#[test]
fn test_single_segment_path_expr_only() {
    let src = "fn f(x: S) -> i32 { x.x + S::x() + ::x::y + x::<u8>() + Foo { x: 1 }.x + Foo { x }.x }";
    let t = parse(src);
    let u = param_uses(&t, first_fn(&t), "x");
    // Only `x` in `x.x` and shorthand `x` in `Foo { x }` are single-segment PathExpr uses of param `x`.
    assert_eq!(u.uses.len(), 2, "{:?}", u.uses);
    assert_eq!(u.uses[0], 20..21);
    assert_eq!(u.uses[1], 79..80);
    assert!(u.macro_uses.is_empty());
}

#[test]
fn test_raw_ident_normalization() {
    let src = "fn f(r#match: i32, r#x: i32) -> i32 { r#match + x + r#x }";
    let t = parse(src);
    let f = first_fn(&t);
    let ps = params(&t, f);
    assert_eq!(ps[0].bindings, vec!["match"]);
    assert_eq!(ps[1].bindings, vec!["x"]);
    assert_eq!(slices(src, &param_uses(&t, f, "match").uses), vec!["r#match"]);
    assert_eq!(slices(src, &param_uses(&t, f, "x").uses), vec!["x", "r#x"]);
}

#[test]
fn test_let_and_let_else_shadowing() {
    let src = "fn f(x: Option<i32>) -> i32 { let Some(x) = x else { return x.unwrap_or(0); }; x + 1 }";
    let t = parse(src);
    let u = param_uses(&t, first_fn(&t), "x");
    // Uses: scrutinee `= x` and `x.unwrap_or(0)` inside `else`; trailing `x + 1` is shadowed!
    assert_eq!(u.uses.len(), 2);
    assert_eq!(&src[u.uses[0].clone()], "x");
    assert_eq!(u.uses[0].start, src.find("= x").unwrap() + 2);
    assert_eq!(u.uses[1].start, src.find("return x").unwrap() + 7);
}

#[test]
fn test_if_let_and_let_chain_shadowing() {
    let src = "fn f(x: Option<i32>, y: Option<i32>) -> i32 {\n\
        if let Some(a) = x && let Some(x) = y && x > a {\n\
            x + a\n\
        } else {\n\
            x.unwrap_or(0)\n\
        }\n\
    }";
    let t = parse(src);
    let u = param_uses(&t, first_fn(&t), "x");
    // `= x` in first LetExpr and `x.unwrap_or(0)` in `else`; `x > a` and `x + a` are shadowed!
    assert_eq!(u.uses.len(), 2);
    assert_eq!(u.uses[0].start, src.find("= x").unwrap() + 2);
    assert_eq!(u.uses[1].start, src.find("x.unwrap_or").unwrap());
}

#[test]
fn test_match_arms_and_guards_shadowing() {
    let src = "fn f(x: i32, v: Option<i32>) -> i32 {\n\
        match v {\n\
            Some(x) if x > 0 => x,\n\
            Some(y) if let Ok(x) = check(x + y) => x,\n\
            None => x + 1,\n\
        }\n\
    }";
    let t = parse(src);
    let u = param_uses(&t, first_fn(&t), "x");
    // Arm 1: `Some(x)` shadows both guard and body.
    // Arm 2: `check(x + y)` uses param `x`, then `let Ok(x)` shadows arm body `=> x`.
    // Arm 3: `None` does not bind `x`, so `x + 1` uses param `x`.
    assert_eq!(u.uses.len(), 2);
    assert_eq!(u.uses[0].start, src.find("check(x").unwrap() + 6);
    assert_eq!(u.uses[1].start, src.find("None => x").unwrap() + 8);
}

#[test]
fn test_for_while_let_and_closure_shadowing() {
    let src = "fn f(x: Vec<i32>, y: i32) -> i32 {\n\
        let mut s = 0;\n\
        for x in x.iter() { s += *x; }\n\
        let c1 = |z: i32| z + y;\n\
        let c2 = |y: i32| y + 1;\n\
        c1(s) + c2(y)\n\
    }";
    let t = parse(src);
    let f = first_fn(&t);
    let ux = param_uses(&t, f, "x");
    assert_eq!(ux.uses.len(), 1);
    assert_eq!(ux.uses[0].start, src.find("x.iter()").unwrap());

    let uy = param_uses(&t, f, "y");
    // `z + y` in `c1` (captures `y`) and `c2(y)` at call site; `|y: i32| y + 1` is shadowed.
    assert_eq!(uy.uses.len(), 2);
    assert_eq!(uy.uses[0].start, src.find("z + y").unwrap() + 4);
    assert_eq!(uy.uses[1].start, src.find("c2(y)").unwrap() + 3);
}

#[test]
fn test_nested_fn_impl_and_const_block_isolation() {
    let src = "impl A {\n\
        fn outer(&self, x: usize) -> usize {\n\
            fn inner(x: usize) -> usize { x }\n\
            struct B;\n\
            impl B {\n\
                fn m(&self, x: usize) -> usize { self.0 + x }\n\
            }\n\
            const C: usize = { let x = 1; x };\n\
            let k = const { 1 + 2 };\n\
            self.val() + x + k\n\
        }\n\
    }";
    let t = parse(src);
    let outer = t.descendants(t.root()).find(|&n| t.kind(n) == K::Fn).unwrap();
    let uself = param_uses(&t, outer, "self");
    assert_eq!(uself.uses.len(), 1);
    assert_eq!(uself.uses[0].start, src.find("self.val()").unwrap());

    let ux = param_uses(&t, outer, "x");
    assert_eq!(ux.uses.len(), 1);
    assert_eq!(ux.uses[0].start, src.find("+ x +").unwrap() + 2);
}

#[test]
fn test_macro_token_tree_uncertain_uses() {
    let src = "fn f(&self, x: i32, y: i32) {\n\
        println!(\"val={x}, other={}\", y);\n\
        dbg!(self.field);\n\
        let x = 99;\n\
        println!(\"{x}\");\n\
    }";
    let t = parse(src);
    let f = first_fn(&t);

    let ux = param_uses(&t, f, "x");
    assert!(ux.uses.is_empty());
    // First `println!("{x}")` is before shadowing -> recorded; second after `let x = 99` -> shadowed!
    assert_eq!(slices(src, &ux.macro_uses), vec!["x"]);

    let uy = param_uses(&t, f, "y");
    assert!(uy.uses.is_empty());
    assert_eq!(slices(src, &uy.macro_uses), vec!["y"]);

    let us = param_uses(&t, f, "self");
    assert!(us.uses.is_empty());
    assert_eq!(slices(src, &us.macro_uses), vec!["self"]);
}

#[test]
fn test_own_returns_and_own_try_ops_cutoffs() {
    let src = "fn f(a: Result<i32, E>) -> Result<i32, E> {\n\
        let v = a?;\n\
        let _c = || -> Result<i32, E> { let q = b()?; return Ok(q); };\n\
        let _fut = async move { let q = c()?; return Ok::<_, E>(q); };\n\
        fn helper() -> Result<i32, E> { d()?; return Ok(1); }\n\
        let t: Result<i32, E> = try {\n\
            let inside_try = e()?;\n\
            if inside_try < 0 { return Err(make_err()?); }\n\
            inside_try\n\
        };\n\
        unsafe { if v == 0 { return t; } }\n\
        Ok(t?)\n\
    }";
    let t = parse(src);
    let f = first_fn(&t);

    let rets: Vec<&str> = own_returns(&t, f).into_iter().map(|n| t.text(n)).collect();
    // `return` inside `try { ... }` and `unsafe { ... }` belongs to `f`, while closure/async/helper do not!
    assert_eq!(rets, vec!["return Err(make_err()?)", "return t"]);

    let tries: Vec<&str> = own_try_ops(&t, f).into_iter().map(|n| t.text(n)).collect();
    // `?` inside `try { ... }` (`e()?` and `make_err()?`) is caught by `try` block; `a?` and `t?` belong to `f`!
    assert_eq!(tries, vec!["a?", "t?"]);
}

#[test]
fn test_tail_expr_variants() {
    for (src, expected) in [
        ("fn a() -> i32 { 1 + 2 }", Some("1 + 2")),
        ("fn b() { 1 + 2; }", None),
        ("fn c() {}", None),
        ("fn d();", None),
        ("fn e(x: i32) -> String { format!(\"{x}\") }", Some("format!(\"{x}\")")),
        ("fn g(x: i32) { println!(\"{x}\"); }", None),
        ("fn h() -> i32 { let x = 1; #[allow(unused)] x }", Some("x")),
    ] {
        let t = parse(src);
        let got = tail_expr(&t, first_fn(&t)).map(|n| t.text(n));
        assert_eq!(got, expected, "{src}");
    }
    let decl = parse("trait T { fn decl(&self); }");
    assert_eq!(fn_body(&decl, first_fn(&decl)), None);

    // Block-local item at bottom of block shadows parameter across the entire block, even before `let x`:
    let hoisted = parse("fn f(x: i32) { consume(x); let x = 1; consume(x); fn x() {} }");
    assert!(param_uses(&hoisted, first_fn(&hoisted), "x").uses.is_empty());
}
