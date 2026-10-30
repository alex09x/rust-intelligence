/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Dependency graph construction, reverse dependency discovery, and topological sorting for direct crate execution.

use super::WorkspaceCrateEntry;
use std::collections::{HashMap, HashSet, VecDeque};

/// Builds direct dependency and reverse-dependency (rdep) graphs among workspace targets.
pub fn build_workspace_dependency_graphs(workspace_crates: &[WorkspaceCrateEntry]) -> (HashMap<String, HashSet<String>>, HashMap<String, HashSet<String>>) {
    // Map exported crate names to their providing library target_ids
    let mut providers_by_name: HashMap<String, HashSet<String>> = HashMap::new();
    for (target_id, crate_name, args, ..) in workspace_crates {
        let is_lib = !args.contains(&"--test".to_string());
        if is_lib {
            providers_by_name.entry(crate_name.clone()).or_default().insert(target_id.clone());
            let norm = crate_name.replace('-', "_");
            providers_by_name.entry(norm).or_default().insert(target_id.clone());
        }
    }

    let mut dependencies_of: HashMap<String, HashSet<String>> = HashMap::new();
    let mut rdeps_of: HashMap<String, HashSet<String>> = HashMap::new();

    for (target_id, _cname, args, ..) in workspace_crates {
        let mut deps = HashSet::new();
        for i in 0..args.len() {
            let ext_val = if args[i] == "--extern" && i + 1 < args.len() { Some(args[i + 1].as_str()) } else { args[i].strip_prefix("--extern=") };
            if let Some(val) = ext_val {
                let dep_name = val.split(['=', ':']).next().unwrap_or(val).trim();
                let clean_name = dep_name.rsplit(':').next().unwrap_or(dep_name);
                let clean_norm = clean_name.replace('-', "_");

                if let Some(provider_ids) = providers_by_name.get(&clean_norm) {
                    for pid in provider_ids {
                        if pid != target_id {
                            deps.insert(pid.clone());
                            rdeps_of.entry(pid.clone()).or_default().insert(target_id.clone());
                        }
                    }
                }
            }
        }
        dependencies_of.insert(target_id.clone(), deps);
    }

    (dependencies_of, rdeps_of)
}

/// Computes a topological ordering of `crates` such that dependencies run before dependents.
pub fn topo_sort_crates(crates: &HashSet<String>, dependencies_of: &HashMap<String, HashSet<String>>) -> Option<Vec<String>> {
    let mut in_degree: HashMap<String, usize> = HashMap::new();
    for c in crates {
        let deps = dependencies_of.get(c);
        let count = deps.map(|ds| ds.iter().filter(|d| crates.contains(*d)).count()).unwrap_or(0);
        in_degree.insert(c.clone(), count);
    }

    let mut queue: VecDeque<String> = in_degree.iter().filter(|(_, &deg)| deg == 0).map(|(c, _)| c.clone()).collect();

    let mut result = Vec::new();
    while let Some(c) = queue.pop_front() {
        result.push(c.clone());
        for other in crates {
            if let Some(deps) = dependencies_of.get(other) {
                if deps.contains(&c) {
                    if let Some(deg) = in_degree.get_mut(other) {
                        *deg = deg.saturating_sub(1);
                        if *deg == 0 {
                            queue.push_back(other.clone());
                        }
                    }
                }
            }
        }
    }

    if result.len() == crates.len() {
        Some(result)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_topo_sort_order() {
        let mut crates = HashSet::new();
        crates.insert("a".to_string());
        crates.insert("b".to_string());
        crates.insert("c".to_string());

        let mut deps = HashMap::new();
        let mut b_deps = HashSet::new();
        b_deps.insert("a".to_string());
        deps.insert("b".to_string(), b_deps);

        let mut c_deps = HashSet::new();
        c_deps.insert("b".to_string());
        deps.insert("c".to_string(), c_deps);

        let sorted = topo_sort_crates(&crates, &deps).unwrap();
        let pos_a = sorted.iter().position(|x| x == "a").unwrap();
        let pos_b = sorted.iter().position(|x| x == "b").unwrap();
        let pos_c = sorted.iter().position(|x| x == "c").unwrap();

        assert!(pos_a < pos_b);
        assert!(pos_b < pos_c);
    }
}
