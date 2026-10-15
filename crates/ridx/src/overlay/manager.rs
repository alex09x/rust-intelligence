/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Document overlay session management across open editor buffers.

use super::{DefKind, DocumentOverlay, OverlayDefinition, OverlaySymbol};
use std::collections::HashMap;

/// Thread-safe manager for all open document overlays in an active LSP session.
#[derive(Default)]
pub struct OverlayManager {
    overlays: HashMap<String, DocumentOverlay>,
}

impl OverlayManager {
    pub fn new() -> Self {
        Self {
            overlays: HashMap::new(),
        }
    }

    #[allow(dead_code)]
    pub fn update(&mut self, file_path: &str, version: i32, text: String) -> bool {
        if let Some(existing) = self.overlays.get_mut(file_path) {
            existing.update(version, text);
            true
        } else {
            false
        }
    }

    pub fn apply_content_changes(
        &mut self,
        file_path: &str,
        version: i32,
        changes: &[serde_json::Value],
    ) -> bool {
        if let Some(existing) = self.overlays.get_mut(file_path) {
            existing.apply_content_changes(version, changes);
            true
        } else {
            false
        }
    }

    #[allow(dead_code)]
    pub fn open_or_update(&mut self, uri: &str, file_path: &str, version: i32, text: String) {
        if let Some(existing) = self.overlays.get_mut(file_path) {
            existing.update(version, text);
        } else {
            let overlay = DocumentOverlay::new_with_provenance(uri, file_path, version, text, true);
            self.overlays.insert(file_path.to_string(), overlay);
        }
    }

    #[allow(dead_code)]
    pub fn open_document(&mut self, uri: &str, file_path: &str, version: i32, text: String) {
        self.open_with_provenance(uri, file_path, version, text, true);
    }

    #[allow(dead_code)]
    pub fn update_document(&mut self, file_path: &str, version: i32, text: String) -> bool {
        self.update(file_path, version, text)
    }

    #[allow(dead_code)]
    pub fn apply_incremental_change(
        &mut self,
        file_path: &str,
        version: i32,
        changes: &[serde_json::Value],
    ) -> bool {
        self.apply_content_changes(file_path, version, changes)
    }

    #[allow(dead_code)]
    pub fn save_document(&mut self, file_path: &str) {
        if let Some(overlay) = self.overlays.get_mut(file_path) {
            overlay.disk_text = Some(overlay.text.clone());
        }
    }

    #[allow(dead_code)]
    pub fn close_document(&mut self, file_path: &str) {
        self.close(file_path);
    }

    pub fn open_with_provenance(
        &mut self,
        uri: &str,
        file_path: &str,
        version: i32,
        text: String,
        is_compiled: bool,
    ) {
        if let Some(existing) = self.overlays.get_mut(file_path) {
            existing.update(version, text);
            if is_compiled {
                existing.compiled_text = Some(existing.text.clone());
            } else {
                existing.compiled_text = None;
            }
        } else {
            let overlay =
                DocumentOverlay::new_with_provenance(uri, file_path, version, text, is_compiled);
            self.overlays.insert(file_path.to_string(), overlay);
        }
    }

    pub fn open_with_source_hash(
        &mut self,
        uri: &str,
        file_path: &str,
        version: i32,
        text: String,
        expected_hash: Option<u64>,
    ) {
        if let Some(existing) = self.overlays.get_mut(file_path) {
            let disk_text = std::fs::read_to_string(file_path).ok();
            existing.update(version, text);
            existing.compiled_text = super::resolve_compiled_text(
                existing.content_hash,
                &existing.text,
                disk_text.as_deref(),
                expected_hash,
            );
            existing.disk_text = disk_text;
        } else {
            let overlay =
                DocumentOverlay::new_with_source_hash(uri, file_path, version, text, expected_hash);
            self.overlays.insert(file_path.to_string(), overlay);
        }
    }

    pub fn close(&mut self, file_path: &str) {
        self.overlays.remove(file_path);
    }

