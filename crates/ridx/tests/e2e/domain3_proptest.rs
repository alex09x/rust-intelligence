/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: Deterministic property-based tests and fuzzing for incremental edits,
//! coordinate mapping monotonicity, round-trip bijectivity, and panic-freedom.

use ridx::overlay::DocumentOverlay;
use ridx_test_support::apply_text_edits;
use serde_json::json;

struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Self(if seed == 0 { 0x853c49e6748fea9b } else { seed })
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn gen_range(&mut self, min: usize, max: usize) -> usize {
        if min >= max {
            return min;
        }
        min + (self.next_u64() as usize % (max - min))
    }
}

/// Computes all valid UTF-16 column boundaries on a line, including character boundaries,
/// EOL, and past-EOL clamping columns, strictly avoiding midpoints of surrogate pairs.
fn get_valid_cols_for_line(content: &str, target_line: u32) -> Vec<u32> {
    let mut cur_line = 0;
    let mut cols = vec![0];
    let mut cur_col_utf16 = 0;
    let mut line_found = false;

    for c in content.chars() {
        if cur_line == target_line {
            line_found = true;
            if c == '\r' || c == '\n' {
                break;
            }
            cur_col_utf16 += c.len_utf16() as u32;
            cols.push(cur_col_utf16);
        } else if c == '\n' {
            cur_line += 1;
        }
    }
    let eol_col = if line_found { cur_col_utf16 } else { 0 };
    for extra in 1..=5 {
        cols.push(eol_col + extra);
    }
    cols
}

#[test]
fn test_proptest_random_incremental_edits_vs_oracle() {
    let mut rng = Rng::new(0xABCD1234F00D);
    let snippets = [
        "let x = 1;\n",
        "fn calc() {}\n",
        "42",
        "/* comment */\n",
        "foo_bar",
        "\n\n",
        "🦀",
        "grüßen",
        "\r\n",
        "δ",
        "/* 🦀 */\r\n",
        "fn test_δ() -> bool { true }\r\n",
        "",
    ];

    let initial_templates = [
        "fn initial() {\n    let start = 0;\n}\n",
        "fn initial() {\r\n    let start = 0;\r\n}\r\n",
        "// Unicode: 🦀 and grüßen\r\nfn run() {\r\n    let δ = 1;\r\n}\r\n",
        "/* Greek test: δ */\nfn math() -> i32 {\n    42\n}\n",
    ];

    for round in 0..50 {
        let initial = initial_templates[round % initial_templates.len()];
        let mut oracle_text = initial.to_string();
        let mut overlay = DocumentOverlay::new(
            format!("file:///test_{round}.rs"),
            format!("/test_{round}.rs"),
            1,
            oracle_text.clone(),
        );

        for step in 0..10 {
            let num_lines = oracle_text.matches('\n').count() + 1;
            let sl = rng.gen_range(0, num_lines) as u32;
            let el = rng.gen_range(sl as usize, num_lines) as u32;

            let start_cols = get_valid_cols_for_line(&oracle_text, sl);
            let end_cols = if el == sl {
                start_cols.clone()
            } else {
                get_valid_cols_for_line(&oracle_text, el)
            };

            let sc_idx = rng.gen_range(0, start_cols.len());
            let sc = start_cols[sc_idx];

            let ec = if el == sl {
                let ec_idx = rng.gen_range(sc_idx, end_cols.len());
                end_cols[ec_idx]
            } else {
                let ec_idx = rng.gen_range(0, end_cols.len());
                end_cols[ec_idx]
            };

            let ins = snippets[rng.gen_range(0, snippets.len())];

            let edit = json!({
                "range": {
                    "start": { "line": sl, "character": sc },
                    "end": { "line": el, "character": ec }
                },
                "newText": ins,
                "text": ins
            });

            oracle_text = apply_text_edits(&oracle_text, std::slice::from_ref(&edit));
            overlay.apply_content_changes(step + 2, std::slice::from_ref(&edit));

            assert_eq!(
                overlay.text, oracle_text,
                "Mismatch on round {round}, step {step}: edit={edit:?}"
            );
        }
    }
}

#[test]
fn test_proptest_mapping_never_panics_on_arbitrary_inputs() {
    let mut rng = Rng::new(0xCAFEBABEDEAD);
    let words = [
        "alpha", "beta", "gamma", "delta", "fn", "struct", "let", "impl",
    ];

    for _ in 0..2000 {
        let num_lines_live = rng.gen_range(0, 25);
        let num_lines_comp = rng.gen_range(0, 25);

        let live_text: String = (0..num_lines_live)
            .map(|_| format!("{}\n", words[rng.gen_range(0, words.len())]))
            .collect();
        let comp_text: String = (0..num_lines_comp)
            .map(|_| format!("{}\n", words[rng.gen_range(0, words.len())]))
            .collect();

        let mut ov = DocumentOverlay::new("file:///p.rs", "/p.rs", 1, live_text);
        ov.advance_compiled_baseline(comp_text);

        // Test boundary queries: 0, 1, len, out-of-range, and huge u32
        let test_lines = [
            0,
            1,
            2,
            num_lines_live as u32,
            (num_lines_live + 5) as u32,
            99999,
            u32::MAX,
        ];
        for &l in &test_lines {
            let _ = ov.map_line_to_compiled(l);
            let _ = ov.map_live_to_compiled(l, 0);
        }

        let test_comp_lines = [
            0,
            1,
            2,
            num_lines_comp as u32,
            (num_lines_comp + 5) as u32,
            99999,
            u32::MAX,
        ];
        for &cl in &test_comp_lines {
            let _ = ov.map_compiled_line_to_live(cl);
            let _ = ov.map_compiled_to_live(cl, 0);
        }
    }
}

