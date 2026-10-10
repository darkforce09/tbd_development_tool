//! Keyboard shortcuts that work anywhere in the window.

use eframe::egui;
use egui::{Key, KeyboardShortcut, Modifiers};

/// Whether this frame's keys toggle the search: ⌘K (Ctrl+K) always, `/` only while no widget has
/// keyboard focus, so a slash typed into a text field stays in the field. Both are consumed.
pub fn search_shortcut(ctx: &egui::Context) -> bool {
    let command_k = ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::K)));
    let slash = !ctx.egui_wants_keyboard_input() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Slash));
    command_k || slash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: Key, modifiers: Modifiers) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    /// Runs frames with a text field, focused when `focus`, and returns whether the last frame
    /// toggled the search, and the field's text.
    fn run(focus: bool, last: Vec<egui::Event>) -> (bool, String) {
        let ctx = egui::Context::default();
        let mut text = String::new();
        let mut toggled = false;
        for (i, events) in [Vec::new(), Vec::new(), last].into_iter().enumerate() {
            let raw = egui::RawInput { events, time: Some(i as f64 * 0.1), ..Default::default() };
            let mut output = ctx.run_ui(raw, |ui| {
                toggled = search_shortcut(ui.ctx());
                let field = ui.add(egui::TextEdit::singleline(&mut text));
                if focus && i == 0 {
                    field.request_focus();
                }
            });
            output.textures_delta.clear();
        }
        (toggled, text)
    }

    #[test]
    fn slash_typed_into_a_text_field_does_not_toggle_search() {
        let slash = || vec![key(Key::Slash, Modifiers::NONE), egui::Event::Text("/".into())];
        let (toggled, text) = run(true, slash());
        assert!(!toggled, "the slash belongs to the field");
        assert_eq!(text, "/");

        let (toggled, text) = run(false, slash());
        assert!(toggled, "with nothing focused, / opens the search");
        assert_eq!(text, "");
    }

    #[test]
    fn command_k_toggles_search_even_from_a_text_field() {
        let (toggled, _) = run(true, vec![key(Key::K, Modifiers::COMMAND)]);
        assert!(toggled);
    }
}
