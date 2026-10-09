//! The View menu: toggles for what the canvas shows. To add a toggle, add a field to
//! `CanvasState` and one entry to `view_toggles`.

use eframe::egui;
use studio_canvas::CanvasState;

/// A View menu entry: label, hover text and the flag it switches.
pub struct ViewToggle<'a> {
    pub label: &'static str,
    pub tooltip: &'static str,
    pub value: &'a mut bool,
}

pub fn view_toggles(canvas: &mut CanvasState) -> [ViewToggle<'_>; 2] {
    [
        ViewToggle { label: "Wires", tooltip: "Show or hide every wire.", value: &mut canvas.show_wires },
        ViewToggle {
            label: "Member wires",
            tooltip:
                "Wires between the functions and types inside files. Off bundles them into one wire per pair of files.",
            value: &mut canvas.show_subnode_wires_globally,
        },
    ]
}

/// The View dropdown. Ticking an entry keeps the menu open.
pub fn view_menu(ui: &mut egui::Ui, canvas: &mut CanvasState) {
    ui.menu_button("View", |ui| {
        for toggle in view_toggles(canvas) {
            ui.checkbox(toggle.value, toggle.label).on_hover_text(toggle.tooltip);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_switch_their_canvas_flags() {
        let mut canvas = CanvasState::default();
        assert!(canvas.show_wires && canvas.show_subnode_wires_globally);
        for toggle in view_toggles(&mut canvas) {
            *toggle.value = false;
        }
        assert!(!canvas.show_wires);
        assert!(!canvas.show_subnode_wires_globally);
    }
}