#[test]
fn test_proptest_mapping_monotonicity_invariant() {
    let mut rng = Rng::new(0x1337BEEF9999);
    let lines_pool = ["line_a\n", "line_b\n", "line_c\n", "line_d\n", "line_e\n"];

    for _ in 0..500 {
        let count_live = rng.gen_range(2, 15);
        let count_comp = rng.gen_range(2, 15);

        let live: String = (0..count_live)
            .map(|_| lines_pool[rng.gen_range(0, lines_pool.len())])
            .collect();
        let comp: String = (0..count_comp)
            .map(|_| lines_pool[rng.gen_range(0, lines_pool.len())])
            .collect();

        let mut ov = DocumentOverlay::new("file:///p.rs", "/p.rs", 1, live);
        ov.advance_compiled_baseline(comp);

        let mut prev_comp: Option<u32> = None;
        for live_line in 1..=count_live as u32 {
            if let Some(comp_line) = ov.map_line_to_compiled(live_line) {
                if let Some(prev) = prev_comp {
                    assert!(
                        comp_line > prev,
                        "Monotonicity violation: live {live_line} mapped to {comp_line}, but previous was {prev}"
                    );
                }
                prev_comp = Some(comp_line);
            }
        }
    }
}

#[test]
fn test_proptest_mapping_roundtrip_bijectivity() {
    let mut rng = Rng::new(0x7777AAAA5555);

    for _ in 0..500 {
        let prefix_count = rng.gen_range(1, 6);
        let live_mid_count = rng.gen_range(1, 6);
        let comp_mid_count = rng.gen_range(1, 6);
        let suffix_count = rng.gen_range(1, 6);

        let mut prefix = String::new();
        for p in 0..prefix_count {
            prefix.push_str(&format!("common_p_{p}\n"));
        }
        let mut suffix = String::new();
        for s in 0..suffix_count {
            suffix.push_str(&format!("common_s_{s}\n"));
        }
        let mut live_middle = String::new();
        for lm in 0..live_mid_count {
            live_middle.push_str(&format!("live_m_{lm}\n"));
        }
        let mut comp_middle = String::new();
        for cm in 0..comp_mid_count {
            comp_middle.push_str(&format!("comp_m_{cm}\n"));
        }

        let live = format!("{}{}{}", prefix, live_middle, suffix);
        let comp = format!("{}{}{}", prefix, comp_middle, suffix);

        let mut ov = DocumentOverlay::new("file:///p.rs", "/p.rs", 1, live);
        ov.advance_compiled_baseline(comp);

        // Lines in prefix
        for l in 1..=prefix_count {
            let cl = ov.map_line_to_compiled(l as u32).expect("prefix must map");
            let roundtrip = ov
                .map_compiled_line_to_live(cl)
                .expect("prefix must roundtrip");
            assert_eq!(roundtrip, l as u32);
        }

        // Lines in suffix
        let live_line_count = prefix_count + live_mid_count;
        for s_idx in 1..=suffix_count {
            let l = (live_line_count + s_idx) as u32;
            let cl = ov.map_line_to_compiled(l).expect("suffix must map");
            let roundtrip = ov
                .map_compiled_line_to_live(cl)
                .expect("suffix must roundtrip");
            assert_eq!(roundtrip, l);
        }
    }
}

#[test]
fn test_proptest_multibyte_random_edits_utf8_validity() {
    let mut rng = Rng::new(0x424299991212);
    let multibyte_tokens = [
        "🦀",
        "🚀",
        "grüßen",
        "nordic_å",
        "λ",
        "日本語",
        "Rust",
        " // 💡\n",
    ];

    let mut text = String::from("fn unicode_test() {\n");
    for _ in 0..10 {
        text.push_str(multibyte_tokens[rng.gen_range(0, multibyte_tokens.len())]);
        text.push(' ');
    }
    text.push_str("\n}\n");

    let mut ov = DocumentOverlay::new("file:///unicode.rs", "/unicode.rs", 1, text);

    for i in 0..10 {
        let ins = multibyte_tokens[rng.gen_range(0, multibyte_tokens.len())];
        let change = json!({
            "range": {
                "start": { "line": 1, "character": 0 },
                "end": { "line": 1, "character": 0 }
            },
            "text": ins
        });
        ov.apply_content_changes(i + 2, &[change]);

        // Assert text remains valid UTF-8 and line starts are properly computed
        assert!(std::str::from_utf8(ov.text.as_bytes()).is_ok());
        assert_eq!(
            ov.line_starts,
            ridx::overlay::mapping::compute_line_starts(&ov.text)
        );
    }
}
