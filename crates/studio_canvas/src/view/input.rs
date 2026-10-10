use egui::{CursorIcon, Pos2, Rect, Ui, Vec2};
use studio_graph::Graph;

use crate::camera::{CameraTarget, Stop};
use crate::interaction::{HoverState, InteractionMode};

use super::gate::InputGate;
use super::layout::port_world_position;
use super::shortcuts::{actions_of, pressed, KeyFocus, ShortcutAction, ShortcutOwner};
use super::types::{CanvasAction, CanvasState, NodeContextMenu};

/// Pans, zooms, hovers, selects and opens the context menu, with only the input the canvas owns
/// this frame, on a canvas filling `rect`. Nothing here changes what the map says: wires come
/// from the code, so the canvas never draws, cuts or deletes them, and cards stay where the
/// layout puts them. Any wheel, pinch, drag or zoom key stops a camera flight.
pub fn handle_canvas_input(state: &mut CanvasState, graph: &mut Graph, ui: &Ui, gate: &InputGate, rect: Rect) {
    // 1. Zoom and pan. A drag with any button moves the view.
    if gate.pointer.is_some_and(|pointer| wheel(state, graph, ui, pointer)) {
        state.camera.cancel();
    }
    keys(state, ui, gate, rect);
    if gate.dragging {
        state.camera.cancel();
        if state.interaction == InteractionMode::Idle {
            state.context_menu = None;
        }
        state.interaction = InteractionMode::Panning;
        state.transform.pan_by(gate.drag_delta);
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    } else {
        state.interaction = InteractionMode::Idle;
    }

    // 2. Spatial grid and scene follow the layout; then what is under the pointer.
    state.refresh_scene(graph);
    state.hover = match gate.pointer {
        Some(pointer) if state.interaction == InteractionMode::Idle => hit_test(state, graph, pointer),
        _ => HoverState::default(),
    };

    let Some(pointer) = gate.pointer else { return };
    // Buttons painted on the canvas (card controls, the context menu) take their own clicks.
    let over_button = state.interactive_rects.iter().any(|r| r.contains(pointer));

    // 3. Clicks: select, open, and the context menu.
    if gate.clicked && !over_button {
        state.context_menu = None;
    }
    if gate.double_clicked && !over_button {
        if let Some(node_id) = state.hover.hovered_node {
            state.action_request = Some(CanvasAction::OpenOnDesk(node_id));
        } else if let Some(id) = folder_at(graph, state.transform.screen_to_world(pointer)) {
            // A closed folder opens in place; an open one fills the view.
            if graph.is_cluster_collapsed(&id) {
                state.open_folder(graph, &id);
            } else {
                state.focus = Some(id.clone());
                state.fly_to(CameraTarget::Folder(id));
            }
        }
    } else if let Some(district) = gate
        .clicked
        .then(|| state.world.region_at(state.transform.screen_to_world(pointer)))
        .flatten()
        .filter(|&d| d != state.stop && !over_button)
    {
        // A click on another district (or on any, from the world view) goes there.
        state.fly_to(CameraTarget::Stop(district));
    } else if gate.clicked && !over_button {
        let shift = ui.input(|i| i.modifiers.shift);
        match state.hover.hovered_node {
            // Shift adds a card to the selection, or takes it out again.
            Some(id) if shift => {
                let added = state.selected_nodes.insert(id);
                if !added {
                    state.selected_nodes.remove(&id);
                }
            }
            Some(id) => {
                state.selected_nodes.clear();
                state.selected_nodes.insert(id);
            }
            None if !shift => state.selected_nodes.clear(),
            None => {}
        }
    }
    if gate.secondary_clicked {
        state.context_menu = state.hover.hovered_node.map(|node_id| {
            state.selected_nodes.clear();
            state.selected_nodes.insert(node_id);
            NodeContextMenu { node_id, screen_pos: pointer }
        });
    }
}

