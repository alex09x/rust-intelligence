# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Deterministic multi-crate Rust workspace generator with 100% known ground-truth spans."""

import json
import random
from pathlib import Path
from typing import Dict, List

from .source_builder import SourceBuilder, SpanRecord


def _write_cargo_manifests(out_dir: Path) -> None:
    (out_dir / "Cargo.toml").write_text(
        '[workspace]\nmembers = ["crates/core_pkg", "crates/service_pkg", "crates/app_pkg"]\nresolver = "2"\n',
        encoding="utf-8",
    )
    (out_dir / "crates/core_pkg").mkdir(parents=True, exist_ok=True)
    (out_dir / "crates/core_pkg/Cargo.toml").write_text(
        '[package]\nname = "core_pkg"\nversion = "0.1.0"\nedition = "2021"\n',
        encoding="utf-8",
    )
    (out_dir / "crates/service_pkg").mkdir(parents=True, exist_ok=True)
    (out_dir / "crates/service_pkg/Cargo.toml").write_text(
        '[package]\nname = "service_pkg"\nversion = "0.1.0"\nedition = "2021"\n\n'
        '[dependencies]\ncore_pkg = { path = "../core_pkg" }\n',
        encoding="utf-8",
    )
    (out_dir / "crates/app_pkg").mkdir(parents=True, exist_ok=True)
    (out_dir / "crates/app_pkg/Cargo.toml").write_text(
        '[package]\nname = "app_pkg"\nversion = "0.1.0"\nedition = "2021"\n\n'
        '[dependencies]\ncore_pkg = { path = "../core_pkg" }\nservice_pkg = { path = "../service_pkg" }\n',
        encoding="utf-8",
    )


