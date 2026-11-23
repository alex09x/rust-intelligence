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

/// Runs `run` on each target of `order` (dependencies before dependents), up to `jobs` at once: a target starts when
/// each of its dependencies in `order` has finished, as Cargo schedules units. Whether all of them succeeded; after a
/// failure none starts. One at a time, a comment edit in ridx's own lib took 28 s for its 7 units (lib, lib tests,
/// test binaries), against 9 s under Cargo (prod-code, issue #7).
pub(crate) fn run_in_parallel(order: &[String], dependencies_of: &HashMap<String, HashSet<String>>, jobs: usize, run: impl Fn(&str) -> bool + Sync) -> bool {
    let in_order: HashSet<&str> = order.iter().map(String::as_str).collect();
    let (mut started, mut done) = (HashSet::new(), HashSet::new());
    let (mut running, mut failed) = (0usize, false);
    let (tx, rx) = std::sync::mpsc::channel::<(&str, bool)>();
    let run = &run;
    std::thread::scope(|s| loop {
        for t in order.iter().map(String::as_str) {
            if failed || running >= jobs.max(1) {
                break;
            }
            let ready = dependencies_of.get(t).is_none_or(|ds| ds.iter().all(|d| !in_order.contains(d.as_str()) || done.contains(d.as_str())));
            if ready && started.insert(t) {
                running += 1;
                let tx = tx.clone();
                s.spawn(move || {
                    let _ = tx.send((t, run(t)));
                });
            }
        }
        if running == 0 {
            break;
        }
        let Ok((t, ok)) = rx.recv() else { break };
        running -= 1;
        if ok {
            done.insert(t);
        } else {
            failed = true;
        }
    });
    !failed && done.len() == in_order.len()
}

/// How many units compile at once: the machine's cores, as Cargo runs them, at most 8 (a driver takes 1.5–3 GB on a
/// large crate).
pub(crate) fn direct_jobs() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get()).min(8)
}

#[cfg(test)]
mod schedule_tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn test_units_run_at_once_after_their_dependencies() {
        let order: Vec<String> = ["lib", "lib_test", "e2e", "bin"].map(String::from).to_vec();
        let deps: HashMap<String, HashSet<String>> = ["lib_test", "e2e", "bin"].iter().map(|t| (t.to_string(), HashSet::from(["lib".to_string()]))).collect();
        let (log, live, peak) = (Mutex::new(Vec::new()), Mutex::new(0usize), Mutex::new(0usize));
        let ok = run_in_parallel(&order, &deps, 4, |t| {
            let n = {
                let mut l = live.lock().unwrap();
                *l += 1;
                *l
            };
            let mut p = peak.lock().unwrap();
            *p = (*p).max(n);
            drop(p);
            std::thread::sleep(std::time::Duration::from_millis(50));
            *live.lock().unwrap() -= 1;
            log.lock().unwrap().push(t.to_string());
            true
        });
        assert!(ok);
        let log = log.into_inner().unwrap();
        assert_eq!(log[0], "lib", "the dependency first: {log:?}");
        assert_eq!(log.len(), 4);
        assert_eq!(peak.into_inner().unwrap(), 3, "the three dependents at once");
    }

    #[test]
    fn test_a_failure_starts_nothing_after_it() {
        let order: Vec<String> = ["lib", "dependent"].map(String::from).to_vec();
        let deps = HashMap::from([("dependent".to_string(), HashSet::from(["lib".to_string()]))]);
        let ran = Mutex::new(Vec::new());
        assert!(!run_in_parallel(&order, &deps, 4, |t| {
            ran.lock().unwrap().push(t.to_string());
            t != "lib"
        }));
        assert_eq!(ran.into_inner().unwrap(), ["lib"]);
        assert!(run_in_parallel(&[], &deps, 4, |_| false), "nothing to run");
    }
}
