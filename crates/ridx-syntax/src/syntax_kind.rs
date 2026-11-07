/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The kinds of syntax tree nodes. Tokens are not nodes: a node covers a range of lexer tokens.

/// Kind of a [`crate::tree::SyntaxTree`] node.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SyntaxKind {
    /// Placeholder of an abandoned or not yet completed marker; never in a built tree.
    Tombstone,
    SourceFile,
    Error,
    Attr,
    Visibility,
    Name,
    TokenTree,
    Abi,

    // Items.
    Fn,
    ParamList,
    Param,
    SelfParam,
    RetType,
    WhereClause,
    WherePred,
    GenericParamList,
    LifetimeParam,
    TypeParam,
    ConstParam,
    Impl,
    Trait,
    Struct,
    Enum,
    Union,
    TypeAlias,
    Const,
    Static,
    Module,
    Use,
    ExternCrate,
    ExternBlock,
    MacroRules,
    MacroCall,
    ItemList,

    // Types and paths.
    PathType,
    RefType,
    PtrType,
    TupleType,
    ArrayType,
    SliceType,
    FnPtrType,
    ImplTraitType,
    DynTraitType,
    NeverType,
    InferType,
    ParenType,
    TypeBoundList,
    TypeBound,
    ForBinder,
    Path,
    PathSegment,
    GenericArgList,
    TypeArg,
    LifetimeArg,
    ConstArg,
    AssocTypeArg,
    Lifetime,

    // Patterns.
    IdentPat,
    WildcardPat,
    RefPat,
    TuplePat,
    TupleStructPat,
    RecordPat,
    RecordPatField,
    PathPat,
    LiteralPat,
    RangePat,
    OrPat,
    SlicePat,
    RestPat,
    BoxPat,
    MacroPat,
    ParenPat,
    ConstBlockPat,

    // Expressions.
    Literal,
    PathExpr,
    CallExpr,
    ArgList,
    MethodCallExpr,
    FieldExpr,
    IndexExpr,
    TryExpr,
    AwaitExpr,
    PrefixExpr,
    RefExpr,
    BinExpr,
    CastExpr,
    RangeExpr,
    ParenExpr,
    TupleExpr,
    ArrayExpr,
    BlockExpr,
    IfExpr,
    MatchExpr,
    MatchArmList,
    MatchArm,
    MatchGuard,
    LoopExpr,
    WhileExpr,
    ForExpr,
    ReturnExpr,
    BreakExpr,
    ContinueExpr,
    ClosureExpr,
    RecordExpr,
    RecordExprFieldList,
    RecordExprField,
    LetExpr,
    Label,
    UnderscoreExpr,
    YieldExpr,
    BecomeExpr,

    // Statements.
    LetStmt,
    LetElse,
    ExprStmt,
}

const F_EXPR: u8 = 1 << 0;
const F_PAT: u8 = 1 << 1;
const F_TYPE: u8 = 1 << 2;
const F_ITEM: u8 = 1 << 3;
const F_BLOCK_LIKE: u8 = 1 << 4;
const F_CF_CUT: u8 = 1 << 5;
const F_SEMA_SKIP: u8 = 1 << 6;

