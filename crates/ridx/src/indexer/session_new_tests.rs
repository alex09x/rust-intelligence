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
/// The second list holds the files whose target is not told for sure: their package's manifest configures its tests
/// (`autotests`, `[[test]]`), which could name, edition, harness or exclude them otherwise (review of fd0e108b).
pub(in crate::indexer) fn new_test_units(deps: &Path, root: &Path, files: &[PathBuf]) -> (Vec<Unit>, Vec<PathBuf>) {
    let units = workspace_units(deps);
    let (mut out, mut unresolved) = (Vec::new(), Vec::new());
    for file in files {
        let Some((pkg, target)) = autotest_of(file) else { continue };
        let pkg = canonical(&pkg);
        // Only a package that discovers its tests as Cargo does by default: then the target is `<name>`, with the
        // package's edition and the test harness, for sure (reviews of c8b32392 and fd0e108b).
        if !plain_autotests(&pkg) {
            unresolved.push(file.clone());
            continue;
        }
        let of_pkg: Vec<&(Vec<String>, HashMap<String, String>)> =
            units.iter().filter(|(_, env)| env.get("CARGO_MANIFEST_DIR").is_some_and(|d| canonical(Path::new(d)) == pkg)).collect();
        let is_test = |a: &[String]| a.iter().any(|x| x == "--test");
        let in_tests = |a: &[String]| root_arg(a, root).is_some_and(|(_, p)| p.starts_with(pkg.join("tests")));
        let lib = of_pkg.iter().find(|(a, _)| !is_test(a) && value_of(a, "--crate-type").is_some_and(|t| t == "lib" || t == "rlib"));
        // An integration test of the package binds its library and dev-dependencies; else any test harness of it (a
        // library's unit tests, a binary's: review of c8b32392, a package without a library) has the dev-dependencies,
        // and the library, when there is one, is bound as an integration test binds it.
        let Some((mut args, mut env)) =
            of_pkg.iter().find(|(a, _)| is_test(a) && in_tests(a)).or_else(|| of_pkg.iter().find(|(a, _)| is_test(a))).map(|(a, e)| (a.clone(), e.clone()))
        else {
            continue;
        };
        if !in_tests(&args) {
            if let Some((l, _)) = lib {
                let (Some(name), Some(dir), Some((_, extra))) = (value_of(l, "--crate-name"), value_of(l, "--out-dir"), codegen(l, "extra-filename=")) else { continue };
                args.extend(["--extern".to_string(), format!("{name}={}", Path::new(&dir).join(format!("lib{name}{extra}.rmeta")).display())]);
            }
        }
        let Some((at, _)) = root_arg(&args, root) else { continue };
        let crate_name = target.replace('-', "_");
        args[at] = file.display().to_string();
        if let Some(i) = args.iter().position(|a| a == "--crate-name") {
            args[i + 1] = crate_name.clone();
        }
        // The new target's own edition, the package's: a template's per-target one is not it (review of c8b32392).
        if let Some(edition) = package_edition(&pkg) {
            set_edition(&mut args, &edition);
        }
        // Its outputs are its own: two packages adding `tests/api.rs` do not share them (review of c8b32392).
        let id = {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            std::hash::Hash::hash(file, &mut h);
            format!("ridxnew{:08x}", std::hash::Hasher::finish(&h) as u32)
        };
        for key in ["extra-filename=", "metadata="] {
            if let Some((i, _)) = codegen(&args, key) {
                args[i] = format!("{key}{}{id}", if key == "extra-filename=" { "-" } else { "" });
            }
        }
        env.insert("CARGO_CRATE_NAME".into(), crate_name.clone());
        let sidecar = deps.join(format!("{crate_name}-{id}.json"));
        let object = Some(PathBuf::from(format!("{crate_name}-{id}.ridx")));
        out.push(Unit { name: crate_name, args, env, object, sidecar, aliases: Vec::new(), own: Some([file.clone()].into()), synthetic: Some(file.clone()) });
    }
    (out, unresolved)
}

