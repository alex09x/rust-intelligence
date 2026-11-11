/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Names suggested for values, as rust-analyzer's `suggest_name` gives them: from a type written in the source
//! (`Arc<Notify>` → `notify`, `Vec<Item>` → `items`), and without clashes (`a`, `a1`, `a2`).

use super::action_generics_sub::split_top_level_commas;
use std::collections::{HashMap, HashSet};

/// Trait names left out of `dyn`/`impl` types.
const USELESS_TRAITS: &[&str] = &["Send", "Sync", "Copy", "Clone", "Eq", "PartialEq"];
/// Names never suggested.
const USELESS_NAMES: &[&str] = &["new", "default", "option", "some", "none", "ok", "err", "str", "string", "from", "into"];
const USELESS_NAME_PREFIXES: &[&str] = &["from_", "with_", "into_"];
/// Types named after their first argument.
const WRAPPER_TYPES: &[&str] = &["Box", "Arc", "Rc", "Option", "Result"];
/// Types named after the plural of their first argument.
const SEQUENCE_TYPES: &[&str] = &["Vec", "VecDeque", "LinkedList"];
/// Primitive types: not ADTs, so they name nothing.
const PRIMITIVES: &[&str] = &["bool", "char", "str", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize", "f32", "f64"];

/// `snake_case` of a `CamelCase` or `snake_case` name, as rust-analyzer's `to_lower_snake_case`.
pub(crate) fn to_lower_snake_case(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() + 4);
    for (i, &c) in chars.iter().enumerate() {
        if c.is_uppercase() {
            let prev = i.checked_sub(1).map(|p| chars[p]);
            let next = chars.get(i + 1).copied();
            let boundary = prev.is_some_and(|p| p.is_lowercase() || p.is_ascii_digit() || (p.is_uppercase() && next.is_some_and(char::is_lowercase)));
            if boundary && !out.ends_with('_') {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn is_keyword(s: &str) -> bool {
    matches!(
        s,
        "as" | "break"
            | "const"
            | "continue"
            | "crate"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "async"
            | "await"
            | "dyn"
            | "abstract"
            | "become"
            | "box"
            | "do"
            | "final"
            | "macro"
            | "override"
            | "priv"
            | "typeof"
            | "unsized"
            | "virtual"
            | "yield"
            | "try"
            | "gen"
    )
}

/// rust-analyzer's `normalize`: snake case, and nothing uninformative or invalid.
pub(crate) fn normalize(name: &str) -> Option<String> {
    let name = to_lower_snake_case(name);
    let valid = name.starts_with(|c: char| c.is_alphabetic() || c == '_') && name.chars().all(|c| c.is_alphanumeric() || c == '_') && name != "_";
    if !valid || is_keyword(&name) || USELESS_NAMES.contains(&name.as_str()) || USELESS_NAME_PREFIXES.iter().any(|p| name.starts_with(p)) {
        return None;
    }
    Some(name)
}

/// The name for a value of the type written `ty`; `generics` are the type parameters in scope, which name nothing.
pub(crate) fn name_of_type(ty: &str, generics: &HashSet<String>) -> Option<String> {
    let t = ty.trim();
    // References name their pointee.
    if let Some(rest) = t.strip_prefix('&') {
        let rest = rest.trim_start();
        let rest = if rest.starts_with('\'') { rest.trim_start_matches(|c: char| c == '\'' || c.is_alphanumeric() || c == '_').trim_start() } else { rest };
        let rest = rest.strip_prefix("mut ").unwrap_or(rest);
        return name_of_type(rest, generics);
    }
    if let Some(inner) = t.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        // A slice names the plural of its items; an array names nothing.
        return (split_top_level_semicolon(inner).is_none()).then(|| sequence_name(inner, generics));
    }
    for kw in ["dyn ", "impl "] {
        if let Some(bounds) = t.strip_prefix(kw) {
            let names: Vec<String> = bounds
                .split('+')
                .map(str::trim)
                .filter(|b| !b.starts_with('\'') && !b.starts_with('?'))
                .filter_map(|b| last_segment(b).map(|(n, _)| n.to_string()))
                .filter(|n| !USELESS_TRAITS.contains(&n.as_str()))
                .collect();
            if kw == "impl " && names.len() != 1 {
                return None;
            }
            return names.first().and_then(|n| normalize(n));
        }
    }
    if t.starts_with('(') || t.starts_with("fn") || t.starts_with("unsafe ") || t.starts_with("extern ") || t == "!" || t.starts_with('*') || t.starts_with('<') {
        return None;
    }
    let (name, args) = last_segment(t)?;
    if PRIMITIVES.contains(&name) || generics.contains(name) || name == "Self" {
        return None;
    }
    if WRAPPER_TYPES.contains(&name) {
        return name_of_type(args.first()?, generics);
    }
    if SEQUENCE_TYPES.contains(&name) {
        return Some(args.first().map_or_else(|| "items".to_string(), |a| sequence_name(a, generics)));
    }
    normalize(name)
}

fn sequence_name(inner: &str, generics: &HashSet<String>) -> String {
    match name_of_type(inner, generics) {
        Some(n) if !n.ends_with(['s', 'x', 'y']) => format!("{n}s"),
        _ => "items".to_string(),
    }
}

fn split_top_level_semicolon(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth -= 1,
            ';' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

/// The last path segment of a type path and its type arguments (`a::B<C, D>` → `B`, [`C`, `D`]); lifetimes and
/// const arguments are left out.
fn last_segment(t: &str) -> Option<(&str, Vec<String>)> {
    let (path, args) = match t.find('<') {
        Some(lt) if t.ends_with('>') => (&t[..lt], &t[lt + 1..t.len() - 1]),
        Some(_) => return None,
        None => (t, ""),
    };
    let name = path.trim().trim_end_matches("::").rsplit("::").next()?.trim();
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    let args = split_top_level_commas(args).into_iter().map(|a| a.trim().to_string()).filter(|a| !a.is_empty() && !a.starts_with('\'') && !a.contains('=')).collect();
    Some((name, args))
}

/// Suggests names that do not clash with one another or with names already taken, as rust-analyzer's
/// `NameGenerator`: a taken `a` gives `a1`, then `a2`.
#[derive(Default)]
pub(crate) struct NameGenerator {
    pool: HashMap<String, usize>,
}

impl NameGenerator {
    fn split_numeric_suffix(name: &str) -> (&str, Option<usize>) {
        let pos = name.rfind(|c: char| !c.is_ascii_digit()).map_or(0, |p| p + name[p..].chars().next().map_or(1, char::len_utf8));
        let (prefix, suffix) = name.split_at(pos);
        (prefix, suffix.parse().ok())
    }

    pub(crate) fn suggest_name(&mut self, name: &str) -> String {
        let (prefix, suffix) = Self::split_numeric_suffix(name);
        let suffix = suffix.unwrap_or(0);
        match self.pool.get_mut(prefix) {
            None => {
                self.pool.insert(prefix.to_string(), suffix);
                name.to_string()
            }
            Some(count) => {
                *count = (*count + 1).max(suffix);
                format!("{prefix}{count}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{name_of_type, to_lower_snake_case, NameGenerator};
    use std::collections::HashSet;

    #[test]
    fn test_names_types_like_rust_analyzer() {
        let g: HashSet<String> = ["T".to_string()].into();
        let n = |t: &str| name_of_type(t, &g);
        assert_eq!(n("Arc<Notify>").as_deref(), Some("notify"));
        assert_eq!(n("Option<Box<dyn Error + Send>>").as_deref(), Some("error"));
        assert_eq!(n("Vec<Item>").as_deref(), Some("items"));
        assert_eq!(n("Vec<Name>").as_deref(), Some("names"));
        assert_eq!(n("&'a mut HashMap<K, V>").as_deref(), Some("hash_map"));
        assert_eq!(n("&[Story]").as_deref(), Some("items"));
        assert_eq!(n("[usize; 64]"), None);
        assert_eq!(n("u32"), None);
        assert_eq!(n("String"), None);
        assert_eq!(n("T"), None);
        assert_eq!(n("(u8, u8)"), None);
        assert_eq!(to_lower_snake_case("HTTPServer"), "http_server");
    }

    #[test]
    fn test_generator_suffixes_clashing_names() {
        let mut g = NameGenerator::default();
        assert_eq!(g.suggest_name("a"), "a");
        assert_eq!(g.suggest_name("a"), "a1");
        assert_eq!(g.suggest_name("b2"), "b2");
        assert_eq!(g.suggest_name("b"), "b3");
        assert_eq!(g.suggest_name("_0"), "_0");
    }
}
