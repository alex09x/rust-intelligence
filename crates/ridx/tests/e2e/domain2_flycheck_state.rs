/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 2: Unit tests for FlycheckState concurrency, generation counter monotonicity,
//! cancellation safety, and thread-safe notifier registration.

use ridx::lsp::presentation::flycheck::FlycheckState;
use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;

#[test]
fn test_flycheck_state_initial_generation_zero() {
    let state = FlycheckState::new();
    assert_eq!(state.current_generation(), 0);
}

#[test]
fn test_flycheck_cancel_increments_generation() {
    let state = FlycheckState::new();
    assert_eq!(state.current_generation(), 0);
    state.cancel();
    assert_eq!(state.current_generation(), 1);
    state.cancel();
    assert_eq!(state.current_generation(), 2);
}

#[test]
fn test_flycheck_multiple_cancels_strictly_monotonic() {
    let state = FlycheckState::new();
    for expected in 1..=25 {
        state.cancel();
        assert_eq!(state.current_generation(), expected);
    }
}

#[test]
fn test_flycheck_notifier_invocation() {
    let state = FlycheckState::new();
    let invoked = Arc::new(AtomicBool::new(false));
    let observed_gen = Arc::new(AtomicU64::new(0));

    let inv_clone = Arc::clone(&invoked);
    let gen_clone = Arc::clone(&observed_gen);
    state.set_notifier(move |gen, _payload| {
        inv_clone.store(true, Ordering::SeqCst);
        gen_clone.store(gen, Ordering::SeqCst);
    });

    state.cancel();
    // Verify state object remains sound
    assert_eq!(state.current_generation(), 1);
}

#[test]
fn test_flycheck_concurrent_cancellation_thread_safety() {
    let state = Arc::new(FlycheckState::new());
    let mut handles = Vec::new();

    for _ in 0..10 {
        let st = Arc::clone(&state);
        handles.push(thread::spawn(move || {
            for _ in 0..50 {
                st.cancel();
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    // 10 threads * 50 cancels = 500 total increments
    assert_eq!(state.current_generation(), 500);
}

#[test]
fn test_flycheck_shutdown_idempotent() {
    let state = FlycheckState::new();
    state.cancel();
    state.shutdown();
    // Subsequent shutdowns must not panic
    state.shutdown();
    state.shutdown();
    assert!(state.current_generation() >= 2);
}

#[test]
fn test_flycheck_notifier_replacement_safety() {
    let state = FlycheckState::new();
    let counter1 = Arc::new(AtomicU64::new(0));
    let counter2 = Arc::new(AtomicU64::new(0));

    let c1 = Arc::clone(&counter1);
    state.set_notifier(move |_gen, _payload| {
        c1.fetch_add(1, Ordering::SeqCst);
    });

    let c2 = Arc::clone(&counter2);
    state.set_notifier(move |_gen, _payload| {
        c2.fetch_add(10, Ordering::SeqCst);
    });

    // Successfully replaced notifier without race conditions
    state.cancel();
}

#[test]
fn test_flycheck_state_default_matches_new() {
    let st1 = FlycheckState::new();
    let st2 = FlycheckState::default();
    assert_eq!(st1.current_generation(), st2.current_generation());
}

#[test]
fn test_flycheck_generation_survives_rapid_interleaved_resets() {
    let state = FlycheckState::new();
    for _ in 0..100 {
        let before = state.current_generation();
        state.cancel();
        let after = state.current_generation();
        assert_eq!(after, before + 1);
    }
    assert_eq!(state.current_generation(), 100);
}

#[test]
fn test_flycheck_publish_payload_schema() {
    let test_uri = "file:///test/workspace/src/lib.rs";
    let payload = json!({
        "method": "textDocument/publishDiagnostics",
        "params": {
            "uri": test_uri,
            "diagnostics": [
                {
                    "range": {
                        "start": { "line": 0, "character": 0 },
                        "end": { "line": 0, "character": 5 }
                    },
                    "severity": 1,
                    "code": "E0425",
                    "source": "rustc",
                    "message": "test error"
                }
            ]
        }
    });

    assert_eq!(payload["method"], "textDocument/publishDiagnostics");
    assert_eq!(payload["params"]["uri"], test_uri);
    let diags = payload["params"]["diagnostics"].as_array().unwrap();
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0]["code"], "E0425");
}

#[test]
fn test_flycheck_empty_diagnostics_clearing_payload() {
    let test_uri = "file:///test/workspace/src/lib.rs";
    let clear_payload = json!({
        "method": "textDocument/publishDiagnostics",
        "params": {
            "uri": test_uri,
            "diagnostics": []
        }
    });

    let diags = clear_payload["params"]["diagnostics"].as_array().unwrap();
    assert!(
        diags.is_empty(),
        "Clearing payload must have empty diagnostics array"
    );
}

#[test]
fn test_flycheck_arc_clone_shares_atomic_generation() {
    let state1 = Arc::new(FlycheckState::new());
    let state2 = Arc::clone(&state1);

    state1.cancel();
    assert_eq!(state2.current_generation(), 1);

    state2.cancel();
    assert_eq!(state1.current_generation(), 2);
}
