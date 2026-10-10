/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Syntactic position classification for `::`-scoped code completion.
//!
//! Restricts completion items based on syntactic context:
//! - Type positions (generics, return types, bounds, type annotations) -> types and associated types.
//! - Expression/pattern positions -> functions, constants, variants (excluding bare associated types).

use ridx_format::DefKind;

/// Syntactic position in Rust source where completion was requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxPosition {
    /// Type position (e.g. inside `<...>`, return type `->`, `where`, `as`, `&'a`, `: Type`).
    /// In this position, only types, associated types, traits, and modules are valid candidates.
    Type,
    /// Expression or pattern position (e.g. `let x =`, `if`, `match`, argument list, `unsafe {`).
    /// In this position, functions, associated functions, constants, and variants are valid candidates.
    /// Bare associated types (`AssocTy`) and traits cannot be expressions.
    ExprOrPat,
    /// Unconstrained or ambiguous position (e.g. `use` statements or unclassified context).
    Any,
}

/// Returns true if a definition kind is allowed in the given syntactic position.
pub fn is_def_allowed_in_position(kind: DefKind, pos: SyntaxPosition) -> bool {
    match pos {
        SyntaxPosition::Type => matches!(
            kind,
            DefKind::AssocTy
                | DefKind::Struct
                | DefKind::Enum
                | DefKind::Union
                | DefKind::Trait
                | DefKind::TyAlias
                | DefKind::ForeignTy
                | DefKind::TraitAlias
                | DefKind::TyParam
                | DefKind::OpaqueTy
                | DefKind::Mod
                | DefKind::ForeignMod
        ),
        // In expressions and patterns, traits and associated types can be referenced
        // as path qualifiers (e.g. `iter::Iterator::...` or `IntoSystem::System::...`).
        // Only fields are excluded because fields are accessed via `.` rather than `::`.
        SyntaxPosition::ExprOrPat => kind != DefKind::Field,
        SyntaxPosition::Any => true,
    }
}

