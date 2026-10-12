/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compiler-grounded L2 integration tests for Domain 6: Hierarchies & Graph.
//! Tests prepareCallHierarchy, incomingCalls, outgoingCalls, prepareTypeHierarchy,
//! supertypes, and subtypes on real compiler-indexed fixture workspaces.

use super::l2_harness::{fixture_dir, get_or_index_l2_fixture, pos_in_file};
use ridx::lsp::hierarchy::call::{
    handle_call_hierarchy_incoming_calls, handle_call_hierarchy_outgoing_calls,
    handle_prepare_call_hierarchy,
};
use ridx::lsp::hierarchy::types::{
    handle_prepare_type_hierarchy, handle_type_hierarchy_subtypes,
    handle_type_hierarchy_supertypes,
};
use ridx::lsp::protocol::{path_to_uri, PositionConverter};
use ridx::overlay::OverlayManager;
use serde_json::json;
use std::time::Instant;

#[test]
fn test_l2_call_hierarchy_prepare_and_incoming() {
    let ws = get_or_index_l2_fixture("traits_and_generics");
    let lib_file = fixture_dir("traits_and_generics").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // 1. Prepare call hierarchy on `fn greet` in `impl Greeter for Robot`
    let (greet_line, greet_col) = pos_in_file(
        &lib_file,
        "impl Greeter for Robot {\n    fn greet(&self)",
        "greet",
    );
    let prep_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": greet_line, "character": greet_col }
        }
    });

    let prep_res = handle_prepare_call_hierarchy(&prep_req, Some(ws), &overlay_mgr, &mut conv).unwrap();
    assert!(
        prep_res.is_array(),
        "prepareCallHierarchy must return an array"
    );
    let items = prep_res.as_array().unwrap();
    assert_eq!(items.len(), 1, "Expected 1 CallHierarchyItem for greet");
    let greet_item = &items[0];
    assert_eq!(greet_item["name"], "greet");

    // 2. Incoming calls to `greet`: called by `call_concrete`
    let inc_req = json!({
        "params": {
            "item": greet_item
        }
    });
    let inc_res = handle_call_hierarchy_incoming_calls(&inc_req, Some(ws), &overlay_mgr, &mut conv);
    assert!(
        inc_res.is_array(),
        "incomingCalls must return an array"
    );
    let inc_calls = inc_res.as_array().unwrap();
    assert!(
        !inc_calls.is_empty(),
        "Expected at least 1 incoming call for greet"
    );
    let caller_names: Vec<&str> = inc_calls
        .iter()
        .filter_map(|c| c["from"]["name"].as_str())
        .collect();
    assert!(
        caller_names.contains(&"call_concrete"),
        "Incoming calls for greet must contain call_concrete, got: {caller_names:?}"
    );
}

#[test]
fn test_l2_call_hierarchy_outgoing() {
    let ws = get_or_index_l2_fixture("traits_and_generics");
    let lib_file = fixture_dir("traits_and_generics").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // 1. Prepare call hierarchy on `process_container`
    let (proc_line, proc_col) = pos_in_file(
        &lib_file,
        "pub fn process_container<C: Container<Item = i32>>(c: &C) -> i32",
        "process_container",
    );
    let prep_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": proc_line, "character": proc_col }
        }
    });

    let prep_res = handle_prepare_call_hierarchy(&prep_req, Some(ws), &overlay_mgr, &mut conv).unwrap();
    let items = prep_res.as_array().unwrap();
    assert_eq!(items.len(), 1);
    let proc_item = &items[0];
    assert_eq!(proc_item["name"], "process_container");

    // 2. Outgoing calls from `process_container`: calls `c.extract()`
    let out_req = json!({
        "params": {
            "item": proc_item
        }
    });
    let out_res = handle_call_hierarchy_outgoing_calls(&out_req, Some(ws), &overlay_mgr, &mut conv);
    assert!(out_res.is_array());
    let out_calls = out_res.as_array().unwrap();
    assert!(
        !out_calls.is_empty(),
        "process_container must have outgoing calls (extract)"
    );
    let target_names: Vec<&str> = out_calls
        .iter()
        .filter_map(|c| c["to"]["name"].as_str())
        .collect();
    assert!(
        target_names.contains(&"extract"),
        "Outgoing calls from process_container must contain extract, got: {target_names:?}"
    );
}

