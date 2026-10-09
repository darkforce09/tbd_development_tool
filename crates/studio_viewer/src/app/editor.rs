use eframe::egui;
use egui::{Color32, CornerRadius, FontFamily, FontId, RichText, Stroke};
use std::path::PathBuf;
use studio_graph::NodeId;
use studio_parser::{apply_edit, content_hash, save_and_reparse, EditOrigin};
use studio_ui::color_tokens::*;

use super::StudioApp;

/// Editor target waiting for the user to resolve unsaved changes.
#[derive(Debug, Clone)]
pub struct PendingEditorSwitch {
    pub node_id: NodeId,
    pub member_id: Option<String>,
}

impl StudioApp {
    /// Loads a node's source into the editor. Disk-backed nodes keep an `EditOrigin` so saving
    /// only ever replaces the region that was actually loaded.
    pub(crate) fn open_node_in_editor(&mut self, node_id: NodeId) {
        let Some(node) = self.graph.nodes.get(&node_id) else { return };
        if !node.content.is_editable_text() {
            // Binary, image, oversized, link or unreadable: nothing to load into the editor.
            self.code_editor_node_id = Some(node_id);
            self.code_editor_member_id = None;
            self.code_editor_path = None;
            self.code_editor_origin = None;
            self.code_editor_buffer = String::new();
            self.code_editor_dirty = false;
            self.code_editor_status = None;
            return;
        }
        let path = node.file_path.as_ref().map(PathBuf::from).filter(|p| p.is_file());
        let preloaded = node.source_code.clone().filter(|s| !s.is_empty());

        // A preloaded snippet is only editable in place if it is verbatim file text; otherwise
        // (e.g. module summary cards) fall back to the whole file.
        let disk = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok());
        let preloaded = match (&preloaded, &disk) {
            (Some(src), Some(disk)) if disk.replace("\r\n", "\n").contains(src.as_str()) => preloaded,
            (Some(_), Some(_)) => None,
            _ => preloaded,
        };

        let (buffer, origin) = match (preloaded, &path) {
            (Some(src), Some(_)) => (src.clone(), Some(EditOrigin::Snippet { original: src })),
            (Some(src), None) => (src, None),
            (None, Some(_)) => match disk {
                Some(content) => {
                    let disk_hash = content_hash(&content);
                    (content, Some(EditOrigin::FullFile { disk_hash }))
                }
                None => (node.description.clone(), None),
            },
            (None, None) => (node.description.clone(), None),
        };

