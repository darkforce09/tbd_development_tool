//! Keyboard shortcuts that work anywhere in the window, read from the SHORTCUTS table
//! (`studio_canvas::shortcuts`) before anything is drawn.

use eframe::egui;
use studio_canvas::shortcuts::{actions_of, consume, key_focus, take, ShortcutAction, ShortcutOwner};

use super::commands::{run_command, AppCommand};
use super::StudioApp;

/// Whether this frame's keys toggle the search: ⌘K (Ctrl+K) always, `/` only while no widget has
/// keyboard focus, so a slash typed into a text field (or the open palette) stays in the field.
/// Both are consumed.
pub fn search_shortcut(ctx: &egui::Context, palette_open: bool) -> bool {
    let focus = key_focus(ctx, palette_open);
    ctx.input_mut(|i| take(i, focus, ShortcutAction::OpenSearch))
}

impl StudioApp {
    /// Reads the app's shortcuts for this frame. Escape closes the palette when it is open, else
    /// the shortcut sheet (and is taken, so the canvas keeps its selection); then each app key in
    /// the table runs.
    pub(crate) fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let focus = key_focus(ctx, self.palette.open);
        if self.palette.open {
            self.palette_keys(ctx, focus);
        } else if self.shortcut_sheet_open && ctx.input_mut(|i| consume(i, focus, ShortcutAction::Escape)) {
            self.shortcut_sheet_open = false;
        }
        let was_open = self.palette.open;
        for action in actions_of(ShortcutOwner::App) {
            if ctx.input_mut(|i| take(i, focus, action)) {
                self.app_key(action);
            }
        }
        if self.palette.open && !was_open {
            // The `/` that opened the field is not typed into it.
            ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Text(t) if t == "/")));
        }
    }

    /// Does what an app key says. Returns whether the app handles `action`; the canvas and the
    /// palette handle the rest.
    pub(crate) fn app_key(&mut self, action: ShortcutAction) -> bool {
        use ShortcutAction::*;
        match action {
            OpenSearch if self.palette.open => self.close_palette(),
            OpenSearch => self.open_palette(),
            ToggleDebug => run_command(self, AppCommand::ToggleDebug),
            // The palette's keys, read while it is open (app/palette.rs).
            PaletteUp | PaletteDown | PaletteOpen | PaletteReveal | PaletteClose => self.palette_key(action),
            // The canvas's keys (studio_canvas view/input.rs, read while it has the keyboard).
            World | ZoomIn | ZoomOut | ActualSize | Escape => return false,
            // Gestures, shown on the sheet only.
            WheelZoom | Pan => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Key, Modifiers};
    use studio_canvas::shortcuts::{pressed, Chord, KeyFocus, Mods, When, SHORTCUTS};

    /// The focus under which a row's keys count.
    fn focus_for(when: When) -> KeyFocus {
        match when {
            When::Always | When::NoFocus => KeyFocus::Free,
            When::PaletteOpen => KeyFocus::Palette,
        }
    }

    fn key(key: Key, modifiers: Modifiers) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    /// The events that press `chord`, the modifiers first.
    fn chord_events(chord: &Chord) -> Vec<egui::Event> {
        let Chord::Key(mods, k) = *chord else { return Vec::new() };
        let modifiers = if let Mods::Logical(m) = mods { m } else { Modifiers::NONE };
        vec![egui::Event::ModifiersChanged(modifiers), key(k, modifiers)]
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
                toggled = search_shortcut(ui.ctx(), false);
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

    #[test]
    fn command_k_never_reaches_the_text_field() {
        let (toggled, text) = run(
            true,
            vec![key(Key::K, Modifiers::COMMAND), egui::Event::Text("k".into()), key(Key::K, Modifiers::NONE)],
        );
        assert!(toggled);
        assert_eq!(text, "k", "only the plain k's text reached the field");
        let (toggled, _) = run(false, vec![key(Key::K, Modifiers::NONE)]);
        assert!(!toggled, "a plain K is not ⌘K");
    }

    /// Runs one frame of the app's shortcuts with `events`.
    fn app_frame(app: &mut StudioApp, ctx: &egui::Context, events: Vec<egui::Event>) {
        let mut output = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            app.handle_shortcuts(ui.ctx());
        });
        output.textures_delta.clear();
    }

    /// E6 (a), app side: every app key in the table fires and is dispatched.
    #[test]
    fn every_app_shortcut_is_dispatched() {
        for s in SHORTCUTS.iter().filter(|s| !s.display_only()) {
            for chord in s.chords {
                let ctx = egui::Context::default();
                let mut fired = false;
                let focus = focus_for(s.when);
                let mut output =
                    ctx.run_ui(egui::RawInput { events: chord_events(chord), ..Default::default() }, |ui| {
                        fired = ui.input(|i| pressed(i, focus, s.action));
                    });
                output.textures_delta.clear();
                assert!(fired, "{:?} via {chord:?}", s.action);
                match s.action.owner() {
                    ShortcutOwner::App => {
                        assert!(StudioApp::for_test().app_key(s.action), "{:?} is handled", s.action);
                        let mut app = StudioApp::for_test();
                        app_frame(&mut app, &ctx, chord_events(chord));
                        let did = match s.action {
                            ShortcutAction::OpenSearch => app.palette.open,
                            ShortcutAction::ToggleDebug => app.debug.open,
                            other => panic!("{other:?} is not the app's"),
                        };
                        assert!(did, "{:?} via {chord:?} did what the sheet says", s.action);
                    }
                    // Checked in studio_canvas (view/input.rs tests).
                    ShortcutOwner::Canvas => {}
                    ShortcutOwner::Palette => {
                        let mut app = StudioApp::for_test();
                        app.open_palette();
                        app.refresh_palette();
                        app.palette.highlight = 1;
                        app_frame(&mut app, &ctx, chord_events(chord));
                        let did = match s.action {
                            ShortcutAction::PaletteUp => app.palette.highlight == 0,
                            ShortcutAction::PaletteDown => app.palette.highlight == 2,
                            // The second stop, World's neighbour: Code.
                            ShortcutAction::PaletteOpen | ShortcutAction::PaletteReveal => {
                                !app.palette.open
                                    && app.canvas_state.camera.pending
                                        == Some((studio_canvas::CameraTarget::Stop(studio_canvas::Stop::Code), true))
                            }
                            ShortcutAction::PaletteClose => !app.palette.open,
                            other => panic!("{other:?} is not the palette's"),
                        };
                        assert!(did, "{:?} via {chord:?} did what the sheet says", s.action);
                    }
                    ShortcutOwner::Pointer => panic!("a gesture row has keys"),
                }
            }
        }
    }

    #[test]
    fn escape_closes_the_shortcut_sheet_before_the_canvas_sees_it() {
        let ctx = egui::Context::default();
        let mut app = StudioApp::for_test();
        app.shortcut_sheet_open = true;
        let mut left = true;
        let events = vec![key(Key::Escape, Modifiers::NONE)];
        let mut output = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            app.handle_shortcuts(ui.ctx());
            left = ui.input(|i| i.key_pressed(Key::Escape));
        });
        output.textures_delta.clear();
        assert!(!app.shortcut_sheet_open);
        assert!(!left, "the selection stays");
    }

    /// Calls that read keys. Only `shortcuts.rs` files may make them, so every app shortcut is in
    /// the table.
    const KEY_READS: [&str; 6] =
        ["key_pressed(", "consume_key(", "consume_shortcut(", "key_down(", "key_released(", "num_presses("];

    /// Files that read keys which are not app shortcuts, with why.
    const ALLOWED: [(&str, &str); 0] = [];

    fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    /// E6 (b): no key read for an app shortcut lives outside the table.
    #[test]
    fn no_key_is_read_outside_the_shortcut_table() {
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let mut files = Vec::new();
        rust_files(&crates.join("studio_viewer/src/app"), &mut files);
        rust_files(&crates.join("studio_canvas/src/view"), &mut files);
        assert!(files.len() > 10);
        let mut found = Vec::new();
        for path in files {
            let rel = path.strip_prefix(crates).unwrap().to_string_lossy().replace('\\', "/");
            if path.file_name().is_some_and(|n| n == "shortcuts.rs" || n == "tests.rs")
                || ALLOWED.iter().any(|(f, _)| *f == rel)
            {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            // Only the code before the test module counts.
            let code = text.split("#[cfg(test)]\nmod tests {").next().unwrap_or("");
            for (n, line) in code.lines().enumerate() {
                if KEY_READS.iter().any(|call| line.contains(call)) {
                    found.push(format!("{rel}:{}: {}", n + 1, line.trim()));
                }
            }
        }
        assert!(found.is_empty(), "read these keys through studio_canvas::shortcuts:\n{}", found.join("\n"));
    }
}