#[test]
fn test_l2_type_hierarchy_supertypes_and_subtypes() {
    let ws = get_or_index_l2_fixture("traits_and_generics");
    let lib_file = fixture_dir("traits_and_generics").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // 1. Prepare type hierarchy on `BoxContainer` struct
    let (box_line, box_col) =
        pos_in_file(&lib_file, "pub struct BoxContainer(pub i32);", "BoxContainer");
    let prep_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": box_line, "character": box_col }
        }
    });
    let prep_res = handle_prepare_type_hierarchy(&prep_req, Some(ws), &overlay_mgr, &mut conv).unwrap();
    assert!(prep_res.is_array());
    let items = prep_res.as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "BoxContainer");

    // 2. Supertypes of `BoxContainer`: must include `Container` trait
    let super_req = json!({
        "params": {
            "item": { "name": "BoxContainer" }
        }
    });
    let super_res = handle_type_hierarchy_supertypes(&super_req, Some(ws));
    assert!(super_res.is_array());
    let supers = super_res.as_array().unwrap();
    assert!(
        !supers.is_empty(),
        "BoxContainer must have supertypes (Container)"
    );
    let super_names: Vec<&str> = supers
        .iter()
        .filter_map(|s| s["name"].as_str())
        .collect();
    assert!(
        super_names.iter().any(|n| n.contains("Container")),
        "Supertypes of BoxContainer must contain Container trait, got: {super_names:?}"
    );

    // 3. Subtypes of `Container` trait: must include `BoxContainer`
    let sub_req = json!({
        "params": {
            "item": { "name": "Container" }
        }
    });
    let sub_res = handle_type_hierarchy_subtypes(&sub_req, Some(ws));
    assert!(sub_res.is_array());
    let subs = sub_res.as_array().unwrap();
    assert!(
        !subs.is_empty(),
        "Container must have subtypes (BoxContainer)"
    );
    let sub_names: Vec<&str> = subs
        .iter()
        .filter_map(|s| s["name"].as_str())
        .collect();
    assert!(
        sub_names.iter().any(|n| n.contains("BoxContainer")),
        "Subtypes of Container must contain BoxContainer struct, got: {sub_names:?}"
    );
}

#[test]
fn test_l2_type_hierarchy_robot_greeter() {
    let ws = get_or_index_l2_fixture("traits_and_generics");

    // 1. Supertypes of `Robot`: must include `Greeter`
    let super_req = json!({
        "params": {
            "item": { "name": "Robot" }
        }
    });
    let super_res = handle_type_hierarchy_supertypes(&super_req, Some(ws));
    let supers = super_res.as_array().unwrap();
    let super_names: Vec<&str> = supers
        .iter()
        .filter_map(|s| s["name"].as_str())
        .collect();
    assert!(
        super_names.iter().any(|n| n.contains("Greeter")),
        "Supertypes of Robot must contain Greeter, got: {super_names:?}"
    );

    // 2. Subtypes of `Greeter`: must include `Robot`
    let sub_req = json!({
        "params": {
            "item": { "name": "Greeter" }
        }
    });
    let sub_res = handle_type_hierarchy_subtypes(&sub_req, Some(ws));
    let subs = sub_res.as_array().unwrap();
    let sub_names: Vec<&str> = subs
        .iter()
        .filter_map(|s| s["name"].as_str())
        .collect();
    assert!(
        sub_names.iter().any(|n| n.contains("Robot")),
        "Subtypes of Greeter must contain Robot, got: {sub_names:?}"
    );
}

