/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The shape of the tree for the constructs code actions rely on, and recovery on broken input.

use ridx_syntax::{parse, NodeId, SyntaxKind as K, SyntaxTree};

/// The dump of the first node of `kind`.
fn first(tree: &SyntaxTree<'_>, kind: K) -> String {
    let id = tree.descendants(tree.root()).find(|&n| tree.kind(n) == kind).unwrap_or_else(|| panic!("no {kind:?} in {}", tree.dump(tree.root(), false)));
    tree.dump(id, true)
}

fn clean(src: &str) -> SyntaxTree<'_> {
    let tree = parse(src);
    assert_eq!(tree.errors().count(), 0, "errors in {}", tree.dump(tree.root(), true));
    tree
}

#[test]
fn test_fn_signature() {
    let t = clean("pub fn f<'a, T: Clone + 'a>(&'a mut self, x: &T, (a, b): (u8, u8)) -> impl Iterator<Item = T> + 'a where T: Send { x }");
    let f = first(&t, K::Fn);
    assert!(f.starts_with("Fn(Visibility\"pub\" Name\"f\" GenericParamList(LifetimeParam(Lifetime\"'a\") TypeParam(Name\"T\" TypeBoundList("), "{f}");
    assert!(f.contains("ParamList(SelfParam(Lifetime\"'a\") Param(IdentPat(Name\"x\") RefType(PathType(Path(PathSegment\"T\")))) Param(TuplePat("), "{f}");
    assert!(f.contains("RetType(ImplTraitType(TypeBoundList(TypeBound(Path(PathSegment(GenericArgList(AssocTypeArg(Name\"Item\""), "{f}");
    assert!(f.contains("WhereClause(WherePred("), "{f}");
    assert!(f.ends_with("BlockExpr(PathExpr(Path(PathSegment\"x\"))))"), "{f}");
}

#[test]
fn test_calls_and_method_chains() {
    let t = clean("fn f() { greet(make(), t.as_str()); Arg::new(\"n\").allow(true).build::<u8>(1, 2); }");
    let call = first(&t, K::CallExpr);
    assert_eq!(call, "CallExpr(PathExpr(Path(PathSegment\"greet\")) ArgList(CallExpr(PathExpr(Path(PathSegment\"make\")) ArgList\"()\") MethodCallExpr(PathExpr(Path(PathSegment\"t\")) Name\"as_str\" ArgList\"()\")))");
    let chain = t.descendants(t.root()).filter(|&n| t.kind(n) == K::MethodCallExpr).map(|n| t.text(n)).max_by_key(|s| s.len()).unwrap();
    assert_eq!(chain, "Arg::new(\"n\").allow(true).build::<u8>(1, 2)");
}

#[test]
fn test_precedence() {
    let t = clean("fn f() { let v = -a as u8 + b * c == d && e || g; }");
    let v = first(&t, K::LetStmt);
    // `||` at the top, then `&&`, then `==`, with `-a as u8` and `b * c` innermost.
    assert!(v.contains("BinExpr(BinExpr(BinExpr(BinExpr(CastExpr(PrefixExpr(PathExpr"), "{v}");
    assert!(v.contains("BinExpr(PathExpr(Path(PathSegment\"b\")) PathExpr(Path(PathSegment\"c\")))"), "{v}");
}

#[test]
fn test_closures_and_returns() {
    let t = clean("fn f(v: &[i32]) -> Option<i32> { v.iter().copied().find(|x| { if x % 2 == 0 { return true; } false }) }");
    let c = first(&t, K::ClosureExpr);
    assert!(c.starts_with("ClosureExpr(ParamList(Param(IdentPat(Name\"x\"))) BlockExpr(ExprStmt(IfExpr("), "closure: {c}");
    let ret = t.descendants(t.root()).find(|&n| t.kind(n) == K::ReturnExpr).unwrap();
    let owner = t.ancestors(ret).find(|&a| matches!(t.kind(a), K::ClosureExpr | K::Fn)).map(|a| t.kind(a));
    assert_eq!(owner, Some(K::ClosureExpr));
}

