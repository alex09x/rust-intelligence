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

pub(super) fn build_immutable_dependency_index(tcx: TyCtxt<'_>, ctx: CrateIndexBuildContext<'_>) {
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
        out_dir: _,
        compiler_args,
    } = ctx;
    let cache_dir = ridx_format::resolve_cache_dir();
    let obj_path = ridx_format::cache_object_path(&cache_dir, &unit_key);

    let is_valid_cache = if obj_path.is_file() {
        if let Ok(reader) = ridx_format::IndexReader::open(&obj_path) {
            reader.header().version == ridx_format::CURRENT_FORMAT_VERSION && reader.unit_key() == unit_key
        } else {
            false
        }
    } else {
        false
    };

    if is_valid_cache {
        let (generation, file_hashes) = {
            let reader = ridx_format::IndexReader::open(&obj_path).unwrap();
            let gen = reader.generation().to_string();
            let embedded = reader.embedded_file_hashes();
            let mut hashes = collect_file_hashes(tcx, manifest_dir.as_deref(), manifest_dir.as_deref());
            ridx_format::merge_embedded_hashes_authoritative(&mut hashes, &embedded, manifest_dir.as_deref());
            (gen, hashes)
        };
        let cas_key = sysroot::compute_sysroot_cas_key(tcx);
        let dep_info = serde_json::json!({
            "name": crate_name,
            "version": crate_version,
            "unit_key": unit_key,
            "driver_hash": format!("{:016x}", ridx_format::get_driver_hash()),
            "format_version": ridx_format::CURRENT_FORMAT_VERSION,
            "generation": generation,
            "object_path": obj_path.display().to_string(),
            "source_root": manifest_dir.as_ref().map(|p| p.display().to_string()),
            "target_kind": "lib",
            "is_dependency": true,
            "dependencies": dep_keys_or_names,
            "extern_units": extern_units,
            "file_hashes": file_hashes,
            "compiler_args": compiler_args,
            "stable_crate_id": format!("{:016x}", local_stable_crate_id),
            "caught_panics": caught_panics,
            "sysroot_cas_key": cas_key,
        });
        write_sidecar_metadata(&deps_dir, &crate_name, &suffix, local_stable_crate_id, &dep_info);

        let file_size = fs::metadata(&obj_path).map(|m| m.len()).unwrap_or(0);
        eprintln!(
            "ridx: cache hit for {} v{} ({} KB, unit: {}) in {:.1} ms",
            crate_name,
            crate_version,
            file_size / 1024,
            &unit_key[..unit_key.len().min(8)],
            start.elapsed().as_secs_f64() * 1000.0
        );
        return;
    } else if obj_path.is_file() {
        let _ = fs::remove_file(&obj_path);
    }

    // Cache miss: extract index with package_root set for relative source paths
    let mut visitor = HirVisitor {
        tcx,
        typeck: None,
        current_body_owner: None,
        package_root: manifest_dir.clone(),
        writer: IndexWriter::new(&crate_name, &crate_version),
        krate_to_unit: krate_to_unit.clone(),
        seen_refs: std::collections::HashSet::new(),
    };
    visitor.writer.set_unit_key(&unit_key);

    extract_crate_items(tcx, &mut visitor, manifest_dir.as_deref());

    let file_hashes = collect_file_hashes(tcx, manifest_dir.as_deref(), manifest_dir.as_deref());

    let snapshot_digest = ridx_format::compute_source_snapshot_digest(&file_hashes);
    let generation = format!("{:016x}", snapshot_digest);
    visitor.writer.set_generation(&generation);
    visitor.writer.set_file_hashes(file_hashes.clone());

    if let Some(parent) = obj_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let tmp_file_path = obj_path.with_extension(format!("tmp.{}", std::process::id()));
    match File::create(&tmp_file_path) {
        Ok(f) => {
            let mut writer = BufWriter::new(f);
            let def_count = visitor.writer.definition_count();
            let ref_count = visitor.writer.reference_count();
            let type_count = visitor.writer.type_count();

            if let Err(e) = visitor.writer.write_to(&mut writer) {
                eprintln!("ridx-driver: write failed for dependency {crate_name}: {e}");
                let _ = fs::remove_file(&tmp_file_path);
            } else {
                drop(writer);
                if let Err(e) = fs::rename(&tmp_file_path, &obj_path) {
                    eprintln!("ridx-driver: failed to atomically rename {:?} to {:?}: {e}", tmp_file_path, obj_path);
                    let _ = fs::remove_file(&tmp_file_path);
                } else {
                    // Publish sidecar metadata only after .ridx file is published
                    let cas_key = sysroot::compute_sysroot_cas_key(tcx);
                    let dep_info = serde_json::json!({
                        "name": crate_name,
                        "version": crate_version,
                        "unit_key": unit_key,
                        "driver_hash": format!("{:016x}", ridx_format::get_driver_hash()),
                        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
                        "generation": generation,
                        "object_path": obj_path.display().to_string(),
                        "source_root": manifest_dir.as_ref().map(|p| p.display().to_string()),
                        "target_kind": "lib",
                        "is_dependency": true,
                        "dependencies": dep_keys_or_names,
                        "extern_units": extern_units,
                        "file_hashes": file_hashes,
                        "compiler_args": compiler_args,
                        "stable_crate_id": format!("{:016x}", local_stable_crate_id),
                        "caught_panics": caught_panics,
                        "sysroot_cas_key": cas_key,
                    });
                    write_sidecar_metadata(&deps_dir, &crate_name, &suffix, local_stable_crate_id, &dep_info);

                    let file_size = fs::metadata(&obj_path).map(|m| m.len()).unwrap_or(0);
                    if caught_panics > 0 {
                        eprintln!(
                            "ridx: indexed dependency {} v{} ({} defs, {} refs, {} types, {} KB in {:.1} ms, cached: {}, CAUGHT PANICS: {})",
                            crate_name,
                            crate_version,
                            def_count,
                            ref_count,
                            type_count,
                            file_size / 1024,
                            start.elapsed().as_secs_f64() * 1000.0,
                            &unit_key[..unit_key.len().min(8)],
                            caught_panics
                        );
                    } else {
                        eprintln!(
                            "ridx: indexed dependency {} v{} ({} defs, {} refs, {} types, {} KB in {:.1} ms, cached: {})",
                            crate_name,
                            crate_version,
                            def_count,
                            ref_count,
                            type_count,
                            file_size / 1024,
                            start.elapsed().as_secs_f64() * 1000.0,
                            &unit_key[..unit_key.len().min(8)]
                        );
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("ridx-driver: failed to create temporary index {:?}: {e}", tmp_file_path);
        }
    }
}
