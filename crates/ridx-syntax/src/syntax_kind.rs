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

impl SyntaxKind {
    /// Whether a node of this kind is an expression.
    pub const fn is_expr(self) -> bool {
        (self as u8) >= (Self::Literal as u8)
            && (self as u8) <= (Self::BecomeExpr as u8)
            && !matches!(self, Self::ArgList | Self::MatchArmList | Self::MatchArm | Self::MatchGuard | Self::RecordExprFieldList | Self::RecordExprField | Self::Label)
    }

    /// Whether a node of this kind is a pattern.
    pub const fn is_pat(self) -> bool {
        (self as u8) >= (Self::IdentPat as u8) && (self as u8) <= (Self::ConstBlockPat as u8) && !matches!(self, Self::RecordPatField)
    }

    /// Whether a node of this kind is a type.
    pub const fn is_type(self) -> bool {
        (self as u8) >= (Self::PathType as u8) && (self as u8) <= (Self::ParenType as u8)
    }

    /// Whether a node of this kind is an item.
    pub const fn is_item(self) -> bool {
        matches!(
            self,
            Self::Fn
                | Self::Impl
                | Self::Trait
                | Self::Struct
                | Self::Enum
                | Self::Union
                | Self::TypeAlias
                | Self::Const
                | Self::Static
                | Self::Module
                | Self::Use
                | Self::ExternCrate
                | Self::ExternBlock
                | Self::MacroRules
        )
    }

    /// Whether an expression of this kind ends a statement without a `;` (`if`, `match`, loops, blocks).
    pub const fn is_block_like(self) -> bool {
        matches!(self, Self::BlockExpr | Self::IfExpr | Self::MatchExpr | Self::LoopExpr | Self::WhileExpr | Self::ForExpr)
    }
}