#[test]
#[ignore = "latency benchmark"]
fn test_l2_hierarchy_latency_benchmark() {
    let ws = get_or_index_l2_fixture("traits_and_generics");
    let lib_file = fixture_dir("traits_and_generics").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // 1. Benchmark prepareCallHierarchy on `greet`
    let (greet_line, greet_col) = pos_in_file(
        &lib_file,
        "impl Greeter for Robot {\n    fn greet(&self)",
        "greet",
    );
    let prep_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": greet_line, "character": greet_col }
        }
    });

    let mut prep_latencies = Vec::with_capacity(100);
    for _ in 0..100 {
        let t0 = Instant::now();
        let _ = handle_prepare_call_hierarchy(&prep_req, Some(ws), &overlay_mgr, &mut conv).unwrap();
        prep_latencies.push(t0.elapsed());
    }
    prep_latencies.sort();
    let prep_p50 = prep_latencies[50];
    let prep_p95 = prep_latencies[95];

    // 2. Benchmark typeHierarchy/supertypes on `Robot`
    let super_req = json!({
        "params": {
            "item": { "name": "Robot" }
        }
    });
    let mut super_latencies = Vec::with_capacity(100);
    for _ in 0..100 {
        let t0 = Instant::now();
        let _ = handle_type_hierarchy_supertypes(&super_req, Some(ws));
        super_latencies.push(t0.elapsed());
    }
    super_latencies.sort();
    let super_p50 = super_latencies[50];
    let super_p95 = super_latencies[95];

    eprintln!(
        "BENCHMARK_PREPARE_CALL_HIERARCHY: p50={:?}, p95={:?}",
        prep_p50, prep_p95
    );
    eprintln!(
        "BENCHMARK_TYPE_SUPERTYPES: p50={:?}, p95={:?}",
        super_p50, super_p95
    );
}

#[test]
fn test_l2_call_hierarchy_outgoing_with_array_and_braces() {
    let ws = get_or_index_l2_fixture("traits_and_generics");
    let lib_file = fixture_dir("traits_and_generics").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let (proc_line, proc_col) = pos_in_file(
        &lib_file,
        "pub fn process_bytes(b: [u8; 4]) -> i32",
        "process_bytes",
    );
    let prep_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": proc_line, "character": proc_col }
        }
    });

    let prep_res = handle_prepare_call_hierarchy(&prep_req, Some(ws), &overlay_mgr, &mut conv).unwrap();
    let items = prep_res.as_array().unwrap();
    assert_eq!(items.len(), 1);
    let proc_item = &items[0];
    assert_eq!(proc_item["name"], "process_bytes");

    let out_req = json!({ "params": { "item": proc_item } });
    let out_res = handle_call_hierarchy_outgoing_calls(&out_req, Some(ws), &overlay_mgr, &mut conv);
    assert!(out_res.is_array());
    let out_calls = out_res.as_array().unwrap();
    assert!(!out_calls.is_empty(), "process_bytes must have outgoing calls");
    let target_names: Vec<&str> = out_calls
        .iter()
        .filter_map(|c| c["to"]["name"].as_str())
        .collect();
    assert!(
        target_names.contains(&"extract"),
        "Outgoing calls from process_bytes must contain extract, got: {target_names:?}"
    );
}

#[test]
fn test_l2_type_hierarchy_cross_crate_impl() {
    let ws = get_or_index_l2_fixture("multi_crate");

    let super_req = json!({ "params": { "item": { "name": "CoreService" } } });
    let super_res = handle_type_hierarchy_supertypes(&super_req, Some(ws));
    let supers = super_res.as_array().unwrap();
    let super_names: Vec<&str> = supers.iter().filter_map(|s| s["name"].as_str()).collect();
    assert!(
        super_names.iter().any(|n| n.contains("Worker")),
        "Supertypes of CoreService must contain cross-crate trait Worker, got: {super_names:?}"
    );

    let sub_req = json!({ "params": { "item": { "name": "Worker" } } });
    let sub_res = handle_type_hierarchy_subtypes(&sub_req, Some(ws));
    let subs = sub_res.as_array().unwrap();
    let sub_names: Vec<&str> = subs.iter().filter_map(|s| s["name"].as_str()).collect();
    assert!(
        sub_names.iter().any(|n| n.contains("CoreService")),
        "Subtypes of Worker must contain CoreService, got: {sub_names:?}"
    );
}

#[test]
#[ignore = "text-based hierarchy, see backlog"]
fn test_l2_type_hierarchy_derive_impl() {
    let ws = get_or_index_l2_fixture("macros_and_derives");

    let sub_req = json!({ "params": { "item": { "name": "Clone" } } });
    let sub_res = handle_type_hierarchy_subtypes(&sub_req, Some(ws));
    let subs = sub_res.as_array().unwrap();
    let sub_names: Vec<&str> = subs.iter().filter_map(|s| s["name"].as_str()).collect();
    assert!(
        sub_names.iter().any(|n| n.contains("DerivedModel")),
        "Subtypes of Clone must contain DerivedModel via #[derive(Clone)], got: {sub_names:?}"
    );
}

