/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Whether a reference to a tuple struct names its type or uses its constructor as a value (`.map(Name)`), which a
//! conversion to named fields cannot rewrite.

/// Whether the reference at `b` (its name ending at `after`) uses the constructor as a value (`.map(Name)`,
/// `let f = Name;`) rather than naming the type: only a type position is accepted.
pub(super) fn is_value_use(masked: &str, b: usize, after: usize) -> bool {
    let next = masked[after..].trim_start();
    if next.starts_with("::") || next.starts_with('<') {
        return false;
    }
    let stmt = masked[statement_start(masked, b)..b].trim_start();
    // An import, or a type alias (`type Output = Self;`).
    if ["use ", "pub use ", "pub(crate) use ", "type ", "pub type "].iter().any(|s| stmt.starts_with(s)) {
        return false;
    }
    if follows_type_prefix(masked, b) {
        return false;
    }
    // A field's attributes and visibility (`Variant(#[serde(…)] Name)`, `S(pub(crate) Name)`) say nothing of what the
    // field is.
    let prev = without_trailing_vis(without_trailing_attrs(path_prefix_stripped(masked, b)));
    // A reference type with a lifetime (`&'s Name`).
    if prev.rsplit(|c: char| !(c.is_alphanumeric() || c == '_' || c == '\'')).next().is_some_and(|t| t.starts_with('\'') && t.len() > 1) {
        return false;
    }
    // An array or slice type (`[Name; 64]`, `&[Name]`); an array of the constructor function would be one too, but no
    // code builds one.
    if prev.ends_with('[') && (next.starts_with(';') || next.starts_with(']')) {
        return false;
    }
    // An associated type's binding (`Iterator<Item = Name>`).
    if prev.ends_with('=') && unmatched_open_at(masked, b).is_some_and(|o| masked.as_bytes()[o] == b'<') {
        return false;
    }
    // An attribute's arguments (`#[require(A, Name)]`) name types.
    let mut at = b;
    while let Some(o) = unmatched_open_at(masked, at) {
        if masked.as_bytes()[o] == b'[' && masked[..o].trim_end().ends_with('#') || masked[..o].trim_end().ends_with("#!") {
            return false;
        }
        at = o;
    }
    if prev.ends_with(',') || prev.ends_with('(') {
        let Some(o) = unmatched_open_at(masked, b) else { return true };
        let before = masked[..o].trim_end();
        return match masked.as_bytes()[o] {
            b'<' => false,
            // An attribute's arguments (`#[require(A, Name)]`) name types.
            b'[' => !before.ends_with('#') && !before.ends_with("#!"),
            // `Fn(Name) -> R`, `fn(Name)`, a tuple variant's or tuple struct's fields (`Variant(Name)` in an `enum`
            // body, `struct W(Name)`), and a tuple type (`Option<(Name, &T)>`, `-> (Name, T)`, `x: (Name, T)`).
            b'(' => {
                !(["Fn", "FnMut", "FnOnce"].iter().any(|f| before.ends_with(f))
                    || ends_with_word(before, "fn")
                    || in_enum_body(masked, o)
                    || in_tuple_struct_decl(before)
                    || tuple_type_at(masked, o))
            }
            _ => true,
        };
    }
    true
}

/// Whether the parenthesis at `o` opens a tuple type: after a type prefix (`<`, `->`, `:`, `&`), or as an argument
/// of a generic list (`HashMap<K, (A, B)>`).
fn tuple_type_at(masked: &str, o: usize) -> bool {
    let before = masked[..o].trim_end();
    if before.ends_with(['<', ':', '&']) && !before.ends_with("::") || before.ends_with("->") {
        return true;
    }
    // A generic argument (`HashMap<K, (A, B)>`) or an associated type's binding (`Iterator<Item = (A, B)>`).
    (before.ends_with(',') || before.ends_with('=')) && unmatched_open_at(masked, o).is_some_and(|p| masked.as_bytes()[p] == b'<')
}

