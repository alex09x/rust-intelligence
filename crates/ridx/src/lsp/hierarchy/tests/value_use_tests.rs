/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! A function passed on as a value is a caller's use of it: incomingCalls lists the function whose body
//! names it as an argument or element, not the one that only assigns it to a variable.

use super::item_position_tests::incoming_callers_of;
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference, WorkspaceIndex};
use std::fs;

fn def(id: u32, name: &str, path: &str, file: &str) -> RawDefinition {
    RawDefinition {
        def_id: id,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: file.to_string(),
        name: name.to_string(),
        start_line: 1,
        start_col: 4,
        end_line: 1,
        end_col: 4 + name.len() as u32,
        path: path.to_string(),
        signature: Some("fn()".to_string()),
    }
}

fn reference(file: &str, target_file: &str, line: u32, col: u32, macro_made: bool) -> RawReference {
    RawReference {
        file: file.to_string(),
        start_line: line,
        start_col: col,
        end_line: line,
        end_col: col + 6,
        target_def_id: Some(1),
        target_path: "t::target".to_string(),
        target_unit: None,
        target_file: Some(target_file.to_string()),
        target_line: 1,
        target_col: 4,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: macro_made,
    }
}

#[test]
fn a_function_passed_as_an_argument_makes_its_caller_but_an_assignment_and_a_macro_made_use_do_not() {
    let dir = std::env::temp_dir().join(format!("ridx_value_use_{}", std::process::id()));
    let _ = fs::create_dir_all(&dir);
    let (a_file, b_file) = (dir.join("a.rs"), dir.join("b.rs"));
    fs::write(&a_file, "fn target() {}\n").unwrap();
    fs::write(&b_file, "fn holder() {\n    run(target);\n    let _p = target;\n    xs.iter().map(target).count();\n    made_by(target);\n}\n").unwrap();
    let (a, b) = (a_file.to_str().unwrap(), b_file.to_str().unwrap());

    let mut w = IndexWriter::new("t", "0.1.0");
    w.add_definition(def(1, "target", "t::target", a));
    w.add_definition(def(2, "holder", "t::holder", b));
    w.add_reference(reference(b, a, 2, 9, false));
    w.add_reference(reference(b, a, 3, 14, false));
    w.add_reference(reference(b, a, 4, 19, false));
    w.add_reference(reference(b, a, 5, 13, true));
    w.write_to(&mut fs::File::create(dir.join("t.ridx")).unwrap()).unwrap();

    let ws = WorkspaceIndex::open(&dir).unwrap();
    let callers = incoming_callers_of(&ws, "target", &a_file, 0, 3);
    assert_eq!(callers, vec![("holder".to_string(), vec![1, 3])]);
    let _ = fs::remove_dir_all(&dir);
}