    #[allow(dead_code)]
    pub fn rename(&mut self, old_file_path: &str, new_uri: &str, new_file_path: &str) -> bool {
        if let Some(mut overlay) = self.overlays.remove(old_file_path) {
            overlay.uri = new_uri.to_string();
            overlay.file_path = new_file_path.to_string();
            self.overlays.insert(new_file_path.to_string(), overlay);
            true
        } else {
            false
        }
    }

    pub fn get(&self, file_path: &str) -> Option<&DocumentOverlay> {
        self.overlays.get(file_path)
    }

    #[allow(dead_code)]
    pub fn get_mut(&mut self, file_path: &str) -> Option<&mut DocumentOverlay> {
        self.overlays.get_mut(file_path)
    }

    pub fn find_definition_at(
        &self,
        file_path: &str,
        line: u32,
        col: u32,
    ) -> Option<OverlayDefinition> {
        let overlay = self.overlays.get(file_path)?;
        overlay.find_definition_at(line, col)
    }

    pub fn document_symbols(&self, file_path: &str) -> Option<&[OverlaySymbol]> {
        self.overlays.get(file_path).map(|o| o.document_symbols())
    }

    pub fn find_global_symbol_definition(
        &self,
        current_file: &str,
        ident: &str,
        path_prefix: Option<&str>,
    ) -> Option<OverlayDefinition> {
        // If the current file has a local declaration for this ident, it must not jump elsewhere.
        if let Some(current_ov) = self.overlays.get(current_file) {
            let has_local_binding = current_ov
                .symbols
                .iter()
                .any(|s| s.name == ident && matches!(s.kind, DefKind::Local | DefKind::Field));
            if has_local_binding {
                return None;
            }
        }

        let mut candidates = Vec::new();
        for (file_path, overlay) in &self.overlays {
            for sym in overlay.document_symbols() {
                if sym.name == ident {
                    let is_item_level = matches!(
                        sym.kind,
                        DefKind::Fn
                            | DefKind::Struct
                            | DefKind::Enum
                            | DefKind::Trait
                            | DefKind::TyAlias
                            | DefKind::Const
                            | DefKind::Static
                            | DefKind::Mod
                            | DefKind::Macro
                    );
                    if is_item_level {
                        candidates.push((file_path, sym));
                    }
                }
            }
        }

        if candidates.is_empty() {
            return None;
        }

        if let Some(prefix) = path_prefix {
            let normalized_prefix = prefix
                .trim_start_matches("crate::")
                .trim_start_matches("super::")
                .trim_start_matches("self::")
                .replace("::", "/");

            if normalized_prefix.is_empty() {
                return None;
            }

            for (file_path, sym) in &candidates {
                let p = file_path.replace(std::path::MAIN_SEPARATOR, "/");
                if p.ends_with(&format!("{}.rs", normalized_prefix))
                    || p.ends_with(&format!("{}/mod.rs", normalized_prefix))
                    || p.contains(&format!("/{}", normalized_prefix))
                {
                    return Some(sym.to_definition(file_path));
                }
            }
            // If path prefix was specified but no candidate matched the module path, do not fall back.
            return None;
        }

        // Without a module path prefix, an unqualified identifier can only resolve within the same file.
        for (file_path, sym) in &candidates {
            if *file_path == current_file {
                return Some(sym.to_definition(file_path));
            }
        }

        None
    }

    pub fn reconcile_with_disk(&mut self, file_path: &str, disk_content_hash: u64) {
        if let Some(overlay) = self.overlays.get_mut(file_path) {
            if overlay.content_hash == disk_content_hash {
                overlay.disk_text = Some(overlay.text.clone());
            } else if let Ok(disk) = std::fs::read_to_string(file_path) {
                overlay.disk_text = Some(disk);
            }
        }
    }

    #[allow(dead_code)]
    pub fn map_compiled_line_to_live(&self, file_path: &str, compiled_line: u32) -> Option<u32> {
        self.overlays
            .get(file_path)?
            .map_compiled_line_to_live(compiled_line)
    }