/// Mouse wheel and pinch at `pointer`. Pinch and Ctrl/⌘ + wheel always zoom the map; a plain
/// wheel scrolls the code of an open card under the pointer and zooms everywhere else. Returns
/// whether the view zoomed.
fn wheel(state: &mut CanvasState, graph: &mut Graph, ui: &Ui, pointer: Pos2) -> bool {
    let (zoom_key, scroll, zoom_delta) =
        ui.input(|i| (i.modifiers.ctrl || i.modifiers.command, i.smooth_scroll_delta.y, i.zoom_delta()));
    let pinch = (zoom_delta - 1.0).abs() > 0.001;
    let wheel_factor = 1.0 + (scroll * 0.002).clamp(-0.25, 0.25);
    if (zoom_key && scroll.abs() > 0.0) || pinch {
        state.transform.zoom_at_pointer(pointer, if pinch { zoom_delta } else { wheel_factor });
        true
    } else if scroll.abs() > 0.5 && !scroll_open_card(state, graph, pointer, scroll) {
        state.transform.zoom_at_pointer(pointer, wheel_factor);
        true
    } else {
        false
    }
}

/// The canvas's keys from the SHORTCUTS table, read only while it has the keyboard: Home goes to
/// the world view; `=` and `-` zoom about the middle; `0` goes to 100% (where a district's text
/// is crisp); Escape closes the menu, then clears the selection. 100% wins over the zoom keys,
/// and zooming in over zooming out.
///
/// Escape typed into a text field takes the field's focus away before the canvas runs (egui does
/// that as the frame begins), so keys also wait while a widget had the keyboard last frame.
fn keys(state: &mut CanvasState, ui: &Ui, gate: &InputGate, rect: Rect) {
    let typed_last_frame = ui.ctx().data_mut(|d| {
        let id = egui::Id::new("canvas_keys_typing");
        let before = d.get_temp::<bool>(id).unwrap_or(false);
        d.insert_temp(id, !gate.keyboard);
        before
    });
    if !gate.keyboard || typed_last_frame {
        return;
    }
    let mut fired: Vec<ShortcutAction> = ui
        .input(|i| actions_of(ShortcutOwner::Canvas).into_iter().filter(|&a| pressed(i, KeyFocus::Free, a)).collect());
    if fired.contains(&ShortcutAction::ActualSize) {
        fired.retain(|a| !matches!(a, ShortcutAction::ZoomIn | ShortcutAction::ZoomOut));
    } else if fired.contains(&ShortcutAction::ZoomIn) {
        fired.retain(|&a| a != ShortcutAction::ZoomOut);
    }
    for action in fired {
        canvas_key(state, action, rect);
    }
}

/// Does what a canvas key says, on a canvas filling `rect`. Returns whether the canvas handles
/// `action`; the others belong to the app or the palette.
pub(crate) fn canvas_key(state: &mut CanvasState, action: ShortcutAction, rect: Rect) -> bool {
    use ShortcutAction::*;
    match action {
        World => state.fly_to(CameraTarget::Stop(Stop::World)),
        ActualSize => state.actual_size(),
        ZoomIn | ZoomOut => {
            state.camera.cancel();
            state.transform.zoom_at_pointer(rect.center(), if action == ZoomIn { 1.25 } else { 0.8 });
        }
        Escape => {
            if state.context_menu.is_some() {
                state.context_menu = None;
            } else {
                state.selected_nodes.clear();
            }
        }
        // The app's (app/shortcuts.rs) and the palette's (app/palette.rs).
        OpenSearch | ToggleDebug | PaletteUp | PaletteDown | PaletteOpen | PaletteReveal | PaletteClose => {
            return false
        }
        // Gestures, read by `wheel` and the input gate.
        WheelZoom | Pan => return false,
    }
    true
}

/// Scrolls the code of the open card under the pointer, if the pointer is on its code. Returns
/// whether it did.
fn scroll_open_card(state: &CanvasState, graph: &mut Graph, pointer: Pos2, scroll: f32) -> bool {
    let Some(node) = state.hover.hovered_node.and_then(|id| graph.nodes.get_mut(&id)) else { return false };
    if !node.is_code_expanded {
        return false;
    }
    let min = state.transform.world_to_screen(Pos2::new(node.position[0], node.position[1]));
    let max =
        state.transform.world_to_screen(Pos2::new(node.position[0] + node.size[0], node.position[1] + node.size[1]));
    let header = (60.0 * state.transform.zoom).max(34.0);
    if !Rect::from_min_max(Pos2::new(min.x, min.y + header), max).contains(pointer) {
        return false;
    }
    let line_count = node.source_code.as_deref().unwrap_or(&node.description).lines().count();
    let max_scroll = (line_count as f32 * 18.0 - (node.size[1] - 80.0)).max(0.0);
    node.scroll_offset_y = (node.scroll_offset_y - scroll * 0.8).clamp(0.0, max_scroll);
    true
}