/// Where the statement holding `b` starts: past the last `;` or block, but not past the groups of an import
/// (`use a::{b::{C}, d::{Name}}`) that hold `b` or come before it.
fn statement_start(masked: &str, b: usize) -> usize {
    let mb = masked.as_bytes();
    let mut i = b;
    let mut groups: Vec<usize> = Vec::new();
    while i > 0 {
        i -= 1;
        match mb[i] {
            b';' if groups.is_empty() => return i + 1,
            b'}' => groups.push(i),
            b'{' => {
                let import_group = masked[..i].trim_end().ends_with("::");
                match groups.pop() {
                    // A closed group before `b`: a block ends the statement there; an import's group is skipped.
                    Some(close) if !import_group && groups.is_empty() => return close + 1,
                    Some(_) => {}
                    None if import_group => {}
                    None => return i + 1,
                }
            }
            _ => {}
        }
    }
    0
}

/// `prev` without the visibility it ends with (`pub`, `pub(crate)`, `pub(in a::b)`).
fn without_trailing_vis(prev: &str) -> &str {
    let t = match prev.strip_suffix(')') {
        Some(inner) => match inner.rfind('(') {
            Some(p) => prev[..p].trim_end(),
            None => return prev,
        },
        None => prev,
    };
    match t.strip_suffix("pub") {
        Some(rest) if !rest.ends_with(|c: char| c.is_alphanumeric() || c == '_') => rest.trim_end(),
        _ => prev,
    }
}

/// Whether `text` ends with the word `w`.
fn ends_with_word(text: &str, w: &str) -> bool {
    text.strip_suffix(w).is_some_and(|r| !r.ends_with(|c: char| c.is_alphanumeric() || c == '_'))
}

/// Whether the parenthesis after `before` opens a tuple struct's fields (`struct Name(`, `struct Name<T>(`).
fn in_tuple_struct_decl(before: &str) -> bool {
    let mut head = before;
    if head.ends_with('>') {
        let b = head.as_bytes();
        let mut depth = 0i32;
        let Some(open) = (0..b.len()).rev().find(|&i| {
            depth += i32::from(b[i] == b'>' && !(i > 0 && b[i - 1] == b'-')) - i32::from(b[i] == b'<');
            depth == 0
        }) else {
            return false;
        };
        head = head[..open].trim_end();
    }
    let name_start = head.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_').len();
    name_start < head.len() && ends_with_word(head[..name_start].trim_end(), "struct")
}

/// `prev` without the outer attributes (`#[…]`) it ends with.
fn without_trailing_attrs(mut prev: &str) -> &str {
    while prev.ends_with(']') {
        let b = prev.as_bytes();
        let mut depth = 0i32;
        let Some(open) = (0..b.len()).rev().find(|&i| {
            depth += i32::from(b[i] == b']') - i32::from(b[i] == b'[');
            depth == 0
        }) else {
            break;
        };
        let Some(before) = prev[..open].trim_end().strip_suffix('#') else { break };
        prev = before.trim_end();
    }
    prev
}

/// The text before `b` with a path qualifier (`a::b::`) left out.
fn path_prefix_stripped(masked: &str, b: usize) -> &str {
    let mut prev = masked[..b].trim_end();
    while let Some(p) = prev.strip_suffix("::") {
        prev = p.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_').trim_end();
    }
    prev
}

/// Whether what precedes `b` makes it a type (`: Name`, `-> Name`, `&Name`, `impl X for Name`).
pub(super) fn follows_type_prefix(masked: &str, b: usize) -> bool {
    let prev = path_prefix_stripped(masked, b);
    [":", "->", "<", "&", "mut", "as", "for", "impl", "dyn", "+", "'static"].iter().any(|p| prev.ends_with(p)) && !prev.ends_with("::")
}

/// Whether the parenthesis at `o` opens a tuple variant's fields: its enclosing block is an `enum`'s body.
fn in_enum_body(masked: &str, o: usize) -> bool {
    let Some(brace) = unmatched_open_at(masked, o).filter(|&p| masked.as_bytes()[p] == b'{') else { return false };
    let head = &masked[masked[..brace].rfind([';', '{', '}']).map_or(0, |p| p + 1)..brace];
    head.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|w| w == "enum")
}