/// Classifies the syntactic position of a `::`-scoped completion trigger from the prefix before `::`.
pub fn classify_colon_position(trimmed_before: &str) -> SyntaxPosition {
    let tb = trimmed_before.trim();
    if tb.starts_with("use ") || tb.starts_with("pub use ") {
        return SyntaxPosition::Any;
    }

    let stripped;
    let trimmed_before = if trimmed_before.contains("::<") {
        stripped = super::owner_completion::strip_turbofish(trimmed_before);
        &stripped
    } else {
        trimmed_before
    };

    let qual = super::owner_completion::extract_colon_qualifier(trimmed_before);
    let colon_idx = trimmed_before.rfind("::").unwrap_or(trimmed_before.len());
    let before_colon = trimmed_before[..colon_idx].trim_end();
    let prefix = if !qual.is_empty() && before_colon.ends_with(&qual) {
        before_colon[..before_colon.len() - qual.len()].trim_end()
    } else if before_colon.ends_with('>') {
        if let Some(ufcs_start) = super::owner_completion::find_ufcs_start(before_colon) {
            before_colon[..ufcs_start].trim_end()
        } else {
            before_colon
        }
    } else {
        before_colon
    };

    // 1. Unclosed angle brackets in prefix (ignoring arrows ->, =>, operators <<, >>, <=, >=, and comparison <)
    let mut angle_depth = 0i32;
    let bytes = prefix.as_bytes();
    let p_len = bytes.len();
    let mut i = 0;
    while i < p_len {
        let b = bytes[i];
        if b == b'-' && i + 1 < p_len && bytes[i + 1] == b'>' {
            i += 2;
            continue;
        }
        if b == b'=' && i + 1 < p_len && bytes[i + 1] == b'>' {
            i += 2;
            continue;
        }
        if b == b'<' {
            if i + 1 < p_len && (bytes[i + 1] == b'<' || bytes[i + 1] == b'=') {
                i += 2;
                continue;
            }
            // Distinguish generic opening bracket from comparison operator `<`.
            // Generic `<` is attached to an identifier (Option<), turbofish (::<),
            // or at the start of a type expression (<T as Trait>).
            // A comparison operator `<` has whitespace before it in expressions (a < b).
            let is_generic = if i == 0
                || bytes[i - 1].is_ascii_alphanumeric()
                || bytes[i - 1] == b'_'
                || (i >= 2 && &bytes[i - 2..i] == b"::")
            {
                true
            } else {
                let before = prefix[..i].trim_end();
                (before.ends_with(':') && !before.ends_with("::"))
                    || before.ends_with("->")
                    || before.ends_with("as")
                    || before.ends_with("where")
            };

            if is_generic {
                angle_depth += 1;
            }
        } else if b == b'>' {
            if i + 1 < p_len && (bytes[i + 1] == b'>' || bytes[i + 1] == b'=') {
                i += 2;
                continue;
            }
            angle_depth = (angle_depth - 1).max(0);
        }
        i += 1;
    }

    if angle_depth > 0 {
        return SyntaxPosition::Type;
    }

    // 2. Return type arrow `->` (not followed by `{` or `;`)
    if let Some(arrow_idx) = prefix.rfind("->") {
        let after_arrow = &prefix[arrow_idx + 2..];
        if !after_arrow.contains('{') && !after_arrow.contains(';') {
            return SyntaxPosition::Type;
        }
    }

    // 3. Reference with lifetime: `&'a `, `&'s `, `&'static `, etc.
    if let Some(amp_idx) = prefix.rfind("&'") {
        let after_amp = &prefix[amp_idx..];
        if !after_amp.contains(';') && !after_amp.contains('{') && !after_amp.contains('}') {
            return SyntaxPosition::Type;
        }
    }

    // 4. Reference with or without mut
    let trimmed_p = prefix.trim();
    if trimmed_p.ends_with('&') || trimmed_p.ends_with("&mut") {
        let amp_idx = if trimmed_p.ends_with("&mut") {
            trimmed_p.len() - 4
        } else {
            trimmed_p.len() - 1
        };
        let before_amp = trimmed_p[..amp_idx].trim_end();
        if (before_amp.ends_with(':') && !before_amp.ends_with("::")) || before_amp.ends_with("->") {
            return SyntaxPosition::Type;
        }
        if before_amp.ends_with('=')
            || before_amp.ends_with('(')
            || before_amp.ends_with('[')
            || before_amp.ends_with(',')
        {
            return SyntaxPosition::ExprOrPat;
        }
    }

    // 5. Type keywords: `as`, `where`, `dyn`, `impl`
    let last_word = trimmed_p.split_whitespace().next_back().unwrap_or("");
    if matches!(last_word, "as" | "where" | "dyn" | "impl") {
        return SyntaxPosition::Type;
    }

    // 6. Type alias definition: `type Foo = `
    if prefix.contains("type ") && prefix.contains('=') {
        if let Some(eq_idx) = prefix.rfind('=') {
            let after_eq = &prefix[eq_idx + 1..];
            if !after_eq.contains(';') && !after_eq.contains('{') {
                return SyntaxPosition::Type;
            }
        }
    }

    // 7. Colon for type annotation
    if trimmed_p.ends_with(':') || trimmed_p.ends_with(": &") || trimmed_p.ends_with(": &mut") {
        if prefix.contains('{') {
            let last_brace = prefix.rfind('{').unwrap_or(0);
            let before_brace = prefix[..last_brace].trim();
            if before_brace.starts_with("struct")
                || before_brace.contains(" struct ")
                || before_brace.starts_with("enum")
                || before_brace.contains(" enum ")
                || before_brace.starts_with("union")
                || before_brace.contains(" union ")
            {
                return SyntaxPosition::Type;
            } else {
                return SyntaxPosition::ExprOrPat;
            }
        }
        if trimmed_p.starts_with("let ")
            || trimmed_p.contains(" let ")
            || trimmed_p.starts_with("const ")
            || trimmed_p.contains(" const ")
            || trimmed_p.starts_with("static ")
            || trimmed_p.contains(" static ")
            || trimmed_p.starts_with("pub ")
            || trimmed_p.contains('(')
            || trimmed_p.contains("where ")
        {
            return SyntaxPosition::Type;
        }
        return SyntaxPosition::Any;
    }

    // 8. Expression / pattern indicators
    if trimmed_p.ends_with('=')
        || trimmed_p.ends_with("=>")
        || trimmed_p.ends_with('(')
        || trimmed_p.ends_with('[')
        || trimmed_p.ends_with(',')
        || trimmed_p.ends_with('<')
        || trimmed_p.ends_with('>')
        || trimmed_p.ends_with('+')
        || trimmed_p.ends_with('*')
        || trimmed_p.ends_with('/')
        || trimmed_p.ends_with('%')
        || trimmed_p.ends_with('!')
    {
        return SyntaxPosition::ExprOrPat;
    }

    if trimmed_p.ends_with('{') {
        let last_brace = prefix.rfind('{').unwrap_or(0);
        let before_brace = prefix[..last_brace].trim();
        if before_brace.starts_with("struct")
            || before_brace.contains(" struct ")
            || before_brace.starts_with("enum")
            || before_brace.contains(" enum ")
            || before_brace.starts_with("union")
            || before_brace.contains(" union ")
        {
            return SyntaxPosition::Type;
        } else {
            return SyntaxPosition::ExprOrPat;
        }
    }

    if matches!(
        last_word,
        "if" | "match" | "return" | "while" | "for" | "unsafe"
    ) {
        return SyntaxPosition::ExprOrPat;
    }

    if prefix.is_empty() {
        return SyntaxPosition::ExprOrPat;
    }

    SyntaxPosition::Any
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_colon_position() {
        // Type positions
        assert_eq!(classify_colon_position("Option<Self::"), SyntaxPosition::Type);
        assert_eq!(classify_colon_position("    state: &'s Self::"), SyntaxPosition::Type);
        assert_eq!(classify_colon_position("    fn reduce<F>(self, f: F) -> Option<Self::"), SyntaxPosition::Type);
        assert_eq!(classify_colon_position("    ) -> Self::"), SyntaxPosition::Type);
        assert_eq!(classify_colon_position("    fn serialize_unit_struct(...) -> Result<Self::Ok, Self::"), SyntaxPosition::Type);
        assert_eq!(classify_colon_position("let x: Self::"), SyntaxPosition::Type);
        assert_eq!(classify_colon_position("const C: Self::"), SyntaxPosition::Type);
        assert_eq!(classify_colon_position("static S: Self::"), SyntaxPosition::Type);
        assert_eq!(classify_colon_position("type Target = Self::"), SyntaxPosition::Type);
        assert_eq!(classify_colon_position("where Self::"), SyntaxPosition::Type);
        assert_eq!(classify_colon_position("x as Self::"), SyntaxPosition::Type);
        assert_eq!(classify_colon_position("dyn Self::"), SyntaxPosition::Type);
        assert_eq!(classify_colon_position("fn foo(x: Self::"), SyntaxPosition::Type);
        assert_eq!(classify_colon_position("let x: &Self::"), SyntaxPosition::Type);
        assert_eq!(classify_colon_position("let x: <T as Tr>::"), SyntaxPosition::Type);
        assert_eq!(classify_colon_position("type Target = <T as Tr>::"), SyntaxPosition::Type);

        // Expression / Pattern positions
        assert_eq!(classify_colon_position("let x = Self::"), SyntaxPosition::ExprOrPat);
        assert_eq!(classify_colon_position("let x = &Self::"), SyntaxPosition::ExprOrPat);
        assert_eq!(classify_colon_position("let v = foo::bar(&Self::"), SyntaxPosition::ExprOrPat);
        assert_eq!(classify_colon_position("call(&Foo::"), SyntaxPosition::ExprOrPat);
        assert_eq!(classify_colon_position("if a < Self::"), SyntaxPosition::ExprOrPat);
        assert_eq!(classify_colon_position("while i < Self::"), SyntaxPosition::ExprOrPat);
        assert_eq!(classify_colon_position("x < Foo::"), SyntaxPosition::ExprOrPat);
        assert_eq!(classify_colon_position("unsafe { UniqueEntityEquivalentSlice::"), SyntaxPosition::ExprOrPat);
        assert_eq!(classify_colon_position("match x { Some(v) => Foo::"), SyntaxPosition::ExprOrPat);
        assert_eq!(classify_colon_position("foo(1, 2, Bar::"), SyntaxPosition::ExprOrPat);
        assert_eq!(classify_colon_position("Self::"), SyntaxPosition::ExprOrPat);
        assert_eq!(classify_colon_position("let query_1 = QueryState::<&A, Without<C>>::"), SyntaxPosition::ExprOrPat);
        assert_eq!(classify_colon_position("world.trigger(&mut EventTriggerState::<E>::"), SyntaxPosition::ExprOrPat);
        assert_eq!(classify_colon_position("Foo::<T>::"), SyntaxPosition::ExprOrPat);
        assert_eq!(classify_colon_position("fn foo<T: Foo::<X>::"), SyntaxPosition::Type);
        assert_eq!(
            classify_colon_position("unsafe { Slice::from_slice_unchecked(<&set::Slice<K>>::"),
            SyntaxPosition::ExprOrPat
        );
        assert_eq!(classify_colon_position("let x = <T as Tr>::"), SyntaxPosition::ExprOrPat);
        assert_eq!(classify_colon_position("<T as Tr>::"), SyntaxPosition::ExprOrPat);

        // Any / unconstrained
        assert_eq!(classify_colon_position("use core::cell::"), SyntaxPosition::Any);
        assert_eq!(classify_colon_position("use alloc::{string::"), SyntaxPosition::Any);
        assert_eq!(classify_colon_position("sparse: Default::"), SyntaxPosition::Any);
    }

    #[test]
    fn test_is_def_allowed_in_position() {
        assert!(is_def_allowed_in_position(DefKind::AssocTy, SyntaxPosition::Type));
        assert!(is_def_allowed_in_position(DefKind::Struct, SyntaxPosition::Type));
        assert!(!is_def_allowed_in_position(DefKind::AssocFn, SyntaxPosition::Type));
        assert!(!is_def_allowed_in_position(DefKind::Fn, SyntaxPosition::Type));
        assert!(!is_def_allowed_in_position(DefKind::Const, SyntaxPosition::Type));

        assert!(is_def_allowed_in_position(DefKind::AssocFn, SyntaxPosition::ExprOrPat));
        assert!(is_def_allowed_in_position(DefKind::Const, SyntaxPosition::ExprOrPat));
        assert!(is_def_allowed_in_position(DefKind::Variant, SyntaxPosition::ExprOrPat));
        assert!(is_def_allowed_in_position(DefKind::AssocTy, SyntaxPosition::ExprOrPat));
        assert!(is_def_allowed_in_position(DefKind::Trait, SyntaxPosition::ExprOrPat));
        assert!(!is_def_allowed_in_position(DefKind::Field, SyntaxPosition::ExprOrPat));
    }
}