def _emit_core_family(sb: SourceBuilder, tag: str, factor_const: int) -> dict:
    trait_base = f"BaseOp_{tag}"
    trait_sub = f"AdvOp_{tag}"
    fn_compute = f"compute_{tag}"
    fn_boost = f"boost_{tag}"
    struct_rec = f"Record_{tag}"
    struct_alt = f"AltRecord_{tag}"
    field_val = f"value_{tag}"
    field_w = f"weight_{tag}"
    enum_state = f"State_{tag}"
    fn_new = f"new_{tag}"
    fn_scale = f"scale_{tag}"
    fn_calc = f"calc_metric_{tag}"
    doc_url = f"https://docs.rs/synth_core/{tag}"

    sb.line_tokens("/// Documentation link: ", (f"doc_{tag}", "doc_link", doc_url))
    t_base_start, _ = sb.line_tokens("pub trait ", (trait_base, "def", trait_base), " {")
    sb.line_tokens("    fn ", (fn_compute, "decl", fn_compute), "(&self, factor: i64) -> i64;")
    t_base_end = sb.raw_line("}")
    sb.record_fold(t_base_start, t_base_end)
    sb.blank()

    t_sub_start, _ = sb.line_tokens(
        "pub trait ", (trait_sub, "def", trait_sub), ": ", (trait_base, "supertrait_ref", trait_base), " {"
    )
    sb.line_tokens("    fn ", (fn_boost, "decl", fn_boost), "(&self, base: i64) -> i64;")
    t_sub_end = sb.raw_line("}")
    sb.record_fold(t_sub_start, t_sub_end)
    sb.blank()

    sb.raw_line("#[derive(Debug, Clone, Copy, PartialEq, Eq)]")
    s_rec_start, _ = sb.line_tokens("pub struct ", (struct_rec, "def", struct_rec), " {")
    sb.line_tokens("    pub ", (field_val, "def", field_val), ": i64,")
    s_rec_end = sb.raw_line("}")
    sb.record_fold(s_rec_start, s_rec_end)
    sb.blank()

    sb.raw_line("#[derive(Debug, Clone, Copy, PartialEq, Eq)]")
    s_alt_start, _ = sb.line_tokens("pub struct ", (struct_alt, "def", struct_alt), " {")
    sb.line_tokens("    pub ", (field_w, "def", field_w), ": i64,")
    s_alt_end = sb.raw_line("}")
    sb.record_fold(s_alt_start, s_alt_end)
    sb.blank()

    sb.raw_line("#[derive(Debug, Clone, Copy, PartialEq, Eq)]")
    e_start, _ = sb.line_tokens("pub enum ", (enum_state, "def", enum_state), " {")
    sb.line_tokens("    Ready_", tag, ",")
    sb.line_tokens("    Busy_", tag, "(i64),")
    e_end = sb.raw_line("}")
    sb.record_fold(e_start, e_end)
    sb.blank()

    inh_start, _ = sb.line_tokens("impl ", (struct_rec, "inherent_impl", struct_rec), " {")
    fn_new_start, _ = sb.line_tokens("    pub fn ", (fn_new, "def", fn_new), "(val: i64) -> Self {")
    sb.line_tokens("        Self { ", (field_val, "ref", field_val), ": val }")
    fn_new_end = sb.raw_line("    }")
    sb.record_fold(fn_new_start, fn_new_end)
    fn_sc_start, _ = sb.line_tokens("    pub fn ", (fn_scale, "def", fn_scale), "(&self, mult: i64, offset: i64) -> i64 {")
    sb.line_tokens("        self.", (field_val, "ref", field_val), " * mult + offset")
    fn_sc_end = sb.raw_line("    }")
    sb.record_fold(fn_sc_start, fn_sc_end)
    inh_end = sb.raw_line("}")
    sb.record_fold(inh_start, inh_end)
    sb.blank()

    ib1_start, _ = sb.line_tokens(
        "impl ", (trait_base, "trait_impl_ref", trait_base), " for ", (struct_rec, "trait_impl_target", struct_rec), " {"
    )
    fc1_start, _ = sb.line_tokens("    fn ", (fn_compute, "impl_def", fn_compute), "(&self, factor: i64) -> i64 {")
    sb.line_tokens("        self.", (fn_scale, "call_ref", fn_scale), f"(factor, {factor_const})")
    fc1_end = sb.raw_line("    }")
    sb.record_fold(fc1_start, fc1_end)
    ib1_end = sb.raw_line("}")
    sb.record_fold(ib1_start, ib1_end)
    sb.blank()

    ib2_start, _ = sb.line_tokens(
        "impl ", (trait_base, "trait_impl_ref", trait_base), " for ", (struct_alt, "trait_impl_target", struct_alt), " {"
    )
    fc2_start, _ = sb.line_tokens("    fn ", (fn_compute, "impl_def_alt", fn_compute), "(&self, factor: i64) -> i64 {")
    sb.line_tokens("        self.", (field_w, "ref", field_w), " * factor")
    fc2_end = sb.raw_line("    }")
    sb.record_fold(fc2_start, fc2_end)
    ib2_end = sb.raw_line("}")
    sb.record_fold(ib2_start, ib2_end)
    sb.blank()

    isub_start, _ = sb.line_tokens(
        "impl ", (trait_sub, "trait_impl_ref", trait_sub), " for ", (struct_rec, "subtrait_impl_target", struct_rec), " {"
    )
    fb_start, _ = sb.line_tokens("    fn ", (fn_boost, "impl_def", fn_boost), "(&self, base: i64) -> i64 {")
    sb.line_tokens("        self.", (fn_compute, "call_ref", fn_compute), "(base) + 1")
    fb_end = sb.raw_line("    }")
    sb.record_fold(fb_start, fb_end)
    isub_end = sb.raw_line("}")
    sb.record_fold(isub_start, isub_end)
    sb.blank()

    fcalc_start, _ = sb.line_tokens(
        "pub fn ", (fn_calc, "def", fn_calc), "(rec: &", (struct_rec, "type_ref", struct_rec), ", bonus: i64) -> i64 {"
    )
    sb.line_tokens(
        "    rec.", (fn_scale, "call_ref", fn_scale), "(2, bonus) + rec.", (fn_compute, "call_ref", fn_compute), "(bonus)"
    )
    fcalc_end = sb.raw_line("}")
    sb.record_fold(fcalc_start, fcalc_end)
    sb.blank()

    return {
        "tag": tag,
        "trait_base": trait_base,
        "trait_sub": trait_sub,
        "fn_compute": fn_compute,
        "fn_boost": fn_boost,
        "struct_rec": struct_rec,
        "struct_alt": struct_alt,
        "field_val": field_val,
        "field_w": field_w,
        "enum_state": enum_state,
        "fn_new": fn_new,
        "fn_scale": fn_scale,
        "fn_calc": fn_calc,
        "doc_url": doc_url,
    }


