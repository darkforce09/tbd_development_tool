//! Every key the app reacts to, in one table. Key dispatch reads it through [`pressed`] and
//! [`consume`], and the keyboard-shortcut sheet lists it, so the sheet always says what the keys
//! do. Pointer gestures are listed too, for the sheet only.

use egui::{Context, Event, InputState, Key, Modifiers};

/// What a shortcut does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ShortcutAction {
    /// Puts the cursor in the search field.
    OpenSearch,
    /// Opens or closes the debug panel.
    ToggleDebug,
    /// Flies to the whole project.
    World,
    /// Zooms in about the middle of the view.
    ZoomIn,
    /// Zooms out about the middle of the view.
    ZoomOut,
    /// Back to 100%.
    ActualSize,
    /// Closes the card menu, else clears the selection.
    Escape,
    /// Moves the palette's highlight up.
    PaletteUp,
    /// Moves the palette's highlight down.
    PaletteDown,
    /// Opens the highlighted result.
    PaletteOpen,
    /// Shows the highlighted result on the map.
    PaletteReveal,
    /// Closes the palette.
    PaletteClose,
    /// Wheel and pinch zoom; listed for the sheet, read by the canvas's wheel handling.
    WheelZoom,
    /// Dragging pans; listed for the sheet, read by the input gate.
    Pan,
}

/// Who reacts to an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ShortcutOwner {
    /// The app, before anything else is drawn.
    App,
    /// The canvas, while it has the keyboard.
    Canvas,
    /// The search palette, while it is open.
    Palette,
    /// Pointer gestures: shown on the sheet, never dispatched as keys.
    Pointer,
}

impl ShortcutAction {
    /// Who reacts to it.
    pub fn owner(self) -> ShortcutOwner {
        use ShortcutAction::*;
        match self {
            OpenSearch | ToggleDebug => ShortcutOwner::App,
            World | ZoomIn | ZoomOut | ActualSize | Escape => ShortcutOwner::Canvas,
            PaletteUp | PaletteDown | PaletteOpen | PaletteReveal | PaletteClose => ShortcutOwner::Palette,
            WheelZoom | Pan => ShortcutOwner::Pointer,
        }
    }

    /// Whether reading it takes the key away from the widgets drawn after, so a text field never
    /// sees it.
    pub fn consumes(self) -> bool {
        use ShortcutAction::*;
        match self {
            OpenSearch | PaletteUp | PaletteDown | PaletteOpen | PaletteReveal | PaletteClose => true,
            ToggleDebug | World | ZoomIn | ZoomOut | ActualSize | Escape | WheelZoom | Pan => false,
        }
    }
}

/// How a chord's modifiers must match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mods {
    /// Any modifiers.
    Any,
    /// No ⌘, Ctrl or Alt held; Shift is free (it makes `+` on many layouts).
    Plain,
    /// The key event's modifiers match these logically: extra Shift and Alt are ignored, ⌘ means
    /// Ctrl off macOS (egui's `matches_logically`).
    Logical(Modifiers),
}

/// A pointer gesture, shown on the sheet only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gesture {
    Wheel,
    CommandWheel,
    Pinch,
    Drag,
}

/// One way to trigger a shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chord {
    Key(Mods, Key),
    Gesture(Gesture),
}

/// Where a shortcut is listed on the sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ShortcutGroup {
    General,
    Map,
    Search,
    Pointer,
}

impl ShortcutGroup {
    /// Every group, in the sheet's order.
    pub const ALL: [ShortcutGroup; 4] =
        [ShortcutGroup::General, ShortcutGroup::Map, ShortcutGroup::Search, ShortcutGroup::Pointer];

    pub fn label(self) -> &'static str {
        match self {
            ShortcutGroup::General => "General",
            ShortcutGroup::Map => "Map",
            ShortcutGroup::Search => "Search",
            ShortcutGroup::Pointer => "Mouse and trackpad",
        }
    }
}

/// When a shortcut's keys count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    /// Always, even while a text field has focus.
    Always,
    /// Only while no widget has keyboard focus, so typing never triggers it.
    NoFocus,
    /// Only while the search palette is open.
    PaletteOpen,
}

