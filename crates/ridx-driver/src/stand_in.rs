/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! A registry or git dependency that does not compile. Cargo builds nothing that depends on it, so one broken
//! dev-dependency left every test target of a package unindexed (serde's `test_suite` with a `trybuild` that
//! fails against the registry `serde_core`), although most of those files never use it. The driver then
//! compiles an empty crate with the same name, flags and outputs in its place: cargo goes on, the dependents are
//! indexed, and only their uses of that crate stay unresolved (indexed like any crate with errors).
//!
//! The stand-in's source is left next to its outputs as `ridx-stand-in-<crate><extra>.rs`, and its dep-info
//! names it for as long as it stands in: `ridx index` lists the stand-ins from there.
//!
//! A crate standing in writes no sidecar, so `feature_seed` counts its package (and those it invalidates
//! with it) dirty on every run and cargo checks them again, as it did the failing crate before.

use std::fs;
use std::path::{Path, PathBuf};

use ridx_format::STAND_IN_PREFIX as PREFIX;

/// Compiles a stand-in for the dependency `args` failed to build; `None` when it is not a dependency library
/// or the stand-in did not compile either (the original failure stands).
pub(crate) fn compile(args: &[String], manifest_dir: Option<&Path>) -> Option<Result<(), ()>> {
    let plan = plan(args, manifest_dir, std::env::var_os("CARGO_PRIMARY_PACKAGE").is_some())?;
    fs::create_dir_all(plan.source.parent()?).ok()?;
    let note = format!("// ridx: stand-in for `{}`, which did not compile; its dependents are indexed without it.\n", plan.crate_name);
    fs::write(&plan.source, note).ok()?;
    // Written during the build, the source would be newer than the unit's start and cargo would rebuild the
    // stand-in, and everything checked against it, on every later run: date it like the crate it replaces.
    if let Ok(modified) = fs::metadata(&plan.original).and_then(|m| m.modified()) {
        let _ = fs::File::options().write(true).open(&plan.source).and_then(|f| f.set_modified(modified));
    }
    if super::run_with_callbacks(&plan.args, &mut super::NoopCallbacks).is_err() {
        let _ = fs::remove_file(&plan.source);
        return None;
    }
    eprintln!("warning: ridx: `{}` did not compile; its dependents are indexed against an empty stand-in", plan.crate_name);
    Some(Ok(()))
}

/// A stand-in compilation: the original arguments with the crate's source replaced.
#[derive(Debug, PartialEq)]
pub(crate) struct Plan {
    pub(crate) crate_name: String,
    /// The crate root that did not compile.
    pub(crate) original: PathBuf,
    pub(crate) source: PathBuf,
    pub(crate) args: Vec<String>,
}

/// Plans the stand-in for a failed compilation, if it is one of a dependency's library or procedural macro.
/// `primary_package`: cargo selected the package for the build (`CARGO_PRIMARY_PACKAGE`), passed in rather than
/// read here because a test harness run by cargo has that variable set too.
pub(crate) fn plan(args: &[String], manifest_dir: Option<&Path>, primary_package: bool) -> Option<Plan> {
    let dependency = manifest_dir.is_some_and(super::metadata::is_immutable_package_dir) || super::metadata::built_as_non_local_package(args.iter().cloned(), primary_package);
    if !dependency {
        return None;
    }
    let crate_name = value_of(args, "--crate-name")?.to_string();
    let types: Vec<&str> = values_of(args, "--crate-type").flat_map(|v| v.split(',')).collect();
    // A build script or binary has no dependents to unblock; a test harness is not a dependency.
    if types.contains(&"bin") || args.iter().any(|a| a == "--test") {
        return None;
    }
    let input = input_index(args)?;
    let out_dir = match value_of(args, "--out-dir") {
        Some(d) => PathBuf::from(d),
        None => Path::new(value_of(args, "-o")?).parent()?.to_path_buf(),
    };
    let extra = codegen_values(args).find_map(|v| v.strip_prefix("extra-filename=")).unwrap_or("");
    let source = out_dir.join(format!("{PREFIX}{crate_name}{extra}.rs"));
    let mut stand_in_args = args.to_vec();
    stand_in_args[input] = source.to_string_lossy().into_owned();
    Some(Plan { crate_name, original: PathBuf::from(&args[input]), source, args: stand_in_args })
}

/// The value of a flag given as `--flag value` or `--flag=value`.
fn value_of<'a>(args: &'a [String], flag: &'a str) -> Option<&'a str> {
    values_of(args, flag).next()
}

fn values_of<'a>(args: &'a [String], flag: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    let prefix = format!("{flag}=");
    args.iter().enumerate().filter_map(move |(i, a)| if a == flag { args.get(i + 1).map(String::as_str) } else { a.strip_prefix(prefix.as_str()) })
}

/// The values of `-C`, in each of its spellings.
fn codegen_values(args: &[String]) -> impl Iterator<Item = &str> {
    args.iter().enumerate().filter_map(move |(i, a)| match a.as_str() {
        "-C" | "--codegen" => args.get(i + 1).map(String::as_str),
        _ => a.strip_prefix("-C").or_else(|| a.strip_prefix("--codegen=")),
    })
}

/// The position of the crate root: the one `.rs` argument that is not a flag's value.
fn input_index(args: &[String]) -> Option<usize> {
    const TAKES_VALUE: &[&str] = &[
        "-o",
        "--out-dir",
        "-L",
        "--extern",
        "--crate-name",
        "--crate-type",
        "--edition",
        "--cfg",
        "--check-cfg",
        "-C",
        "--codegen",
        "--cap-lints",
        "--target",
        "--sysroot",
        "--error-format",
        "--json",
        "--emit",
        "-A",
        "-W",
        "-D",
        "-F",
    ];
    (1..args.len()).find(|&i| args[i].ends_with(".rs") && !args[i].starts_with('-') && !TAKES_VALUE.contains(&args[i - 1].as_str()))
}