        self.code_editor_node_id = Some(node_id);
        self.code_editor_member_id = None;
        self.code_editor_path = if origin.is_some() { path } else { None };
        self.code_editor_origin = origin;
        self.code_editor_buffer = buffer;
        self.code_editor_dirty = false;
        self.code_editor_status = None;
    }

    pub(crate) fn open_member_in_editor(&mut self, node_id: NodeId, member_id: String) {
        let Some(node) = self.graph.nodes.get(&node_id) else { return };
        let Some(member) = node.member_nodes.iter().find(|m| m.id == member_id) else { return };
        let path = node.file_path.as_ref().map(PathBuf::from).filter(|p| p.is_file());

        self.code_editor_status = Some(format!("Inspecting member {}", member.name));
        self.code_editor_buffer = member.source_code.clone();
        self.code_editor_origin = path.as_ref().map(|_| EditOrigin::Snippet { original: member.source_code.clone() });
        self.code_editor_path = path;
        self.code_editor_node_id = Some(node_id);
        self.code_editor_member_id = Some(member_id);
        self.code_editor_dirty = false;
    }

    /// Replaces the editor buffer with the whole file read fresh from disk.
    pub(crate) fn load_full_file_in_editor(&mut self) {
        let Some(node_id) = self.code_editor_node_id else { return };
        let path = self
            .graph
            .nodes
            .get(&node_id)
            .and_then(|n| n.file_path.as_ref())
            .map(PathBuf::from)
            .filter(|p| p.is_file());
        let Some(path) = path else {
            self.code_editor_member_id = None;
            self.open_node_in_editor(node_id);
            return;
        };
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                self.code_editor_origin = Some(EditOrigin::FullFile { disk_hash: content_hash(&content) });
                self.code_editor_buffer = content;
                self.code_editor_path = Some(path);
                self.code_editor_member_id = None;
                self.code_editor_dirty = false;
                self.code_editor_status = Some("Loaded full file".to_string());
            }
            Err(e) => self.code_editor_status = Some(format!("⚠ Could not read {}: {}", path.display(), e)),
        }
    }

    /// Keeps the editor in step with the canvas selection. If the buffer has unsaved edits,
    /// the selection change is held back and the unsaved-changes prompt is shown instead.
    pub(crate) fn sync_editor_with_selection(&mut self) {
        let Some(selected) = self.canvas_state.selected_nodes.iter().next().copied() else { return };
        if self.code_editor_node_id == Some(selected) {
            return;
        }
        if self.code_editor_dirty {
            if let Some(current) = self.code_editor_node_id.filter(|id| self.graph.nodes.contains_key(id)) {
                self.pending_editor_switch = Some(PendingEditorSwitch { node_id: selected, member_id: None });
                self.canvas_state.selected_nodes.clear();
                self.canvas_state.selected_nodes.insert(current);
                return;
            }
        }
        self.open_node_in_editor(selected);
        let is_markdown = self.graph.nodes.get(&selected).is_some_and(|node| {
            let lang = studio_ui::detect_language(node.file_path.as_deref(), node.badge.as_deref());
            lang == "md" || lang == "markdown" || node.badge.as_deref() == Some("MD")
        });
        if is_markdown {
            self.markdown_preview_mode = true;
            self.markdown_inspector_tab = 0;
        }
    }

    /// Opens a file member in the editor, prompting first if the current buffer has unsaved edits.
    pub(crate) fn request_open_member(&mut self, node_id: NodeId, member_id: String) {
        let same_target = self.code_editor_node_id == Some(node_id)
            && self.code_editor_member_id.as_deref() == Some(member_id.as_str());
        if self.code_editor_dirty && !same_target {
            self.pending_editor_switch = Some(PendingEditorSwitch { node_id, member_id: Some(member_id) });
            return;
        }
        self.open_member_in_editor(node_id, member_id);
    }

    fn apply_pending_editor_switch(&mut self) {
        let Some(pending) = self.pending_editor_switch.take() else { return };
        self.code_editor_dirty = false;
        self.canvas_state.selected_nodes.clear();
        self.canvas_state.selected_nodes.insert(pending.node_id);
        match pending.member_id {
            Some(member_id) => self.open_member_in_editor(pending.node_id, member_id),
            None => self.open_node_in_editor(pending.node_id),
        }
    }

    /// Saves the editor buffer. Returns true when the buffer is no longer dirty.
    pub fn save_current_editor_code(&mut self) -> bool {
        if !self.code_editor_dirty {
            self.code_editor_status = Some("No changes to save".to_string());
            return true;
        }

        let (Some(path), Some(origin)) = (self.code_editor_path.clone(), self.code_editor_origin.clone()) else {
            return self.save_editor_in_memory();
        };

        let disk = match std::fs::read_to_string(&path) {
            Ok(d) => d,
            Err(e) => {
                self.code_editor_status = Some(format!("⚠ Save Error: could not read {}: {}", path.display(), e));
                return false;
            }
        };
        let new_content = match apply_edit(&disk, &origin, &self.code_editor_buffer) {
            Ok(c) => c,
            Err(e) => {
                self.code_editor_status = Some(format!("⚠ Not saved: {}", e));
                return false;
            }
        };

        match save_and_reparse(&path, &new_content, &mut self.graph) {
            Ok(report) => {
                self.code_editor_origin = Some(match origin {
                    EditOrigin::FullFile { .. } => EditOrigin::FullFile { disk_hash: content_hash(&new_content) },
                    EditOrigin::Snippet { .. } => EditOrigin::Snippet { original: self.code_editor_buffer.clone() },
                });
                self.code_editor_dirty = false;
                self.code_editor_status = Some(if report.needs_reload {
                    format!(
                        "✔ Saved ({} nodes updated). Items changed; reload the project to refresh the layout.",
                        report.updated_nodes
                    )
                } else {
                    format!("✔ Saved to disk! ({} nodes updated)", report.updated_nodes)
                });
                self.canvas_state.status_message = Some(format!("Hot reloaded {}", path.display()));
                true
            }
            Err(e) => {
                self.code_editor_status = Some(format!("⚠ Save Error: {}", e));
                false
            }
        }
    }

    /// Mock/showcase graphs have no backing file: update the model only.
    fn save_editor_in_memory(&mut self) -> bool {
        let Some(node) = self.code_editor_node_id.and_then(|id| self.graph.nodes.get_mut(&id)) else {
            return false;
        };
        match &self.code_editor_member_id {
            Some(member_id) => {
                if let Some(member) = node.member_nodes.iter_mut().find(|m| &m.id == member_id) {
                    member.source_code = self.code_editor_buffer.clone();
                }
            }
            None => node.source_code = Some(self.code_editor_buffer.clone()),
        }
        self.code_editor_dirty = false;
        self.code_editor_status = Some("✔ Updated in-memory model".to_string());
        true
    }

    pub(crate) fn render_unsaved_changes_modal(&mut self, ctx: &egui::Context) {
        if self.pending_editor_switch.is_none() {
            return;
        }

        let mut choice = None;
        egui::Window::new("unsaved_changes_modal")
            .title_bar(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .resizable(false)
            .frame(
                egui::Frame::NONE
                    .fill(Color32::from_rgb(18, 20, 29))
                    .stroke(Stroke::new(1.5, Color32::from_rgb(251, 146, 60)))
                    .corner_radius(CornerRadius::from(12.0))
                    .inner_margin(egui::Margin::same(18)),
            )
            .show(ctx, |ui| {
                ui.label(
                    RichText::new("Unsaved changes")
                        .font(FontId::new(14.0, FontFamily::Proportional))
                        .strong()
                        .color(TEXT_HIGHLIGHT),
                );
                ui.add_space(6.0);
                let target = self
                    .code_editor_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "the current node".to_string());
                ui.label(
                    RichText::new(format!("Save your edits to {} before switching?", target)).color(TEXT_SECONDARY),
                );
                if let Some(status) = &self.code_editor_status {
                    ui.label(RichText::new(status).font(FontId::new(10.0, FontFamily::Monospace)).color(TEXT_DIM));
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        choice = Some(UnsavedChoice::Save);
                    }
                    if ui.button("Discard").clicked() {
                        choice = Some(UnsavedChoice::Discard);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(UnsavedChoice::Cancel);
                    }
                });
            });

        match choice {
            Some(UnsavedChoice::Save) if self.save_current_editor_code() => self.apply_pending_editor_switch(),
            Some(UnsavedChoice::Save) => {}
            Some(UnsavedChoice::Discard) => self.apply_pending_editor_switch(),
            Some(UnsavedChoice::Cancel) => self.pending_editor_switch = None,
            None => {}
        }
    }
}

enum UnsavedChoice {
    Save,
    Discard,
    Cancel,
}
