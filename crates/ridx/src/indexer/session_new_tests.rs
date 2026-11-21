/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! A test file the edit adds that no unit of the index compiles yet: `tests/<name>.rs` or `tests/<name>/main.rs` of a
//! package, which Cargo builds as an integration test (autotests). It is compiled as that target would be, from the
//! arguments of a test unit of the same package, for its diagnostics (prod-code, m=20181: a proposed test file waited
//! for diagnostics that never came).

use super::{canonical, Unit};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// The package directory and the target name Cargo's autotests give `file`, when it is one of theirs.
fn autotest_of(file: &Path) -> Option<(PathBuf, String)> {
    let parent = file.parent()?;
    if parent.file_name()? == "tests" && file.extension()? == "rs" {
        return Some((parent.parent()?.to_path_buf(), file.file_stem()?.to_str()?.to_string()));
    }
    if file.file_name()? == "main.rs" && parent.parent()?.file_name()? == "tests" {
        return Some((parent.parent()?.parent()?.to_path_buf(), parent.file_name()?.to_str()?.to_string()));
    }
    None
}

/// The position of the crate root among `args` (the first naming an existing `.rs` file, relative to the workspace root
/// where Cargo runs rustc) and that file.
fn root_arg(args: &[String], root: &Path) -> Option<(usize, PathBuf)> {
    args.iter().enumerate().skip(1).find_map(|(i, a)| {
        let p = if Path::new(a).is_absolute() { PathBuf::from(a) } else { root.join(a) };
        (a.ends_with(".rs") && p.is_file()).then(|| (i, canonical(&p)))
    })
}

/// The value of `-C key=` in `args`, and where it is.
fn codegen(args: &[String], key: &str) -> Option<(usize, String)> {
    args.windows(2).enumerate().find_map(|(i, w)| (w[0] == "-C").then(|| w[1].strip_prefix(key)).flatten().map(|v| (i + 1, v.to_string())))
}

fn value_of(args: &[String], flag: &str) -> Option<String> {
    args.windows(2).find(|w| w[0] == flag).map(|w| w[1].clone())
}

/// The workspace units of the sidecars in `deps`, as (compiler arguments, environment).
fn workspace_units(deps: &Path) -> Vec<(Vec<String>, HashMap<String, String>)> {
    let mut entries: Vec<PathBuf> = fs::read_dir(deps).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    entries.sort();
    let mut seen = HashSet::new();
    entries
        .into_iter()
        .filter_map(|p| fs::read(&p).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()))
        .filter(|j| j["is_dependency"].as_bool() != Some(true) && seen.insert(j["object_path"].to_string()))
        .map(|j| {
            let args = j["compiler_args"].as_array().into_iter().flatten().filter_map(|a| a.as_str().map(str::to_string)).collect();
            let env = j["env"].as_object().into_iter().flatten().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect();
            (args, env)
        })
        .collect()
}

/// The units compiling `files` (canonical buffers no unit owns) that are integration tests of a package of the index:
/// from the arguments of an integration test of the same package (they bind its library and its dev-dependencies),
/// else of its library's unit tests with the library bound, with the crate root, name and output names of the new
/// target.
pub(in crate::indexer) fn new_test_units(deps: &Path, root: &Path, files: &[PathBuf]) -> Vec<Unit> {
    let units = workspace_units(deps);
    let mut out = Vec::new();
    for file in files {
        let Some((pkg, target)) = autotest_of(file) else { continue };
        let pkg = canonical(&pkg);
        let of_pkg: Vec<&(Vec<String>, HashMap<String, String>)> =
            units.iter().filter(|(_, env)| env.get("CARGO_MANIFEST_DIR").is_some_and(|d| canonical(Path::new(d)) == pkg)).collect();
        let is_test = |a: &[String]| a.iter().any(|x| x == "--test");
        let in_tests = |a: &[String]| root_arg(a, root).is_some_and(|(_, p)| p.starts_with(pkg.join("tests")));
        let lib = of_pkg.iter().find(|(a, _)| !is_test(a) && value_of(a, "--crate-type").is_some_and(|t| t == "lib" || t == "rlib"));
        let (mut args, mut env) = if let Some((a, e)) = of_pkg.iter().find(|(a, _)| is_test(a) && in_tests(a)) {
            (a.clone(), e.clone())
        } else if let (Some((a, e)), Some((l, _))) = (of_pkg.iter().find(|(a, _)| is_test(a)), lib) {
            // The library's unit tests have its dev-dependencies; the library itself is bound as an integration test binds it.
            let mut a = a.clone();
            let (Some(name), Some(dir), Some((_, extra))) = (value_of(l, "--crate-name"), value_of(l, "--out-dir"), codegen(l, "extra-filename=")) else { continue };
            a.extend(["--extern".to_string(), format!("{name}={}", Path::new(&dir).join(format!("lib{name}{extra}.rmeta")).display())]);
            (a, e.clone())
        } else {
            continue;
        };
        let Some((at, _)) = root_arg(&args, root) else { continue };
        let crate_name = target.replace('-', "_");
        args[at] = file.display().to_string();
        if let Some(i) = args.iter().position(|a| a == "--crate-name") {
            args[i + 1] = crate_name.clone();
        }
        for key in ["extra-filename=", "metadata="] {
            if let Some((i, _)) = codegen(&args, key) {
                args[i] = format!("{key}{}ridxnew", if key == "extra-filename=" { "-" } else { "" });
            }
        }
        env.insert("CARGO_CRATE_NAME".into(), crate_name.clone());
        let sidecar = deps.join(format!("{crate_name}-ridxnew.json"));
        out.push(Unit { name: crate_name, args, env, object: None, sidecar, aliases: Vec::new(), own: Some([file.clone()].into()), synthetic: Some(file.clone()) });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cargo_autotests_name_their_target() {
        assert_eq!(autotest_of(Path::new("/w/p/tests/a_b.rs")), Some((PathBuf::from("/w/p"), "a_b".into())));
        assert_eq!(autotest_of(Path::new("/w/p/tests/suite/main.rs")), Some((PathBuf::from("/w/p"), "suite".into())));
        assert_eq!(autotest_of(Path::new("/w/p/tests/suite/helper.rs")), None, "a module of a test, not a test");
        assert_eq!(autotest_of(Path::new("/w/p/src/x.rs")), None);
    }
}
