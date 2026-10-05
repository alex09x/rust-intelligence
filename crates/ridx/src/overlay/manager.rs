//! Document overlay session management across open editor buffers.

use super::{DocumentOverlay, OverlayDefinition, OverlaySymbol};
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

    pub fn close(&mut self, file_path: &str) {
        self.overlays.remove(file_path);
    }

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