    #[allow(dead_code)]
    pub fn advance_compiled_baseline(&mut self, file_path: &str, text: String) {
        if let Some(overlay) = self.overlays.get_mut(file_path) {
            overlay.advance_compiled_baseline(text);
        }
    }

    #[allow(dead_code)]
    pub fn open_count(&self) -> usize {
        self.overlays.len()
    }

    pub fn open_file_paths(&self) -> Vec<String> {
        self.overlays.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_overlay_manager_lifecycle_and_methods() {
        let mut mgr = OverlayManager::new();
        assert_eq!(mgr.open_count(), 0);
        assert!(mgr.open_file_paths().is_empty());

        let file = "/tmp/test.rs";
        let uri = "file:///tmp/test.rs";
        let text = "fn main() { let x = 1; }\n".to_string();

        mgr.open_document(uri, file, 1, text.clone());
        assert_eq!(mgr.open_count(), 1);
        assert!(mgr.get(file).is_some());
        assert!(mgr.open_file_paths().contains(&file.to_string()));

        // update_document
        assert!(mgr.update_document(file, 2, "fn main() { let x = 2; }\n".to_string()));
        assert!(!mgr.update_document("/nonexistent", 1, "test".to_string()));

        // open_or_update
        mgr.open_or_update(uri, file, 3, "fn main() { let x = 3; }\n".to_string());
        mgr.open_or_update(
            "file:///tmp/other.rs",
            "/tmp/other.rs",
            1,
            "fn other() {}\n".to_string(),
        );
        assert_eq!(mgr.open_count(), 2);

        // rename
        let new_file = "/tmp/renamed.rs";
        let new_uri = "file:///tmp/renamed.rs";
        assert!(mgr.rename(file, new_uri, new_file));
        assert!(mgr.get(new_file).is_some());
        assert!(mgr.get(file).is_none());
        assert!(!mgr.rename("/nonexistent", "file:///tmp/x", "/tmp/x"));

        // save_document & close_document
        mgr.save_document(new_file);
        mgr.close_document(new_file);
        assert!(mgr.get(new_file).is_none());

        // map_compiled_line_to_live & advance_compiled_baseline
        mgr.open_with_provenance(uri, file, 1, "line 1\nline 2\n".to_string(), true);
        assert_eq!(mgr.map_compiled_line_to_live(file, 1), Some(1));
        assert_eq!(mgr.map_compiled_line_to_live("/nonexistent", 1), None);
        mgr.advance_compiled_baseline(file, "line 1\nnew line\nline 2\n".to_string());

        // apply_incremental_change
        let change = serde_json::json!({
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 0, "character": 0 }
            },
            "text": "// comment\n"
        });
        assert!(mgr.apply_incremental_change(file, 2, std::slice::from_ref(&change)));
        assert!(!mgr.apply_incremental_change("/nonexistent", 2, &[change]));

        // reconcile_with_disk
        let hash = mgr.get(file).unwrap().content_hash;
        mgr.reconcile_with_disk(file, hash);
        mgr.reconcile_with_disk(file, 9999);

        let real_tmp =
            std::env::temp_dir().join(format!("ridx_mgr_disk_test_{}.rs", std::process::id()));
        std::fs::write(&real_tmp, b"pub fn disk_fn() {}").unwrap();
        let real_tmp_str = real_tmp.to_string_lossy().to_string();
        mgr.open_with_provenance("uri_disk", &real_tmp_str, 1, "overlay".to_string(), false);
        mgr.reconcile_with_disk(&real_tmp_str, 9999);
        let _ = std::fs::remove_file(&real_tmp);

        // open_with_provenance update with is_compiled false (lines 108-109)
        mgr.open_with_provenance("uri_prov", file, 3, "prov1".to_string(), true);
        mgr.open_with_provenance("uri_prov", file, 4, "prov2".to_string(), false);

        // find_definition_at and document_symbols
        assert!(mgr.document_symbols(file).is_some());
        assert!(mgr.document_symbols("/nonexistent").is_none());
        let _ = mgr.find_definition_at(file, 1, 1);
        assert!(mgr.find_definition_at("/nonexistent", 1, 1).is_none());
    }
}