const KIND_FLAGS: [u8; 256] = {
    use SyntaxKind as K;
    let mut t = [0u8; 256];

    // Expressions: Literal..=BecomeExpr excluding non-expr helper nodes.
    let mut i = K::Literal as usize;
    while i <= K::BecomeExpr as usize {
        t[i] |= F_EXPR;
        i += 1;
    }
    t[K::ArgList as usize] &= !F_EXPR;
    t[K::MatchArmList as usize] &= !F_EXPR;
    t[K::MatchArm as usize] &= !F_EXPR;
    t[K::MatchGuard as usize] &= !F_EXPR;
    t[K::RecordExprFieldList as usize] &= !F_EXPR;
    t[K::RecordExprField as usize] &= !F_EXPR;
    t[K::Label as usize] &= !F_EXPR;

    // Patterns: IdentPat..=ConstBlockPat excluding RecordPatField.
    let mut p = K::IdentPat as usize;
    while p <= K::ConstBlockPat as usize {
        t[p] |= F_PAT;
        p += 1;
    }
    t[K::RecordPatField as usize] &= !F_PAT;

    // Types: PathType..=ParenType.
    let mut ty = K::PathType as usize;
    while ty <= K::ParenType as usize {
        t[ty] |= F_TYPE | F_CF_CUT | F_SEMA_SKIP;
        ty += 1;
    }

    // Items: Fn plus Impl..=MacroRules.
    t[K::Fn as usize] |= F_ITEM | F_CF_CUT | F_SEMA_SKIP;
    let mut it = K::Impl as usize;
    while it <= K::MacroRules as usize {
        t[it] |= F_ITEM | F_CF_CUT | F_SEMA_SKIP;
        it += 1;
    }

    // Block-like expressions.
    t[K::BlockExpr as usize] |= F_BLOCK_LIKE;
    t[K::IfExpr as usize] |= F_BLOCK_LIKE;
    t[K::MatchExpr as usize] |= F_BLOCK_LIKE;
    t[K::LoopExpr as usize] |= F_BLOCK_LIKE;
    t[K::WhileExpr as usize] |= F_BLOCK_LIKE;
    t[K::ForExpr as usize] |= F_BLOCK_LIKE;

    // Control-flow & sema subtree cutoffs.
    t[K::ClosureExpr as usize] |= F_CF_CUT;
    t[K::TokenTree as usize] |= F_CF_CUT | F_SEMA_SKIP;
    t[K::Attr as usize] |= F_CF_CUT | F_SEMA_SKIP;
    t[K::ConstBlockPat as usize] |= F_CF_CUT | F_SEMA_SKIP;
    t[K::Path as usize] |= F_CF_CUT | F_SEMA_SKIP;
    t[K::ParamList as usize] |= F_CF_CUT | F_SEMA_SKIP;
    t[K::GenericParamList as usize] |= F_CF_CUT | F_SEMA_SKIP;
    t[K::WhereClause as usize] |= F_CF_CUT | F_SEMA_SKIP;

    // Additional value-sema leaf/non-expr skip subtrees.
    t[K::Name as usize] |= F_SEMA_SKIP;
    t[K::Lifetime as usize] |= F_SEMA_SKIP;
    t[K::Label as usize] |= F_SEMA_SKIP;
    t[K::Visibility as usize] |= F_SEMA_SKIP;
    t[K::RetType as usize] |= F_SEMA_SKIP;
    t[K::GenericArgList as usize] |= F_SEMA_SKIP;

    t
};

impl SyntaxKind {
    /// Whether a node of this kind is an expression.
    #[inline(always)]
    pub const fn is_expr(self) -> bool {
        (KIND_FLAGS[self as usize] & F_EXPR) != 0
    }

    /// Whether a node of this kind is a pattern.
    #[inline(always)]
    pub const fn is_pat(self) -> bool {
        (KIND_FLAGS[self as usize] & F_PAT) != 0
    }

    /// Whether a node of this kind is a type.
    #[inline(always)]
    pub const fn is_type(self) -> bool {
        (KIND_FLAGS[self as usize] & F_TYPE) != 0
    }

    /// Whether a node of this kind is an item.
    #[inline(always)]
    pub const fn is_item(self) -> bool {
        (KIND_FLAGS[self as usize] & F_ITEM) != 0
    }

    /// Whether an expression of this kind ends a statement without a `;` (`if`, `match`, loops, blocks).
    #[inline(always)]
    pub const fn is_block_like(self) -> bool {
        (KIND_FLAGS[self as usize] & F_BLOCK_LIKE) != 0
    }

    #[inline(always)]
    pub(crate) const fn is_cf_cutoff(self) -> bool {
        (KIND_FLAGS[self as usize] & F_CF_CUT) != 0
    }

    #[inline(always)]
    pub(crate) const fn is_sema_skip(self) -> bool {
        (KIND_FLAGS[self as usize] & F_SEMA_SKIP) != 0
    }
}