/// Who has the keyboard this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyFocus {
    /// No widget: keys are for the app and the canvas.
    Free,
    /// A widget, such as a text field.
    Widget,
    /// The open search palette.
    Palette,
}

impl When {
    /// Whether keys count under `focus`.
    pub fn allows(self, focus: KeyFocus) -> bool {
        match self {
            When::Always => true,
            When::NoFocus => focus == KeyFocus::Free,
            When::PaletteOpen => focus == KeyFocus::Palette,
        }
    }
}

/// A row of the table.
#[derive(Debug, Clone, Copy)]
pub struct Shortcut {
    pub action: ShortcutAction,
    pub chords: &'static [Chord],
    pub label: &'static str,
    pub group: ShortcutGroup,
    pub when: When,
}

impl Shortcut {
    /// A gesture row: shown on the sheet, never dispatched.
    pub fn display_only(&self) -> bool {
        self.chords.iter().all(|c| matches!(c, Chord::Gesture(_)))
    }
}

const fn key(mods: Mods, key: Key) -> Chord {
    Chord::Key(mods, key)
}

/// Every key the app reacts to, in the sheet's order.
pub static SHORTCUTS: &[Shortcut] = &[
    Shortcut {
        action: ShortcutAction::OpenSearch,
        chords: &[key(Mods::Logical(Modifiers::COMMAND), Key::K)],
        label: "Search",
        group: ShortcutGroup::General,
        when: When::Always,
    },
    Shortcut {
        action: ShortcutAction::OpenSearch,
        chords: &[key(Mods::Logical(Modifiers::NONE), Key::Slash)],
        label: "Search, while not typing",
        group: ShortcutGroup::General,
        when: When::NoFocus,
    },
    Shortcut {
        action: ShortcutAction::ToggleDebug,
        chords: &[key(Mods::Any, Key::F3)],
        label: "Debug panel",
        group: ShortcutGroup::General,
        when: When::Always,
    },
    Shortcut {
        action: ShortcutAction::World,
        chords: &[key(Mods::Any, Key::Home)],
        label: "The whole project",
        group: ShortcutGroup::Map,
        when: When::NoFocus,
    },
    Shortcut {
        action: ShortcutAction::ZoomIn,
        chords: &[key(Mods::Plain, Key::Equals), key(Mods::Plain, Key::Plus)],
        label: "Zoom in",
        group: ShortcutGroup::Map,
        when: When::NoFocus,
    },
    Shortcut {
        action: ShortcutAction::ZoomOut,
        chords: &[key(Mods::Plain, Key::Minus)],
        label: "Zoom out",
        group: ShortcutGroup::Map,
        when: When::NoFocus,
    },
    Shortcut {
        action: ShortcutAction::ActualSize,
        chords: &[key(Mods::Plain, Key::Num0)],
        label: "Actual size (100%)",
        group: ShortcutGroup::Map,
        when: When::NoFocus,
    },
    Shortcut {
        action: ShortcutAction::Escape,
        chords: &[key(Mods::Any, Key::Escape)],
        label: "Close the menu, else clear the selection",
        group: ShortcutGroup::Map,
        when: When::NoFocus,
    },
    Shortcut {
        action: ShortcutAction::PaletteUp,
        chords: &[key(Mods::Any, Key::ArrowUp)],
        label: "Previous result",
        group: ShortcutGroup::Search,
        when: When::PaletteOpen,
    },
    Shortcut {
        action: ShortcutAction::PaletteDown,
        chords: &[key(Mods::Any, Key::ArrowDown)],
        label: "Next result",
        group: ShortcutGroup::Search,
        when: When::PaletteOpen,
    },
    Shortcut {
        action: ShortcutAction::PaletteReveal,
        chords: &[key(Mods::Logical(Modifiers::SHIFT), Key::Enter)],
        label: "Show the result on the map",
        group: ShortcutGroup::Search,
        when: When::PaletteOpen,
    },
    Shortcut {
        action: ShortcutAction::PaletteOpen,
        chords: &[key(Mods::Logical(Modifiers::NONE), Key::Enter)],
        label: "Open the result",
        group: ShortcutGroup::Search,
        when: When::PaletteOpen,
    },
    Shortcut {
        action: ShortcutAction::PaletteClose,
        chords: &[key(Mods::Any, Key::Escape)],
        label: "Close the search",
        group: ShortcutGroup::Search,
        when: When::PaletteOpen,
    },
    Shortcut {
        action: ShortcutAction::WheelZoom,
        chords: &[Chord::Gesture(Gesture::Wheel)],
        label: "Zoom; over an open card's code, scroll it",
        group: ShortcutGroup::Pointer,
        when: When::Always,
    },
    Shortcut {
        action: ShortcutAction::WheelZoom,
        chords: &[Chord::Gesture(Gesture::CommandWheel), Chord::Gesture(Gesture::Pinch)],
        label: "Zoom, even over code",
        group: ShortcutGroup::Pointer,
        when: When::Always,
    },
    Shortcut {
        action: ShortcutAction::Pan,
        chords: &[Chord::Gesture(Gesture::Drag)],
        label: "Pan the map (any button)",
        group: ShortcutGroup::Pointer,
        when: When::Always,
    },
];