/// What is under the pointer: a socket first (with its card), then the top-most card, then a wire.
fn hit_test(state: &CanvasState, graph: &Graph, pointer: Pos2) -> HoverState {
    let mut hover = HoverState::default();
    let zoom = state.transform.zoom;
    let pointer_world = state.transform.screen_to_world(pointer);
    let socket_radius = (12.0 * zoom).clamp(7.0, 24.0);
    let socket_world = socket_radius / zoom.max(0.1);
    let candidates =
        state.spatial_grid.query_rect(Rect::from_center_size(pointer_world, Vec2::splat(socket_world * 2.0 + 40.0)));
    let visible = |id| !graph.is_node_in_collapsed_cluster(id);

    for &node_id in candidates.iter().filter(|&&id| visible(id)) {
        let Some(node) = graph.nodes.get(&node_id) else { continue };
        let hit = node.inputs.iter().chain(&node.outputs).find(|port| {
            let member_port =
                node.member_nodes.iter().any(|m| m.in_port_id == Some(port.id) || m.out_port_id == Some(port.id));
            let hidden = member_port
                && (!node.is_dropdown_expanded || (!node.show_member_wires && !state.show_subnode_wires_globally));
            !hidden
                && port_world_position(node, port.id)
                    .is_some_and(|w| state.transform.world_to_screen(w).distance(pointer) <= socket_radius)
        });
        if let Some(port) = hit {
            hover.hovered_port = Some((node_id, port.id));
            hover.hovered_node = Some(node_id);
            return hover;
        }
    }

    hover.hovered_node = candidates.iter().rev().copied().filter(|&id| visible(id)).find(|id| {
        graph.nodes.get(id).is_some_and(|node| {
            let min = state.transform.world_to_screen(Pos2::new(node.position[0], node.position[1]));
            let max = state
                .transform
                .world_to_screen(Pos2::new(node.position[0] + node.size[0], node.position[1] + node.size[1]));
            Rect::from_min_max(min, max).contains(pointer)
        })
    });

    // Wires near the pointer, wherever their cards are.
    if state.show_wires && hover.hovered_node.is_none() {
        let reach = 7.0 / zoom.max(1e-4);
        hover.hovered_edge = state.scene.hit_test(pointer_world, reach, |kind| state.wire_kind_visible(kind));
    }
    hover
}

/// The innermost visible folder under a world point.
fn folder_at(graph: &Graph, world: Pos2) -> Option<String> {
    graph
        .clusters
        .iter()
        .filter(|c| !graph.hidden_cluster_ids.contains(&c.id) && c.size[0] > 1.0)
        .filter(|c| Rect::from_min_size(Pos2::from(c.position), Vec2::new(c.size[0], c.size[1])).contains(world))
        .max_by_key(|c| c.depth)
        .map(|c| c.id.clone())
}

#[cfg(test)]
mod tests {
    use egui::{Event, Modifiers, Pos2, Rect, Vec2};
    use studio_graph::{Graph, NodeArchetype};

    use super::super::shortcuts::{Chord, Mods, ShortcutAction, ShortcutOwner, SHORTCUTS};
    use super::super::{CanvasState, CanvasView};
    use crate::camera::{CameraTarget, Stop};

    /// The real canvas on a 1200×800 screen at the Code district, zoomed to 2, with one card
    /// selected; optionally under a focused text field.
    struct Canvas {
        ctx: egui::Context,
        time: f64,
        state: CanvasState,
        graph: Graph,
        field: Option<String>,
    }

    impl Canvas {
        fn new(field: bool) -> Self {
            let mut graph = Graph::new();
            let id = graph.add_node("a.rs", NodeArchetype::File, "", None, vec![], vec![], [0.0, 0.0]);
            let mut state = CanvasState::default();
            state.use_gpu_wires = false;
            state.jump_to(CameraTarget::Stop(Stop::Code));
            let mut c = Self { ctx: egui::Context::default(), time: 0.0, state, graph, field: field.then(String::new) };
            c.frames(vec![], 3);
            c.state.transform.zoom_at_pointer(Pos2::new(600.0, 400.0), 2.0);
            c.state.selected_nodes.insert(id);
            c
        }

