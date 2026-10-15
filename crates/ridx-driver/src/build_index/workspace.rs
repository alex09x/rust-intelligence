/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::*;

pub(super) fn build_workspace_crate_index(tcx: TyCtxt<'_>, ctx: CrateIndexBuildContext<'_>) {
    let CrateIndexBuildContext {
        start,
        local_stable_crate_id,
        caught_panics,
        crate_name,
        crate_version,
        manifest_dir,
        suffix,
        unit_key,
        deps_dir,
        dep_keys_or_names,
        extern_units,
        krate_to_unit,
        out_dir,
        compiler_args,
    } = ctx;
    // Workspace crate: write to out_dir
    if let Err(e) = fs::create_dir_all(out_dir) {
        eprintln!("ridx-driver: failed to create out_dir {:?}: {e}", out_dir);
        return;
    }

    let out_file_path = out_dir.join(format!("{crate_name}{suffix}.ridx"));

    let mut visitor = HirVisitor {
        tcx,
        typeck: None,
        current_body_owner: None,
        package_root: None,
        writer: IndexWriter::new(&crate_name, &crate_version),
        krate_to_unit,
        seen_refs: std::collections::HashSet::new(),
    };
    visitor.writer.set_unit_key(&unit_key);

    extract_crate_items(tcx, &mut visitor, None);

    let file_hashes = collect_file_hashes(tcx, None, manifest_dir.as_deref());

    let snapshot_digest = ridx_format::compute_source_snapshot_digest(&file_hashes);
    let generation = format!("{:016x}", snapshot_digest);
    visitor.writer.set_generation(&generation);
    visitor.writer.set_file_hashes(file_hashes.clone());

    let tmp_file_path = out_dir.join(format!(
        "{crate_name}{suffix}.ridx.tmp.{}",
        std::process::id()
    ));

    match File::create(&tmp_file_path) {
        Ok(f) => {
            let mut writer = BufWriter::new(f);
            let def_count = visitor.writer.definition_count();
            let ref_count = visitor.writer.reference_count();
            let type_count = visitor.writer.type_count();

            if let Err(e) = visitor.writer.write_to(&mut writer) {
                eprintln!("ridx-driver: write failed for {crate_name}: {e}");
                let _ = fs::remove_file(&tmp_file_path);
            } else {
                drop(writer);
                if let Err(e) = fs::rename(&tmp_file_path, &out_file_path) {
                    eprintln!(
                        "ridx-driver: failed to atomically rename {:?} to {:?}: {e}",
                        tmp_file_path, out_file_path
                    );
                    let _ = fs::remove_file(&tmp_file_path);
                } else {
                    // Publish sidecar metadata only after .ridx file is published
                    let mut env_map = std::collections::HashMap::new();
                    for (k, v) in std::env::vars() {
                        if k.starts_with("CARGO_") || k == "OUT_DIR" || k == "TARGET" || k == "HOST"
                        {
                            env_map.insert(k, v);
                        }
                    }
                    if let Ok(out_dir_val) = std::env::var("OUT_DIR") {
                        let out_path = Path::new(&out_dir_val);
                        if let Some(build_dir) = out_path.parent() {
                            for cand in ["output", "root-output"] {
                                let cand_file = build_dir.join(cand);
                                if let Ok(contents) = fs::read_to_string(&cand_file) {
                                    for line in contents.lines() {
                                        let trimmed = line.trim();
                                        let env_payload = trimmed
                                            .strip_prefix("cargo:rustc-env=")
                                            .or_else(|| trimmed.strip_prefix("cargo::rustc-env="));
                                        if let Some(payload) = env_payload {
                                            if let Some((k, v)) = payload.split_once('=') {
                                                env_map.insert(
                                                    k.trim().to_string(),
                                                    v.trim().to_string(),
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    let cas_key = sysroot::compute_sysroot_cas_key(tcx);
                    let sysroot_src = sysroot::discover_rust_src_path(tcx);
                    sysroot::ensure_sysroot_indexed(tcx, sysroot_src.as_deref());

                    let target_kind = determine_target_kind(compiler_args);
                    let dep_info = serde_json::json!({
                        "name": crate_name,
                        "version": crate_version,
                        "unit_key": unit_key,
                        "generation": generation,
                        "object_path": out_file_path.display().to_string(),
                        "source_root": manifest_dir.as_ref().map(|p| p.display().to_string()),
                        "target_kind": target_kind.as_str(),
                        "is_dependency": false,
                        "dependencies": dep_keys_or_names,
                        "extern_units": extern_units,
                        "file_hashes": file_hashes,
                        "compiler_args": compiler_args,
                        "env": env_map,
                        "stable_crate_id": format!("{:016x}", local_stable_crate_id),
                        "caught_panics": caught_panics,
                        "sysroot_cas_key": cas_key,
                    });
                    write_sidecar_metadata(
                        &deps_dir,
                        &crate_name,
                        &suffix,
                        local_stable_crate_id,
                        &dep_info,
                    );

                    let file_size = fs::metadata(&out_file_path).map(|m| m.len()).unwrap_or(0);
                    if caught_panics > 0 {
                        eprintln!(
                            "ridx: indexed {} ({} defs, {} refs, {} types, {} KB in {:.1} ms, CAUGHT PANICS: {})",
                            crate_name,
                            def_count,
                            ref_count,
                            type_count,
                            file_size / 1024,
                            start.elapsed().as_secs_f64() * 1000.0,
                            caught_panics
                        );
                    } else {
                        eprintln!(
                            "ridx: indexed {} ({} defs, {} refs, {} types, {} KB in {:.1} ms)",
                            crate_name,
                            def_count,
                            ref_count,
                            type_count,
                            file_size / 1024,
                            start.elapsed().as_secs_f64() * 1000.0
                        );
                    }
                }
            }
        }
        Err(e) => {
            eprintln!(
                "ridx-driver: failed to create temporary index {:?}: {e}",
                tmp_file_path
            );
        }
    }
}