/// The key actions `owner` reacts to, each once, in table order.
pub fn actions_of(owner: ShortcutOwner) -> Vec<ShortcutAction> {
    let mut actions: Vec<ShortcutAction> = Vec::new();
    for s in SHORTCUTS.iter().filter(|s| s.action.owner() == owner && !s.display_only()) {
        if !actions.contains(&s.action) {
            actions.push(s.action);
        }
    }
    actions
}

/// Who has the keyboard: the palette while it is open, else any widget with focus.
pub fn key_focus(ctx: &Context, palette_open: bool) -> KeyFocus {
    if palette_open {
        KeyFocus::Palette
    } else if ctx.egui_wants_keyboard_input() {
        KeyFocus::Widget
    } else {
        KeyFocus::Free
    }
}

/// The chords of `action` that count under `focus`.
fn live_chords(focus: KeyFocus, action: ShortcutAction) -> impl Iterator<Item = &'static Chord> {
    SHORTCUTS.iter().filter(move |s| s.action == action && s.when.allows(focus)).flat_map(|s| s.chords.iter())
}

/// Whether a key event is this chord, with the modifiers held now being `held`. `Any` and `Plain`
/// read as egui's `key_pressed` does, repeats included.
fn event_is(event: &Event, chord: &Chord, held: Modifiers) -> bool {
    let Event::Key { key, modifiers, pressed: true, .. } = event else { return false };
    match *chord {
        Chord::Key(Mods::Any, k) => *key == k,
        Chord::Key(Mods::Plain, k) => *key == k && !(held.command || held.ctrl || held.alt),
        Chord::Key(Mods::Logical(pattern), k) => *key == k && modifiers.matches_logically(pattern),
        Chord::Gesture(_) => false,
    }
}

/// Whether one of `action`'s keys was pressed this frame (key repeats count), under `focus`. With
/// [`consume`], the only way the app and the canvas read their shortcut keys.
pub fn pressed(input: &InputState, focus: KeyFocus, action: ShortcutAction) -> bool {
    live_chords(focus, action).any(|chord| input.events.iter().any(|e| event_is(e, chord, input.modifiers)))
}

/// Like [`pressed`], and takes the matching key events away, so widgets drawn after never see
/// them (⌘K stays out of a focused text field).
pub fn consume(input: &mut InputState, focus: KeyFocus, action: ShortcutAction) -> bool {
    let held = input.modifiers;
    let mut hit = false;
    for chord in live_chords(focus, action) {
        input.events.retain(|e| {
            let is = event_is(e, chord, held);
            hit |= is;
            !is
        });
    }
    hit
}

/// Reads `action` the way its table row says: consumed or only looked at.
pub fn take(input: &mut InputState, focus: KeyFocus, action: ShortcutAction) -> bool {
    if action.consumes() {
        consume(input, focus, action)
    } else {
        pressed(input, focus, action)
    }
}