/// Whether the manifest of the package at `pkg` leaves its tests to Cargo's default discovery: it says nothing of
/// `autotests` and declares no test target (`[[test]]`, or `test = [...]`), comments aside. Any such setting may
/// change which file is a test and how it is built, which only Cargo's own reading of TOML tells; a manifest that
/// cannot be read says nothing of them. Read conservatively, as TOML spells keys: quoted (`[["test"]]`, review of
/// c6b62a99) or not, and any key with an escape (`"\u0074est"`) is taken for one that may be `test`; a line that
/// only looks like one (in a multi-line string) declines too.
fn plain_autotests(pkg: &Path) -> bool {
    let text = fs::read_to_string(pkg.join("Cargo.toml")).unwrap_or_default();
    !text.lines().any(|line| {
        let code: String = line.split('#').next().unwrap_or("").chars().filter(|c| !c.is_whitespace() && !matches!(c, '"' | '\'')).collect();
        // The key a line sets or the table it opens.
        let key = match code.strip_prefix('[') {
            Some(header) => header.trim_start_matches('[').split(']').next().unwrap_or(""),
            None => code.split('=').next().unwrap_or(""),
        };
        code.contains("autotests") || key.contains('\\') || key == "test" || key.starts_with("test.")
    })
}

/// The edition of the package at `pkg`, as Cargo resolves it (a workspace's inherited one too).
fn package_edition(pkg: &Path) -> Option<String> {
    let out =
        std::process::Command::new("cargo").args(["metadata", "--format-version", "1", "--no-deps", "--offline", "--manifest-path"]).arg(pkg.join("Cargo.toml")).output().ok()?;
    let j: Value = serde_json::from_slice(&out.stdout).ok()?;
    let manifest = canonical(&pkg.join("Cargo.toml"));
    j["packages"].as_array()?.iter().find(|p| p["manifest_path"].as_str().is_some_and(|m| canonical(Path::new(m)) == manifest))?["edition"].as_str().map(str::to_string)
}

