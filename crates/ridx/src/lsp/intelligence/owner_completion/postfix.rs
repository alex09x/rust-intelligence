/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Where postfix snippets apply. rust-analyzer offers `let` and `letm` only when the expression being completed
//! is a whole statement: `recv.name(args);`, not `x = recv.name()` or `recv.name().more()`.

/// Whether the dot expression around the cursor is a statement of its own. `line` is the whole line, `before` the
/// text before the cursor up to the completed name's start (ending in `.`), `prev_line` the nearest non-blank line
/// above.
pub fn is_statement_expression(line: &str, before: &str, prev_line: Option<&str>) -> bool {
    let after: String = line.chars().skip(before.chars().count()).collect();
    receiver_starts_statement(before.trim_end().strip_suffix('.').unwrap_or(before), prev_line) && call_ends_statement(&after)
}

/// Whether the receiver before the dot starts a statement: nothing but `{`, `;` or `}` precedes it on its line.
fn receiver_starts_statement(receiver_and_before: &str, prev_line: Option<&str>) -> bool {
    let chars: Vec<char> = receiver_and_before.chars().collect();
    let mut depth = 0i32;
    let mut at = chars.len();
    while at > 0 {
        let c = chars[at - 1];
        match c {
            ')' | ']' | '}' if depth >= 0 => depth += 1,
            '(' | '[' | '{' if depth > 0 => depth -= 1,
            '(' | '[' | '{' => break,
            _ if depth > 0 => {}
            c if c.is_alphanumeric() || matches!(c, '_' | '.' | ':' | '?' | '!' | '&' | '*' | '\'') => {}
            _ => break,
        }
        at -= 1;
    }
    let receiver: String = chars[at..].iter().collect();
    let lead: String = chars[..at].iter().collect();
    let lead = lead.trim();
    if lead.is_empty() && (receiver.trim().is_empty() || receiver.trim_start().starts_with('.')) {
        // A chain continued from the line above.
        return false;
    }
    if lead.is_empty() {
        return prev_line.is_none_or(|p| {
            let p = p.trim_end();
            p.is_empty() || p.ends_with(['{', ';', '}']) || p.trim_start().starts_with("//")
        });
    }
    lead.ends_with(['{', ';', '}'])
}

/// Whether the completed name, with its turbofish and call arguments, ends the statement.
fn call_ends_statement(after: &str) -> bool {
    let rest = after.trim_start_matches(|c: char| c.is_alphanumeric() || c == '_');
    let rest = skip_balanced(rest, "::<", '<', '>');
    let rest = skip_balanced(rest, "(", '(', ')');
    let rest = rest.trim_start();
    rest.is_empty() || rest.starts_with([';', '}']) || rest.starts_with("//")
}

/// `text` past a bracketed group starting with `open_tok`, or `text` unchanged when it does not start with one. An
/// unclosed group (arguments continued on the next lines) consumes the rest of the line.
fn skip_balanced<'a>(text: &'a str, open_tok: &str, open: char, close: char) -> &'a str {
    if !text.starts_with(open_tok) {
        return text;
    }
    let mut depth = 0i32;
    for (i, c) in text.char_indices() {
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return &text[i + c.len_utf8()..];
            }
        }
    }
    ""
}

#[cfg(test)]
mod tests {
    use super::is_statement_expression;

    #[test]
    fn test_let_applies_only_to_whole_statements() {
        let st = |line: &str, before: &str| is_statement_expression(line, before, Some("fn f() {"));
        assert!(st("    v.push(1);", "    v."));
        assert!(st("    self.items.clear()", "    self.items."));
        assert!(!st("    let n = v.len();", "    let n = v."));
        assert!(!st("    v.iter().count();", "    v."));
        assert!(!st("    f(v.len());", "    f(v."));
        assert!(!st("        .map(|x| x + 1);", "        ."));
        assert!(!is_statement_expression("    v.len();", "    v.", Some("    let n = foo")));
    }
}