/// How a chord is spelled on the sheet: "⌘K" on macOS, "Ctrl K" elsewhere, as the title bar
/// spells it.
pub fn chord_text(chord: &Chord, mac: bool) -> String {
    match *chord {
        Chord::Key(mods, k) => {
            let mut text = String::new();
            if let Mods::Logical(m) = mods {
                if m.command || m.ctrl || m.mac_cmd {
                    text.push_str(if mac { "⌘" } else { "Ctrl " });
                }
                if m.alt {
                    text.push_str(if mac { "⌥" } else { "Alt " });
                }
                if m.shift {
                    text.push_str(if mac { "⇧" } else { "Shift " });
                }
            }
            text.push_str(key_text(k));
            text
        }
        Chord::Gesture(g) => match g {
            Gesture::Wheel => "Wheel".into(),
            Gesture::CommandWheel => (if mac { "⌘ Wheel" } else { "Ctrl Wheel" }).into(),
            Gesture::Pinch => "Pinch".into(),
            Gesture::Drag => "Drag".into(),
        },
    }
}

fn key_text(k: Key) -> &'static str {
    match k {
        Key::ArrowUp => "↑",
        Key::ArrowDown => "↓",
        Key::Enter => "Enter",
        Key::Escape => "Esc",
        Key::Equals => "=",
        Key::Plus => "+",
        Key::Minus => "-",
        Key::Num0 => "0",
        Key::Slash => "/",
        other => other.name(),
    }
}

