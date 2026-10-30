/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! A function named where it is passed on rather than called: `.map(f)`, `(a, f)`, `f.after(x)`.
//! rust-analyzer lists the function that does this among the callers of `f`; so does RIDX.

/// Whether the reference over `[start_col, end_col)` (1-based, end exclusive) of a function is a
/// use of the function `name` as a value in the body of a caller (the reference spells the name: a
/// reference over `u32` that a derive made for `serialize` is no use): an argument or element (`f` followed by
/// `,`, `)`, `]` or `}`), a receiver of a combinator method (`f.after(x)`), the end of a line inside
/// a multi-line call, or a closed turbofish (`f::<T>`). Not a use: an import (`use a::f;`), a
/// declaration (`fn f`), a line of an attribute (`#[derive(..)]`, whose generated code the index
/// attributes to the attribute), a bare assignment (`let p = f;`), an unclosed turbofish.
pub(crate) fn is_function_value_use_in_lines(lines: &[&str], name: &str, start_line: u32, start_col: u32, end_line: u32, end_col: u32) -> bool {
    let (Some(first), Some(last)) = (lines.get(start_line.saturating_sub(1) as usize), lines.get(end_line.saturating_sub(1) as usize)) else {
        return false;
    };
    if start_line != end_line || name.is_empty() {
        return false;
    }
    let spelled: String = first.chars().skip(start_col.saturating_sub(1) as usize).take(end_col.saturating_sub(start_col) as usize).collect();
    if !spelled.ends_with(name) {
        return false;
    }
    let before: String = first.chars().take(start_col.saturating_sub(1) as usize).collect();
    let head = first.trim_start();
    if head.starts_with("use ") || (head.starts_with("pub") && head.contains(" use ")) || head.starts_with("#[") || head.starts_with("#![") {
        return false;
    }
    let last_word = before.trim_end().rsplit(|c: char| !c.is_alphanumeric() && c != '_').next();
    if last_word == Some("fn") {
        return false;
    }
    let rest: String = last.chars().skip(end_col.saturating_sub(1) as usize).collect();
    let rest = rest.trim_start();
    match rest.chars().next() {
        None => true,
        Some(',' | ')' | ']' | '}' | '.') => true,
        _ if rest.starts_with("::<") => rest.contains('>'),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::is_function_value_use_in_lines;

    fn at(line: &str, name: &str) -> bool {
        let col = line.find(name).unwrap() as u32 + 1;
        is_function_value_use_in_lines(&[line], name, 1, col, 1, col + name.len() as u32)
    }

    #[test]
    fn a_reference_over_another_token_is_no_value_use_of_the_function() {
        let line = "    Var(u32),";
        let col = line.find("u32").unwrap() as u32 + 1;
        assert!(!is_function_value_use_in_lines(&[line], "serialize", 1, col, 1, col + 3));
        assert!(!is_function_value_use_in_lines(&[line], "", 1, col, 1, col + 3));
    }

    #[test]
    fn a_function_passed_on_is_a_value_use() {
        assert!(at("    xs.iter().map(requires_default).count();", "requires_default"));
        assert!(at("    app.add_systems(Update, (rotate, move_it));", "move_it"));
        assert!(at("    app.add_systems(Update, (rotate, move_it));", "rotate"));
        assert!(at("    system.in_set(Set::A);", "system"));
        assert!(at("    .and_then(Error::regex)", "regex"));
        assert!(at("        handler,", "handler"));
        assert!(at("    run(spawn::<u8>);", "spawn"));
    }

    #[test]
    fn an_import_a_declaration_an_assignment_and_an_unclosed_turbofish_are_no_value_use() {
        assert!(!at("use crate::tasks::spawn;", "spawn"));
        assert!(!at("#[derive(Deserialize)]", "Deserialize"));
        assert!(!at("    #[serde(with = \"spawn\")]", "spawn"));
        assert!(!at("pub use crate::tasks::spawn;", "spawn"));
        assert!(!at("    pub(crate) fn spawn() {}", "spawn"));
        assert!(!at("fn spawn() {}", "spawn"));
        assert!(!at("    let p = spawn;", "spawn"));
        assert!(!at("    run(spawn::<u8", "spawn"));
        assert!(!at("    spawn = other;", "spawn"));
    }
}