#[test]
fn test_let_shadowing_and_let_else() {
    let t = clean("fn norm(x: f64) -> f64 { let x = x.abs(); let Some(y) = g(x) else { return 0.0 }; x.sqrt() + y }");
    let lets: Vec<String> = t.descendants(t.root()).filter(|&n| t.kind(n) == K::LetStmt).map(|n| t.dump(n, true)).collect();
    assert_eq!(lets[0], "LetStmt(IdentPat(Name\"x\") MethodCallExpr(PathExpr(Path(PathSegment\"x\")) Name\"abs\" ArgList\"()\"))");
    assert!(lets[1].starts_with("LetStmt(TupleStructPat(Path(PathSegment\"Some\") IdentPat(Name\"y\")) CallExpr("), "{}", lets[1]);
    assert!(lets[1].ends_with("LetElse(BlockExpr(ReturnExpr(Literal\"0.0\"))))"), "{}", lets[1]);
}

#[test]
fn test_conditions_do_not_take_struct_literals() {
    let t = clean("fn f() { if let Some(x) = a && x == S { g(S { a: 1, b: ::m::C }) } else if c {} match v { A::B { .. } | C if d => {} _ => e, } }");
    let i = first(&t, K::IfExpr);
    assert!(i.starts_with("IfExpr(BinExpr(LetExpr(TupleStructPat("), "{i}");
    assert!(
        i.contains("RecordExpr(Path(PathSegment\"S\") RecordExprFieldList(RecordExprField(Literal\"1\") RecordExprField(PathExpr(Path(PathSegment\"m\" PathSegment\"C\")))))"),
        "{i}"
    );
    let arm = first(&t, K::MatchArm);
    assert!(arm.starts_with("MatchArm(OrPat(RecordPat(Path(PathSegment\"A\" PathSegment\"B\") RestPat\"..\") IdentPat(Name\"C\")) MatchGuard("), "{arm}");
}

#[test]
fn test_fields_ranges_and_macros() {
    let t = clean("fn f() { let a = t.0.1 + x[1..=n].len(); for i in 0..10 { assert_eq!(f(i), 1); } }");
    let a = first(&t, K::LetStmt);
    assert!(a.contains("FieldExpr(FieldExpr(PathExpr(Path(PathSegment\"t\"))))") || a.contains("FieldExpr(PathExpr(Path(PathSegment\"t\")))"), "{a}");
    assert!(a.contains("IndexExpr(PathExpr(Path(PathSegment\"x\")) RangeExpr(Literal\"1\" PathExpr(Path(PathSegment\"n\"))))"), "{a}");
    assert!(first(&t, K::ForExpr)
        .starts_with("ForExpr(IdentPat(Name\"i\") RangeExpr(Literal\"0\" Literal\"10\") BlockExpr(ExprStmt(MacroCall(Path(PathSegment\"assert_eq\") TokenTree"));
}

#[test]
fn test_block_like_statement_ends_at_the_brace() {
    let t = clean("fn f() { if c {} (a, b); match x {}.len(); }");
    let stmts: Vec<K> = t.children(t.descendants(t.root()).find(|&n| t.kind(n) == K::BlockExpr).unwrap()).map(|n| t.kind(n)).collect();
    assert_eq!(stmts, vec![K::ExprStmt, K::ExprStmt, K::ExprStmt]);
    assert_eq!(first(&t, K::TupleExpr), "TupleExpr(PathExpr(Path(PathSegment\"a\")) PathExpr(Path(PathSegment\"b\")))");
}

#[test]
fn test_items() {
    let t = clean("#![allow(x)]\nuse a::{b, c};\nimpl<T> Display for dyn Tr<T> + '_ { fn fmt(&self) {} }\ntrait A: B { type X<'a>: C where Self: 'a; const N: usize = 1; }\nmacro_rules! m { () => {} }\nstruct S<T>(T) where T: Copy;\nenum E { A, B(u8) }\n");
    let kinds: Vec<K> = t.children(t.root()).map(|n| t.kind(n)).collect();
    assert_eq!(kinds, vec![K::Attr, K::Use, K::Impl, K::Trait, K::MacroRules, K::Struct, K::Enum]);
    assert!(first(&t, K::Impl).contains("ItemList(Fn(Name\"fmt\" ParamList(SelfParam"));
}

#[test]
fn test_broken_input_is_covered() {
    for src in ["fn f() { let x = ; foo( }", "fn f(a: ) -> { a.", "impl X { fn", "fn f() { match x { A => } }", "}}}", "fn f() { |x| }"] {
        let t = parse(src);
        let root: NodeId = t.root();
        assert_eq!(t.node(root).end as usize, t.tokens.len(), "{src}");
        let text: String = t.tokens.iter().map(|tok| tok.text(src)).collect();
        assert_eq!(text, src);
    }
}