def generate_synth_workspace(out_dir: Path, seed: int = 42, scale: int = 3) -> dict:
    """Generates a compilable 3-crate workspace in `out_dir` and returns the ground-truth manifest."""
    out_dir = Path(out_dir).resolve()
    out_dir.mkdir(parents=True, exist_ok=True)
    _write_cargo_manifests(out_dir)

    rng = random.Random(seed)
    families = []
    sb_models = SourceBuilder("crates/core_pkg/src/models.rs")
    sb_core_lib = SourceBuilder("crates/core_pkg/src/lib.rs")
    sb_service = SourceBuilder("crates/service_pkg/src/lib.rs")
    sb_app = SourceBuilder("crates/app_pkg/src/lib.rs")
    sb_unfmt = SourceBuilder("crates/app_pkg/src/unformatted.rs")

    sb_core_lib.raw_line("pub mod models;")

    for i in range(scale):
        tag = f"f{i}_{rng.randint(100, 999)}"
        factor_const = rng.randint(2, 9)
        fam = _emit_core_family(sb_models, tag, factor_const)

        sb_core_lib.line_tokens(
            "pub use crate::models::{",
            (fam["trait_sub"], "reexport", fam["trait_sub"]), ", ",
            (fam["struct_alt"], "reexport", fam["struct_alt"]), ", ",
            (fam["trait_base"], "reexport", fam["trait_base"]), ", ",
            (fam["struct_rec"], "reexport", fam["struct_rec"]), ", ",
            (fam["enum_state"], "reexport", fam["enum_state"]), ", ",
            (fam["fn_calc"], "reexport", fam["fn_calc"]), "};",
        )

        fn_proc = f"process_batch_{tag}"
        local_item = f"item_{tag}"
        local_step = f"step_{tag}"
        fam["fn_proc"] = fn_proc
        fam["local_item"] = local_item
        fam["local_step"] = local_step

        sb_service.line_tokens(
            "use core_pkg::models::{",
            (fam["trait_sub"], "use_ref", fam["trait_sub"]), ", ",
            (fam["trait_base"], "use_ref", fam["trait_base"]), ", ",
            (fam["struct_rec"], "use_ref", fam["struct_rec"]), ", ",
            (fam["fn_calc"], "use_ref", fam["fn_calc"]), "};",
        )
        p_start, _ = sb_service.line_tokens("pub fn ", (fn_proc, "def", fn_proc), "(seed_val: i64) -> i64 {")
        sb_service.line_tokens(
            "    let ", (local_item, "local_def", local_item), " = ",
            (fam["struct_rec"], "type_ref", fam["struct_rec"]), "::",
            (fam["fn_new"], "call_ref", fam["fn_new"]), "(seed_val);",
        )
        sb_service.line_tokens(
            "    let ", (local_step, "local_def", local_step), " = ",
            (fam["fn_calc"], "call_ref", fam["fn_calc"]), "(&",
            (local_item, "local_ref1", local_item), ", ",
            (f"sig_arg2_{tag}", "sig_arg2", "10"), ");",
        )
        sb_service.line_tokens(
            "    ", (local_item, "local_ref2", local_item), ".",
            (fam["fn_boost"], "call_ref", fam["fn_boost"]), "(",
            (local_step, "local_ref1", local_step), ") + ",
            (local_item, "local_ref3", local_item), ".",
            (fam["fn_compute"], "call_ref", fam["fn_compute"]), "(2)",
        )
        p_end = sb_service.raw_line("}")
        sb_service.record_fold(p_start, p_end)
        sb_service.blank()

        fn_exec = f"execute_app_{tag}"
        fn_test = f"test_execute_app_{tag}"
        fam["fn_exec"] = fn_exec
        fam["fn_test"] = fn_test

        if i == 0:
            sb_app.raw_line("pub mod unformatted;")
        sb_app.line_tokens(
            "use core_pkg::models::{",
            (fam["trait_base"], "use_ref", fam["trait_base"]), ", ",
            (fam["struct_rec"], "use_ref", fam["struct_rec"]), ", ",
            (fam["fn_calc"], "use_ref", fam["fn_calc"]), "};",
        )
        sb_app.line_tokens("use service_pkg::", (fn_proc, "use_ref", fn_proc), ";")
        ex_start, _ = sb_app.line_tokens("pub fn ", (fn_exec, "def", fn_exec), "(input: i64) -> i64 {")
        sb_app.line_tokens("    let batch = ", (fn_proc, "call_ref", fn_proc), "(input);")
        sb_app.line_tokens(
            "    let rec = ", (fam["struct_rec"], "type_ref", fam["struct_rec"]), "::",
            (fam["fn_new"], "call_ref", fam["fn_new"]), "(batch);",
        )
        sb_app.line_tokens(
            "    ", (fam["fn_calc"], "call_ref", fam["fn_calc"]), "(&rec, 5) + rec.",
            (fam["fn_compute"], "call_ref", fam["fn_compute"]), "(3)",
        )
        ex_end = sb_app.raw_line("}")
        sb_app.record_fold(ex_start, ex_end)
        sb_app.raw_line("#[test]")
        t_start, _ = sb_app.line_tokens("fn ", (fn_test, "def", fn_test), "() {")
        sb_app.line_tokens("    assert!(", (fn_exec, "call_ref", fn_exec), "(4) > 0);")
        t_end = sb_app.raw_line("}")
        sb_app.record_fold(t_start, t_end)
        sb_app.blank()

        sb_unfmt.line_tokens(
            "pub fn   ", (f"messy_{tag}", "def", f"messy_{tag}"), "( x:i64,y:i64 )->i64{let   z=x+y;z}"
        )
        families.append(fam)

    builders = [sb_models, sb_core_lib, sb_service, sb_app, sb_unfmt]
    all_by_symbol: Dict[str, List[dict]] = {}
    files_meta = {}
    for b in builders:
        b.verify_spans()
        b.write(out_dir)
        files_meta[b.rel_path] = {
            "line_count": len(b.lines),
            "folds": [{"start_line": s, "end_line": e, "kind": k} for s, e, k in b.folds],
        }
        for sym_id, recs in b.by_symbol.items():
            all_by_symbol.setdefault(sym_id, []).extend(r.to_dict() for r in recs)

    manifest = {
        "seed": seed,
        "scale": scale,
        "workspace_dir": str(out_dir),
        "families": families,
        "files": files_meta,
        "symbols": all_by_symbol,
    }
    (out_dir / "ground_truth.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    return manifest