/// The innermost bracket (`(`, `[`, `{` or a generic `<`) left open before `b` and its offset; `->` and `=>` are no
/// brackets.
fn unmatched_open_at(masked: &str, b: usize) -> Option<usize> {
    let mb = masked.as_bytes();
    let mut depth = 0i32;
    for i in (0..b).rev() {
        let c = mb[i];
        let arrow = c == b'>' && i > 0 && matches!(mb[i - 1], b'-' | b'=');
        match c {
            b')' | b']' | b'}' => depth += 1,
            b'>' if !arrow => depth += 1,
            b'(' | b'[' | b'{' | b'<' if depth == 0 => return Some(i),
            b'(' | b'[' | b'{' | b'<' => depth -= 1,
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::is_value_use;

    #[test]
    fn finds_value_uses() {
        let src = "let ids = v.map(Id); fn f(x: &Id) -> Option<Id> { None } use crate::m::{Id, Other}; let h: HashMap<u8, Id>;";
        let at = |n: usize| src.match_indices("Id").nth(n).unwrap().0;
        assert!(is_value_use(src, at(0), at(0) + 2), "a constructor passed as a function");
        assert!(!is_value_use(src, at(1), at(1) + 2) && !is_value_use(src, at(2), at(2) + 2), "types");
        assert!(!is_value_use(src, at(3), at(3) + 2), "an import");
        assert!(!is_value_use(src, at(4), at(4) + 2), "a generic argument");
        let src = "fn f(g: &impl Fn(Id) -> bool) {} #[require(A, Id)] struct C; enum E { V(Id), W } let x = Some(Id);";
        let at = |n: usize| src.match_indices("Id").nth(n).unwrap().0;
        assert!(!is_value_use(src, at(0), at(0) + 2), "a closure type's parameter");
        assert!(!is_value_use(src, at(1), at(1) + 2), "an attribute's type argument");
        assert!(!is_value_use(src, at(2), at(2) + 2), "a tuple variant's field");
        assert!(is_value_use(src, at(3), at(3) + 2), "a constructor as an argument");
        let src = "enum E { V(#[doc(alias = x)] Id,) } impl Neg for Id { type Output = Self; } struct L([Id; 64]); let f = [Id];";
        let at = |n: usize| src.match_indices("Id").nth(n).unwrap().0;
        assert!(!is_value_use(src, at(0), at(0) + 2), "a tuple variant's field after its attribute");
        let selfs = src.find("Self").unwrap();
        assert!(!is_value_use(src, selfs, selfs + 4), "a type alias");
        assert!(!is_value_use(src, at(2), at(2) + 2), "an array type");
        assert!(!is_value_use(src, at(3), at(3) + 2), "a one-element slice reads as a slice type");
        let src = "fn f(c: impl Iterator<Item = Id>) { let x = Id; }";
        let at = |n: usize| src.match_indices("Id").nth(n).unwrap().0;
        assert!(!is_value_use(src, at(0), at(0) + 2), "an associated type's binding");
        assert!(is_value_use(src, at(1), at(1) + 2), "a constructor bound to a variable");
        let src = concat!(
            "use crate::{\n    tick::{A, B},\n    component::{C, Id},\n};\n",
            "fn iter(&self) -> impl Iterator<Item = (Id, &X)> { todo!() }\n",
            "struct R { f: fn(&mut X, Id, u8), g: &'s Id }\n",
            "struct W(#[serde(with = x)] Id); pub struct K(pub(crate) Id); struct G<T>(Id, T);\n",
            "fn g() { if x { y } let v = [1].map(Id); }",
        );
        let at = |n: usize| src.match_indices("Id").nth(n).unwrap().0;
        let names = [
            "a nested import",
            "a tuple in a binding",
            "a fn pointer's parameter",
            "a lifetime reference",
            "a field after an attribute",
            "a field after its visibility",
            "a generic tuple struct's field",
        ];
        for (n, what) in names.iter().enumerate() {
            assert!(!is_value_use(src, at(n), at(n) + 2), "{what}");
        }
        assert!(is_value_use(src, at(7), at(7) + 2), "a constructor after a block");
    }
}