/// `args` compiling with `edition`.
fn set_edition(args: &mut Vec<String>, edition: &str) {
    if let Some(i) = args.iter().position(|a| a.starts_with("--edition=")) {
        args[i] = format!("--edition={edition}");
    } else if let Some(i) = args.iter().position(|a| a == "--edition") {
        if let Some(v) = args.get_mut(i + 1) {
            *v = edition.to_string();
        }
    } else {
        args.push(format!("--edition={edition}"));
    }
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

    /// A package `name` under `root` with a sidecar per (crate root, test or not, crate type) in `deps`.
    fn package(root: &Path, deps: &Path, name: &str, manifest: &str, units: &[(&str, bool, Option<&str>)]) -> PathBuf {
        let pkg = root.join(name);
        fs::create_dir_all(pkg.join("src")).unwrap();
        fs::create_dir_all(pkg.join("tests")).unwrap();
        fs::write(pkg.join("Cargo.toml"), manifest).unwrap();
        for (i, (src, test, kind)) in units.iter().enumerate() {
            fs::write(pkg.join(src), "").unwrap();
            let mut args: Vec<String> = ["rustc", "--crate-name", name, "--edition=2015"].map(String::from).into();
            args.push(pkg.join(src).display().to_string());
            if *test {
                args.push("--test".into());
            }
            if let Some(k) = kind {
                args.extend(["--crate-type".to_string(), k.to_string()]);
            }
            args.extend(["-C", "extra-filename=-abc", "--out-dir", "/out"].map(String::from));
            let j = serde_json::json!({ "compiler_args": args, "object_path": format!("{name}-{i}"), "env": { "CARGO_MANIFEST_DIR": pkg.display().to_string() } });
            fs::write(deps.join(format!("{name}-{i}.json")), j.to_string()).unwrap();
        }
        canonical(&pkg)
    }

    const PKG: &str = "[package]\nname = \"p\"\nversion = \"0.1.0\"\nedition = \"2021\"\n";

    /// Review of c8b32392: two packages adding `tests/api.rs` shared outputs; `autotests = false` was ignored; a
    /// package without a library could not add a test; a template's edition was taken for the new target's.
    #[test]
    fn test_a_new_test_target_is_the_one_cargo_would_build() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let deps = root.join("deps");
        fs::create_dir_all(&deps).unwrap();
        let a = package(&root, &deps, "a", PKG, &[("src/lib.rs", false, Some("lib")), ("tests/old.rs", true, None)]);
        let b = package(&root, &deps, "b", PKG, &[("src/main.rs", true, None)]);
        let off = package(&root, &deps, "off", &format!("{PKG}autotests = false\n\n[[test]]\nname = \"kept\"\npath = \"tests/kept.rs\"\n"), &[("tests/old.rs", true, None)]);
        let new = |p: &Path, f: &str| canonical(&p.join(f));
        let files = [new(&a, "tests/api.rs"), new(&b, "tests/api.rs"), new(&off, "tests/new.rs"), new(&off, "tests/kept.rs")];
        let commented = package(&root, &deps, "commented", "[package] # settings\nname = \"p\"\nversion = \"0.1.0\"\nautotests = false\n", &[("tests/old.rs", true, None)]);
        let files = [files[0].clone(), files[1].clone(), files[2].clone(), files[3].clone(), new(&commented, "tests/new.rs")];
        let (units, unresolved) = new_test_units(&deps, &root, &files);
        let made: Vec<&PathBuf> = units.iter().filter_map(|u| u.synthetic.as_ref()).collect();
        assert_eq!(made, [&files[0], &files[1]], "packages that leave their tests to Cargo's discovery");
        // Review of fd0e108b: a manifest that configures its tests (a `[[test]]` with Cargo's default path, a header with
        // a comment) is not told by reading it line by line: said so, not guessed.
        assert_eq!(unresolved, [files[2].clone(), files[3].clone(), files[4].clone()]);
        assert_ne!(units[0].sidecar, units[1].sidecar, "each its own outputs");
        assert_ne!(codegen(&units[0].args, "extra-filename="), codegen(&units[1].args, "extra-filename="));
        assert!(!units[1].args.iter().any(|x| x == "--extern"), "a package without a library binds none: {:?}", units[1].args);
        assert!(
            units[0].args.iter().any(|x| x == "--edition=2021") && !units[0].args.iter().any(|x| x == "--edition=2015"),
            "the package's edition, not the template's: {:?}",
            units[0].args
        );
    }

    /// Review of c6b62a99: `[["test"]]` is Cargo's `[[test]]`, and was read as no test declared.
    #[test]
    fn test_a_test_declared_in_any_spelling_is_seen() {
        let dir = tempfile::tempdir().unwrap();
        let plain = |extra: &str| {
            fs::write(dir.path().join("Cargo.toml"), format!("{PKG}{extra}")).unwrap();
            plain_autotests(dir.path())
        };
        assert!(plain(""));
        assert!(plain("\n[dependencies]\ntestx = \"1\"\n# [[test]]\n"), "a key that only starts so, a comment");
        for declared in [
            "\n[[test]]\nname = \"t\"\n",
            "\n[[\"test\"]]\nname = \"t\"\nharness = false\n",
            "\n[[ 'test' ]]\nname = \"t\"\n",
            "\n\"test\" = [{ name = \"t\" }]\n",
            "\ntest = [{ name = \"t\" }]\n",
            "\n[[\"\\u0074est\"]]\nname = \"t\"\n",
            "autotests = false\n",
            "\"autotests\" = false\n",
        ] {
            assert!(!plain(declared), "{declared}");
        }
    }
}
