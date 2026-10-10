//! The View menu: toggles for what the canvas shows. To add a toggle, add a field to
//! `CanvasState` and one entry to `VIEW_TOGGLES`. Each runs as `AppCommand::ToggleView`, so the
//! menu and the search palette switch the same flags.

use eframe::egui;
use studio_canvas::CanvasState;

/// A View menu entry: label, palette title, hover text and the flag it switches.
pub struct ViewToggle {
    pub label: &'static str,
    /// How the search palette names it: "Show wires".
    pub title: &'static str,
    pub tooltip: &'static str,
    /// Whether it is on.
    pub get: fn(&CanvasState) -> bool,
    /// The flag it switches.
    pub flag: fn(&mut CanvasState) -> &mut bool,
}

/// Every View toggle, in menu order.
pub const VIEW_TOGGLES: [ViewToggle; 9] = [
    ViewToggle {
        label: "Wires",
        title: "Show wires",
        tooltip: "Show or hide every wire.",
        get: |c| c.show_wires,
        flag: |c| &mut c.show_wires,
    },
    ViewToggle {
        label: "Member wires",
        title: "Show member wires",
        tooltip:
            "Wires between the functions and types inside files. Off bundles them into one wire per pair of files.",
        get: |c| c.show_subnode_wires_globally,
        flag: |c| &mut c.show_subnode_wires_globally,
    },
    ViewToggle {
        label: "Calls",
        title: "Show call wires",
        tooltip: "Green: a function calls another.",
        get: |c| c.wire_kinds.calls,
        flag: |c| &mut c.wire_kinds.calls,
    },
    ViewToggle {
        label: "Type uses",
        title: "Show type-use wires",
        tooltip: "Orange: code uses a type defined elsewhere.",
        get: |c| c.wire_kinds.type_uses,
        flag: |c| &mut c.wire_kinds.type_uses,
    },
    ViewToggle {
        label: "Implements",
        title: "Show implements wires",
        tooltip: "Violet: a type implements or inherits.",
        get: |c| c.wire_kinds.implements,
        flag: |c| &mut c.wire_kinds.implements,
    },
    ViewToggle {
        label: "Imports",
        title: "Show import wires",
        tooltip: "Slate: a file imports or includes another.",
        get: |c| c.wire_kinds.imports,
        flag: |c| &mut c.wire_kinds.imports,
    },
    ViewToggle {
        label: "Documentation",
        title: "Show documentation wires",
        tooltip: "Sky blue: documentation links to code.",
        get: |c| c.wire_kinds.documentation,
        flag: |c| &mut c.wire_kinds.documentation,
    },
    ViewToggle {
        label: "Assets",
        title: "Show asset wires",
        tooltip: "Pink: code references an asset.",
        get: |c| c.wire_kinds.assets,
        flag: |c| &mut c.wire_kinds.assets,
    },
    ViewToggle {
        label: "Dependencies",
        title: "Show dependency wires",
        tooltip: "Slate: a package depends on another, by a path dependency in its manifest.",
        get: |c| c.wire_kinds.depends,
        flag: |c| &mut c.wire_kinds.depends,
    },
];

/// The View dropdown. Ticking an entry keeps the menu open. Returns the toggle ticked, for the
/// caller to run as `AppCommand::ToggleView`.
pub fn view_menu(ui: &mut egui::Ui, canvas: &CanvasState) -> Option<usize> {
    let mut ticked = None;
    ui.menu_button("View", |ui| {
        for (i, toggle) in VIEW_TOGGLES.iter().enumerate() {
            let mut on = (toggle.get)(canvas);
            if ui.checkbox(&mut on, toggle.label).on_hover_text(toggle.tooltip).changed() {
                ticked = Some(i);
            }
        }
    });
    ticked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_switch_their_canvas_flags() {
        let mut canvas = CanvasState::default();
        assert!(canvas.show_wires && canvas.show_subnode_wires_globally);
        let docs = 1 << studio_graph::EdgeKind::Documentation as u32;
        assert_eq!(canvas.wire_kinds.mask(), 0b111_1111 & !docs, "every kind but documentation shows by default");
        for toggle in &VIEW_TOGGLES {
            *(toggle.flag)(&mut canvas) = false;
            assert!(!(toggle.get)(&canvas), "{} reads its own flag", toggle.label);
        }
        assert!(!canvas.show_wires);
        assert!(!canvas.show_subnode_wires_globally);
        assert_eq!(canvas.wire_kinds.mask(), 0, "each kind has its own toggle");
    }
}