        fn frames(&mut self, events: Vec<Event>, idle: usize) {
            for mut events in std::iter::once(events).chain((0..idle).map(|_| Vec::new())) {
                self.time += 1.0 / 60.0;
                if let Some(Event::Key { modifiers, .. }) = events.last() {
                    events.insert(0, Event::ModifiersChanged(*modifiers));
                }
                let raw = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0))),
                    time: Some(self.time),
                    events,
                    ..Default::default()
                };
                let (state, graph, field) = (&mut self.state, &mut self.graph, &mut self.field);
                let mut output = self.ctx.run_ui(raw, |ui| {
                    CanvasView::new(&mut *state, &mut *graph).show(ui);
                    if let Some(text) = field.as_mut() {
                        egui::Area::new(egui::Id::new("field")).fixed_pos(Pos2::new(10.0, 10.0)).show(ui.ctx(), |ui| {
                            ui.add(egui::TextEdit::singleline(text)).request_focus();
                        });
                    }
                });
                output.textures_delta.clear();
            }
        }

        /// Presses and releases `chord`.
        fn press(&mut self, chord: &Chord) {
            let Chord::Key(mods, key) = *chord else { panic!("a gesture has no key") };
            let modifiers = if let Mods::Logical(m) = mods { m } else { Modifiers::NONE };
            let event = |pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers };
            self.frames(vec![event(true)], 0);
            self.frames(vec![event(false)], 1);
        }

        /// What a canvas key can change.
        fn look(&self) -> (Vec2, f32, usize, bool) {
            let t = &self.state.transform;
            (t.pan, t.zoom, self.state.selected_nodes.len(), self.state.camera.is_flying())
        }
    }

    fn canvas_rows() -> impl Iterator<Item = (ShortcutAction, &'static Chord)> {
        SHORTCUTS
            .iter()
            .filter(|s| s.action.owner() == ShortcutOwner::Canvas && !s.display_only())
            .flat_map(|s| s.chords.iter().map(move |c| (s.action, c)))
    }

    /// E6 (a), canvas side: every canvas key in the table does what the sheet says.
    #[test]
    fn every_canvas_shortcut_is_dispatched() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0));
        for (action, chord) in canvas_rows() {
            assert!(super::canvas_key(&mut CanvasState::default(), action, rect), "{action:?} is handled");
            let mut c = Canvas::new(false);
            let (_, zoom, selected, _) = c.look();
            c.press(chord);
            let (_, after, now_selected, flying) = c.look();
            let did = match action {
                ShortcutAction::World => flying && c.state.stop == Stop::World,
                ShortcutAction::ZoomIn => (after - zoom * 1.25).abs() < 1e-4,
                ShortcutAction::ZoomOut => (after - zoom * 0.8).abs() < 1e-4,
                ShortcutAction::ActualSize => (after - 1.0).abs() < 1e-4,
                ShortcutAction::Escape => selected == 1 && now_selected == 0,
                other => panic!("{other:?} is not the canvas's"),
            };
            assert!(did, "{action:?} via {chord:?}");
        }
    }

    /// AM3: no canvas key in the table reaches the map while a text field has focus.
    #[test]
    fn canvas_shortcuts_are_ignored_while_a_text_field_has_focus() {
        let mut count = 0;
        for (action, chord) in canvas_rows() {
            let mut c = Canvas::new(true);
            let before = c.look();
            c.press(chord);
            c.frames(vec![], 60);
            assert_eq!(c.look(), before, "{action:?} via {chord:?} typed into a field");
            count += 1;
        }
        assert_eq!(count, 6, "Home, =, +, -, 0 and Escape");
    }

    #[test]
    fn hundred_percent_wins_over_the_zoom_keys() {
        let key =
            |key| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
        let mut c = Canvas::new(false);
        c.frames(vec![key(egui::Key::Equals), key(egui::Key::Num0), key(egui::Key::Minus)], 1);
        assert!((c.state.transform.zoom - 1.0).abs() < 1e-4);
        let mut c = Canvas::new(false);
        let zoom = c.state.transform.zoom;
        c.frames(vec![key(egui::Key::Minus), key(egui::Key::Equals)], 1);
        assert!((c.state.transform.zoom - zoom * 1.25).abs() < 1e-4, "zooming in wins, once");
    }
}