/// Every chord of a row, spelled for the sheet and joined: "= or +".
pub fn shortcut_text(shortcut: &Shortcut, mac: bool) -> String {
    shortcut.chords.iter().map(|c| chord_text(c, mac)).collect::<Vec<_>>().join(" or ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_event(key: Key, modifiers: Modifiers) -> Event {
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    /// Runs one frame with `events` (the modifiers held being the last event's) and returns what
    /// `read` saw.
    fn frame<T>(mut events: Vec<Event>, read: impl FnOnce(&mut InputState) -> T) -> T {
        let ctx = Context::default();
        if let Some(Event::Key { modifiers, .. }) = events.last() {
            events.insert(0, Event::ModifiersChanged(*modifiers));
        }
        let mut out = None;
        let mut read = Some(read);
        let raw = egui::RawInput { events, ..Default::default() };
        let mut output = ctx.run_ui(raw, |ui| {
            if let Some(read) = read.take() {
                out = Some(ui.ctx().input_mut(read));
            }
        });
        output.textures_delta.clear();
        out.unwrap()
    }

    /// The events that press `chord`.
    pub(crate) fn chord_events(chord: &Chord) -> Vec<Event> {
        match *chord {
            Chord::Key(Mods::Logical(m), k) => vec![key_event(k, m)],
            Chord::Key(_, k) => vec![key_event(k, Modifiers::NONE)],
            Chord::Gesture(_) => Vec::new(),
        }
    }

    fn focus_for(when: When) -> KeyFocus {
        match when {
            When::Always | When::NoFocus => KeyFocus::Free,
            When::PaletteOpen => KeyFocus::Palette,
        }
    }

    #[test]
    fn every_key_row_fires_its_action() {
        for s in SHORTCUTS.iter().filter(|s| !s.display_only()) {
            for chord in s.chords {
                let focus = focus_for(s.when);
                assert!(frame(chord_events(chord), |i| pressed(i, focus, s.action)), "{:?} {chord:?}", s.action);
                assert!(frame(chord_events(chord), |i| consume(i, focus, s.action)), "{:?} {chord:?}", s.action);
            }
        }
    }

    #[test]
    fn consume_takes_the_key_away() {
        let left = frame(vec![key_event(Key::K, Modifiers::COMMAND)], |i| {
            let first = consume(i, KeyFocus::Widget, ShortcutAction::OpenSearch);
            (first, consume(i, KeyFocus::Widget, ShortcutAction::OpenSearch), i.key_pressed(Key::K))
        });
        assert_eq!(left, (true, false, false));
    }

    #[test]
    fn when_gates_by_focus() {
        let slash = || vec![key_event(Key::Slash, Modifiers::NONE)];
        assert!(frame(slash(), |i| pressed(i, KeyFocus::Free, ShortcutAction::OpenSearch)));
        assert!(!frame(slash(), |i| pressed(i, KeyFocus::Widget, ShortcutAction::OpenSearch)));
        let cmd_k = || vec![key_event(Key::K, Modifiers::COMMAND)];
        assert!(frame(cmd_k(), |i| pressed(i, KeyFocus::Widget, ShortcutAction::OpenSearch)));
        // One Escape key, two meanings: the palette's while it is open, else the canvas's.
        let esc = || vec![key_event(Key::Escape, Modifiers::NONE)];
        assert!(frame(esc(), |i| pressed(i, KeyFocus::Free, ShortcutAction::Escape)));
        assert!(!frame(esc(), |i| pressed(i, KeyFocus::Free, ShortcutAction::PaletteClose)));
        assert!(frame(esc(), |i| pressed(i, KeyFocus::Palette, ShortcutAction::PaletteClose)));
        assert!(!frame(esc(), |i| pressed(i, KeyFocus::Palette, ShortcutAction::Escape)));
    }

    #[test]
    fn zoom_keys_need_no_command_modifier() {
        let cmd_minus = vec![key_event(Key::Minus, Modifiers::COMMAND)];
        assert!(!frame(cmd_minus, |i| pressed(i, KeyFocus::Free, ShortcutAction::ZoomOut)));
        let shift_plus = vec![key_event(Key::Plus, Modifiers::SHIFT)];
        assert!(frame(shift_plus, |i| pressed(i, KeyFocus::Free, ShortcutAction::ZoomIn)));
    }

    #[test]
    fn shift_enter_reveals_and_plain_enter_opens() {
        let shift_enter = || vec![key_event(Key::Enter, Modifiers::SHIFT)];
        assert!(frame(shift_enter(), |i| pressed(i, KeyFocus::Palette, ShortcutAction::PaletteReveal)));
        let enter = || vec![key_event(Key::Enter, Modifiers::NONE)];
        assert!(frame(enter(), |i| pressed(i, KeyFocus::Palette, ShortcutAction::PaletteOpen)));
        assert!(!frame(enter(), |i| pressed(i, KeyFocus::Palette, ShortcutAction::PaletteReveal)));
        // Shift+Enter also matches Enter logically, so the palette takes Reveal first.
        let open_after_reveal = frame(shift_enter(), |i| {
            (
                consume(i, KeyFocus::Palette, ShortcutAction::PaletteReveal),
                pressed(i, KeyFocus::Palette, ShortcutAction::PaletteOpen),
            )
        });
        assert_eq!(open_after_reveal, (true, false));
    }

    #[test]
    fn gestures_are_listed_but_never_pressed() {
        let gestures: Vec<_> = SHORTCUTS.iter().filter(|s| s.display_only()).collect();
        assert!(gestures.iter().all(|s| s.action.owner() == ShortcutOwner::Pointer));
        assert!(SHORTCUTS.iter().filter(|s| !s.display_only()).all(|s| s.action.owner() != ShortcutOwner::Pointer));
        assert!(actions_of(ShortcutOwner::Pointer).is_empty());
    }

    #[test]
    fn chords_are_spelled_per_platform() {
        let cmd_k = &SHORTCUTS[0].chords[0];
        assert_eq!(chord_text(cmd_k, true), "⌘K");
        assert_eq!(chord_text(cmd_k, false), "Ctrl K");
        let zoom_in = SHORTCUTS.iter().find(|s| s.action == ShortcutAction::ZoomIn).unwrap();
        assert_eq!(shortcut_text(zoom_in, false), "= or +");
        let reveal = SHORTCUTS.iter().find(|s| s.action == ShortcutAction::PaletteReveal).unwrap();
        assert_eq!(shortcut_text(reveal, true), "⇧Enter");
        assert_eq!(shortcut_text(reveal, false), "Shift Enter");
    }

    #[test]
    fn owners_list_their_actions_once_in_table_order() {
        use ShortcutAction::*;
        assert_eq!(actions_of(ShortcutOwner::App), vec![OpenSearch, ToggleDebug]);
        assert_eq!(actions_of(ShortcutOwner::Canvas), vec![World, ZoomIn, ZoomOut, ActualSize, Escape]);
        assert_eq!(
            actions_of(ShortcutOwner::Palette),
            vec![PaletteUp, PaletteDown, PaletteReveal, PaletteOpen, PaletteClose]
        );
    }

    #[test]
    fn every_row_has_a_group_on_the_sheet() {
        for s in SHORTCUTS {
            assert!(ShortcutGroup::ALL.contains(&s.group));
            assert!(!s.label.is_empty() && !s.chords.is_empty());
        }
    }
}
