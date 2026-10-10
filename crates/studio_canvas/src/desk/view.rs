//! Drawing the Desk: real egui widgets in a layer that moves and scales with the camera (the
//! pattern of egui's `Scene`), so text can be selected and copied and later edited. Below
//! [`SKELETON_ZOOM`] only outlines are painted: text that small cannot be read anyway.
//!
//! Layout is in the Desk's local units (points at 100%): the navigator on the left, the cards in
//! a row to its right, scrolling sideways when there are more than fit.

use std::path::PathBuf;

use egui::emath::TSTransform;
use egui::{
    pos2, vec2, Align, Align2, Color32, CornerRadius, FontId, Frame, Id, LayerId, Margin, Pos2, Rect, RichText,
    ScrollArea, Sense, Stroke, StrokeKind, Ui, UiBuilder,
};
use studio_graph::{Graph, NodeId};
use studio_ui::color_tokens::*;
use studio_ui::{file_extension_color, with_alpha, CodeView, MarkdownViewer};

use super::{code_lights, CardContent, DeskCard, DeskState, NavigatorRow, PlanBody, PlanDoc, PlanEdit};
use crate::camera::Stop;
use crate::transform::CanvasTransform;
use crate::world::WorldLayout;

/// Below this many points per local unit the Desk is painted as outlines.
pub const SKELETON_ZOOM: f32 = 0.45;
/// Id of the Desk's layer.
pub const DESK_LAYER: &str = "studio_desk";

const MARGIN: f32 = 36.0;
/// Room left at the top for the district's name and description.
const TOP: f32 = 112.0;
const NAV_WIDTH: f32 = 700.0;
const COLUMN_WIDTH: f32 = 228.0;
const CARD_WIDTH: f32 = 780.0;
const GAP: f32 = 24.0;

/// What happened on the Desk this frame, for the canvas to act on.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct DeskOutput {
    /// A file picked in the navigator: open it.
    pub open: Option<NodeId>,
    /// A card's "show on the map" button.
    pub show_on_map: Option<PathBuf>,
    /// An unloaded folder's Load row.
    pub load_folder: Option<String>,
    /// A link clicked in a document: the document, and the target as written.
    pub link: Option<(PathBuf, String)>,
    /// An edit picked on a plan card: open its file with these lines lit.
    pub open_lit: Option<(PathBuf, Vec<super::LitRange>)>,
}

fn screen_rect(transform: &CanvasTransform, world: Rect) -> Rect {
    Rect::from_min_max(transform.world_to_screen(world.min), transform.world_to_screen(world.max))
}

/// Draws the Desk for this frame. `ui` is the canvas's; `canvas` its rectangle on screen.
pub fn show_desk(
    ui: &mut Ui,
    desk: &mut DeskState,
    graph: &Graph,
    world: &WorldLayout,
    transform: &CanvasTransform,
    canvas: Rect,
    focus: Stop,
) -> DeskOutput {
    let mut out = DeskOutput::default();
    let on_screen = screen_rect(transform, world.desk);
    if !on_screen.intersects(canvas) {
        return out;
    }
    let local = transform.zoom * world.scale;
    let size = world.desk.size() / world.scale;
    if local < SKELETON_ZOOM {
        paint_skeleton(ui, desk, on_screen.min, local, size, canvas);
        return out;
    }

    // A layer just above the canvas, scaled and moved by the camera.
    let layer = LayerId::new(ui.layer_id().order, Id::new(DESK_LAYER));
    ui.ctx().set_sublayer(ui.layer_id(), layer);
    let to_global = TSTransform::new(on_screen.min.to_vec2(), local);
    ui.ctx().set_transform_layer(layer, to_global);
    let mut desk_ui = ui.new_child(UiBuilder::new().layer_id(layer).max_rect(Rect::from_min_size(Pos2::ZERO, size)));
    desk_ui.set_clip_rect(to_global.inverse() * canvas);

    let height = size.y - TOP - MARGIN;
    let nav_rect = Rect::from_min_size(pos2(MARGIN, TOP), vec2(NAV_WIDTH, height));
    desk_ui.scope_builder(UiBuilder::new().max_rect(nav_rect), |ui| navigator_panel(ui, desk, graph, &mut out));

    let cards_rect = Rect::from_min_max(pos2(MARGIN + NAV_WIDTH + GAP, TOP), pos2(size.x - MARGIN, size.y - MARGIN));
    desk_ui.scope_builder(UiBuilder::new().max_rect(cards_rect), |ui| {
        if desk.cards.is_empty() {
            ui.painter().text(
                cards_rect.left_center() + vec2(24.0, 0.0),
                Align2::LEFT_CENTER,
                "Double-click a file on the map, or pick one on the left, to open it here.",
                FontId::proportional(17.0),
                TEXT_DIM,
            );
            return;
        }
        cards_row(ui, desk, height, &mut out);
    });
    // Dimmed with the other districts while the camera is elsewhere (the canvas's veil is under
    // this layer).
    let target = if focus == Stop::World || focus == Stop::Desk { 0.0 } else { 1.0 };
    let dim = ui.ctx().animate_value_with_time(Id::new(("district_veil", Stop::Desk)), target, 0.3);
    if dim > 0.0 {
        let all = Rect::from_min_size(Pos2::ZERO, size);
        desk_ui.painter().rect_filled(all, CornerRadius::ZERO, with_alpha(CANVAS_BG, (dim * 150.0) as u8));
    }
    out
}

/// The open cards in a row, scrolling sideways; returns the row's size. A card outside the clip
/// only takes its room: its text is laid out when it comes into view, not all of it on the frame
/// the cards are filled.
fn cards_row(ui: &mut Ui, desk: &mut DeskState, height: f32, out: &mut DeskOutput) -> egui::Vec2 {
    let reveal = desk.reveal.take();
    let mut close = None;
    let row = ScrollArea::horizontal().id_salt("desk_cards").auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for card in &mut desk.cards {
                let size = vec2(CARD_WIDTH, height);
                let room = Rect::from_min_size(ui.cursor().min, size + card_frame().total_margin().sum());
                let response = if ui.is_rect_visible(room) {
                    ui.allocate_ui(size, |ui| card_ui(ui, card, height, out, &mut close)).response
                } else {
                    start_highlights(ui, card);
                    ui.allocate_exact_size(room.size(), Sense::hover()).1
                };
                if reveal == Some(card.id) {
                    response.scroll_to_me(Some(Align::Center));
                }
            }
        });
    });
    if let Some(id) = close {
        desk.close(id);
    }
    row.content_size
}

/// Starts colouring a card's code off screen, as its view would, so it shows coloured when it
/// comes into view.
fn start_highlights(ui: &Ui, card: &DeskCard) {
    let doc = match &card.content {
        CardContent::Code(doc) => doc,
        CardContent::Markdown(doc) if card.show_source => doc,
        _ => return,
    };
    let ctx = ui.ctx().clone();
    studio_ui::code_view::highlights_for(doc, move || ctx.request_repaint());
}

/// A card's frame, its stroke outside the card's width and height.
fn card_frame() -> Frame {
    Frame::new()
        .fill(CODE_EDITOR_BG)
        .stroke(Stroke::new(1.0, CARD_BORDER_NORMAL))
        .corner_radius(CornerRadius::from(14.0))
        .inner_margin(Margin::symmetric(0, 0))
}

/// One open file: its header and its contents.
fn card_ui(ui: &mut Ui, card: &mut DeskCard, height: f32, out: &mut DeskOutput, close: &mut Option<u64>) {
    let accent = if card.session.is_some() {
        DISTRICT_CHANGES
    } else {
        file_extension_color(card.path.extension().and_then(|e| e.to_str()).unwrap_or(""))
    };
    card_frame().show(ui, |ui| {
        ui.set_min_size(vec2(CARD_WIDTH, height));
        ui.set_max_size(vec2(CARD_WIDTH, height));
        ui.vertical(|ui| {
            Frame::new().inner_margin(Margin { left: 16, right: 10, top: 10, bottom: 8 }).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (dot, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                    ui.painter().circle_filled(dot.center(), 4.0, accent);
                    ui.label(RichText::new(card.title()).font(FontId::proportional(15.0)).color(TEXT_PRIMARY).strong());
                    ui.label(RichText::new(&card.rel).font(FontId::proportional(11.5)).color(TEXT_DIM));
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        if ui.button(egui_phosphor::regular::X).on_hover_text("Put it away").clicked() {
                            *close = Some(card.id);
                        }
                        if card.session.is_none() {
                            let map = ui.button(egui_phosphor::regular::CROSSHAIR).on_hover_text("Show it on the map");
                            if map.clicked() {
                                out.show_on_map = Some(card.path.clone());
                            }
                        }
                        if let CardContent::Markdown(_) = card.content {
                            let label = if card.show_source { "Rendered" } else { "Source" };
                            if ui.button(label).clicked() {
                                card.show_source = !card.show_source;
                            }
                        }
                    });
                });
            });
            ui.add(egui::Separator::default().spacing(0.0));
            let scroll_to = card.scroll_to.take();
            let id = Id::new(("desk_card", card.id));
            let lights = card.document().is_some().then(|| code_lights(card.lights()));
            let (lit, marks) = lights.unwrap_or_default();
            match &card.content {
                CardContent::Loading => {
                    ui.add_space(24.0);
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        ui.spinner();
                        ui.label(RichText::new("Reading…").color(TEXT_DIM));
                    });
                }
                CardContent::Code(doc) => {
                    CodeView::new(doc, id).lit(&lit).marks(&marks).scroll_to(scroll_to).show(ui);
                }
                CardContent::Markdown(doc) if card.show_source => {
                    CodeView::new(doc, id).lit(&lit).marks(&marks).scroll_to(scroll_to).show(ui);
                }
                CardContent::Markdown(doc) => {
                    // Lines are lit (by the chosen session): say so, one click from the source.
                    if !marks.is_empty() || !lit.is_empty() {
                        let text = format!("{} lines lit — view source", egui_phosphor::regular::CODE);
                        let note = ui.add(
                            egui::Button::new(RichText::new(text).color(DISTRICT_CHANGES).size(12.5)).frame(false),
                        );
                        if note.clicked() {
                            card.show_source = true;
                        }
                    }
                    ScrollArea::vertical().id_salt(id).auto_shrink([false, false]).show(ui, |ui| {
                        Frame::new().inner_margin(Margin::symmetric(22, 16)).show(ui, |ui| {
                            if let Some(target) = MarkdownViewer::new(&doc.text).font_size(14.0).show(ui) {
                                out.link = Some((card.path.clone(), target));
                            }
                        });
                    });
                }
                CardContent::Binary(bytes) => note(ui, &format!("Binary file · {}", studio_graph::human_bytes(*bytes))),
                CardContent::TooLarge(bytes) => {
                    note(ui, &format!("Too large to show here · {}", studio_graph::human_bytes(*bytes)))
                }
                CardContent::Unreadable(err) => note(ui, &format!("Could not read it: {err}")),
                CardContent::Plan(plan) => plan_ui(ui, plan, id, out),
            }
        });
    });
}

/// A plan card's body: who and where, the plan, its steps with their edits, the other edits.
fn plan_ui(ui: &mut Ui, plan: &PlanDoc, id: Id, out: &mut DeskOutput) {
    ScrollArea::vertical().id_salt(id).auto_shrink([false, false]).show(ui, |ui| {
        Frame::new().inner_margin(Margin::symmetric(20, 14)).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            let dim = |t: &str| RichText::new(t).font(FontId::proportional(12.5)).color(TEXT_DIM);
            ui.horizontal_wrapped(|ui| {
                ui.label(dim(&format!("started {}", plan.started)));
                if !plan.branches.is_empty() {
                    ui.label(dim(&format!("· {} {}", egui_phosphor::regular::GIT_BRANCH, plan.branches.join(", "))));
                }
                ui.label(dim(&format!("· {}", plan.worktree)));
            });
            ui.label(RichText::new("Observed · session log").font(FontId::proportional(12.0)).color(DISTRICT_CHANGES));
            ui.add_space(4.0);

            if let Some(note) = plan.plan.note() {
                ui.label(RichText::new(note).font(FontId::proportional(13.0)).color(DISTRICT_PIPELINE));
            }
            if let PlanBody::Text { doc, path, .. } = &plan.plan {
                let from = path.clone().unwrap_or_default();
                if let Some(target) = MarkdownViewer::new(&doc.text).font_size(14.0).show(ui) {
                    out.link = Some((from, target));
                }
            }

            ui.add_space(8.0);
            heading(ui, "Steps");
            if plan.steps.is_empty() {
                ui.label(dim("The session created no tasks."));
            }
            for (n, step) in plan.steps.iter().enumerate() {
                ui.horizontal(|ui| {
                    let (badge, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                    ui.painter().circle_filled(badge.center(), 11.0, with_alpha(DISTRICT_CHANGES, 200));
                    ui.painter().text(
                        badge.center(),
                        Align2::CENTER_CENTER,
                        (n + 1).to_string(),
                        FontId::proportional(11.5),
                        TEXT_PRIMARY,
                    );
                    let subject = if step.subject.is_empty() { "(subject not in the log)" } else { &step.subject };
                    ui.label(RichText::new(subject).font(FontId::proportional(13.5)).color(TEXT_PRIMARY));
                    ui.label(dim(&step.state));
                });
                for edit in &step.edits {
                    edit_row(ui, edit, &plan.slug, 30.0, out);
                }
            }

            ui.add_space(8.0);
            heading(ui, "Edits");
            let reads = match plan.reads {
                0 => String::new(),
                1 => " · 1 file read".to_string(),
                n => format!(" · {n} files read"),
            };
            if plan.edits.is_empty() {
                ui.label(dim(&format!("No edits outside the steps{reads}.")));
            } else {
                ui.label(dim(&format!("Made outside any step, in order{reads}.")));
            }
            for edit in &plan.edits {
                edit_row(ui, edit, &plan.slug, 0.0, out);
            }
        });
    });
}

fn heading(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(FontId::proportional(14.0)).color(TEXT_PRIMARY).strong());
}

/// One edit: its file, kind, time and where it is now. Clicking opens the file with it lit.
fn edit_row(ui: &mut Ui, edit: &PlanEdit, slug: &str, indent: f32, out: &mut DeskOutput) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
    let opens = edit.matched.opens();
    if opens && response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::from(5.0), FLOATING_BTN_HOVER);
    }
    let painter = ui.painter();
    let left = rect.left_center() + vec2(indent + 6.0, 0.0);
    let right = format!("{} · {} · {}", edit.kind, edit.time, edit.matched.label());
    let right_width = right.chars().count() as f32 * 6.4;
    let max_chars = ((rect.width() - indent - right_width - 24.0) / 7.0).max(8.0) as usize;
    let color = if opens { TEXT_SECONDARY } else { TEXT_DIM };
    let file = studio_ui::truncate_with_ellipsis(&edit.rel, max_chars);
    painter.text(left, Align2::LEFT_CENTER, file, FontId::monospace(12.0), color);
    let tone = match edit.matched {
        super::EditMatch::Lit(_) => DISTRICT_CHANGES,
        super::EditMatch::Stale(_) => DISTRICT_PIPELINE,
        _ => TEXT_DIM,
    };
    painter.text(rect.right_center() - vec2(6.0, 0.0), Align2::RIGHT_CENTER, right, FontId::proportional(11.5), tone);
    if opens && response.clicked() {
        out.open_lit = Some((edit.path.clone(), edit.lights(slug)));
    }
}

fn note(ui: &mut Ui, text: &str) {
    ui.add_space(24.0);
    ui.horizontal(|ui| {
        ui.add_space(16.0);
        ui.label(RichText::new(text).font(FontId::proportional(14.0)).color(TEXT_DIM));
    });
}

/// The navigator: Finder-style columns from the project's parts down to a file.
fn navigator_panel(ui: &mut Ui, desk: &mut DeskState, graph: &Graph, out: &mut DeskOutput) {
    let columns = desk.navigator.columns(graph);
    let mut picked_folder = None;
    Frame::new()
        .fill(PANEL_BG)
        .stroke(Stroke::new(1.0, PANEL_BORDER))
        .corner_radius(CornerRadius::from(14.0))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            ui.horizontal(|ui| {
                ui.label(RichText::new("Navigator").font(FontId::proportional(14.0)).color(TEXT_PRIMARY).strong());
                for column in columns.iter().skip(1) {
                    ui.label(RichText::new(egui_phosphor::regular::CARET_RIGHT).color(TEXT_DIM));
                    ui.label(RichText::new(&column.title).color(TEXT_SECONDARY));
                }
            });
            ui.add_space(6.0);
            ScrollArea::horizontal().id_salt("desk_navigator").auto_shrink([false, false]).show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    for (i, column) in columns.iter().enumerate() {
                        ui.allocate_ui(vec2(COLUMN_WIDTH, ui.available_height()), |ui| {
                            ScrollArea::vertical().id_salt(("desk_nav_column", i)).auto_shrink([false, false]).show(
                                ui,
                                |ui| {
                                    ui.set_width(COLUMN_WIDTH - 12.0);
                                    for row in &column.rows {
                                        navigator_row(ui, i, row, &mut picked_folder, out);
                                    }
                                },
                            );
                        });
                        ui.separator();
                    }
                });
            });
        });
    if let Some((column, id)) = picked_folder {
        desk.navigator.pick_folder(column, id);
    }
    if let Some(file) = out.open {
        desk.navigator.file = Some(file);
    }
}

fn navigator_row(
    ui: &mut Ui,
    column: usize,
    row: &NavigatorRow,
    picked_folder: &mut Option<(usize, String)>,
    out: &mut DeskOutput,
) {
    let (text, right, picked, color) = match row {
        NavigatorRow::Folder { label, items, unloaded, picked, .. } => {
            let right = match unloaded {
                Some(Some(files)) => format!("load {files}"),
                Some(None) => "load".to_string(),
                None => format!("{items} {}", egui_phosphor::regular::CARET_RIGHT),
            };
            (format!("{} {label}", egui_phosphor::regular::FOLDER), right, *picked, TEXT_PRIMARY)
        }
        NavigatorRow::File { label, badge, picked, .. } => {
            let color = file_extension_color(badge.as_deref().unwrap_or("").to_lowercase().as_str());
            (format!("{} {label}", egui_phosphor::regular::FILE), String::new(), *picked, with_alpha(color, 255))
        }
    };
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
    let fill = if picked {
        with_alpha(CARD_BORDER_SELECTED, 70)
    } else if response.hovered() {
        FLOATING_BTN_HOVER
    } else {
        Color32::TRANSPARENT
    };
    let painter = ui.painter();
    painter.rect(rect, CornerRadius::from(6.0), fill, Stroke::NONE, StrokeKind::Inside);
    let max_chars = ((rect.width() - 54.0) / 7.0).max(6.0) as usize;
    let text = studio_ui::truncate_with_ellipsis(&text, max_chars);
    painter.text(rect.left_center() + vec2(8.0, 0.0), Align2::LEFT_CENTER, text, FontId::proportional(13.0), color);
    painter.text(
        rect.right_center() - vec2(8.0, 0.0),
        Align2::RIGHT_CENTER,
        right,
        FontId::proportional(11.0),
        TEXT_DIM,
    );
    if response.clicked() {
        match row {
            NavigatorRow::Folder { id, unloaded: Some(_), .. } => out.load_folder = Some(id.clone()),
            NavigatorRow::Folder { id, .. } => *picked_folder = Some((column, id.clone())),
            NavigatorRow::File { id, .. } => out.open = Some(*id),
        }
    }
}

/// From far away: the navigator and each card as an outline with its name.
fn paint_skeleton(ui: &Ui, desk: &DeskState, origin: Pos2, local: f32, size: egui::Vec2, canvas: Rect) {
    let painter = ui.painter().with_clip_rect(canvas);
    let to_screen = |r: Rect| Rect::from_min_max(origin + r.min.to_vec2() * local, origin + r.max.to_vec2() * local);
    let height = size.y - TOP - MARGIN;
    let radius = CornerRadius::from((14.0 * local).clamp(2.0, 14.0));
    let outline = |r: Rect, title: &str| {
        let r = to_screen(r);
        painter.rect(r, radius, with_alpha(PANEL_BG, 220), Stroke::new(1.0, PANEL_BORDER), StrokeKind::Inside);
        let font = 15.0 * local;
        if font >= 6.0 {
            painter.text(
                r.min + vec2(16.0, 14.0) * local,
                Align2::LEFT_TOP,
                title,
                FontId::proportional(font),
                TEXT_SECONDARY,
            );
        }
    };
    outline(Rect::from_min_size(pos2(MARGIN, TOP), vec2(NAV_WIDTH, height)), "Navigator");
    for (i, card) in desk.cards.iter().enumerate() {
        let x = MARGIN + NAV_WIDTH + GAP + i as f32 * (CARD_WIDTH + GAP);
        if x + CARD_WIDTH > size.x - MARGIN {
            break;
        }
        outline(Rect::from_min_size(pos2(x, TOP), vec2(CARD_WIDTH, height)), &card.title());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Three code cards, each to scroll to line 5, drawn once in a window `width` wide.
    fn draw(width: f32) -> (DeskState, egui::Vec2) {
        let mut desk = DeskState::default();
        for name in ["a.rs", "b.rs", "c.rs"] {
            let path = Path::new("/p").join(name);
            let id = desk.open(&path, Some(5));
            desk.fill(id, super::super::content_for_text(&path, "fn main() {}\n".repeat(40).into()));
        }
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 600.0))),
            ..Default::default()
        };
        let mut size = egui::Vec2::ZERO;
        let mut output = ctx.run_ui(raw, |ui| {
            size = cards_row(ui, &mut desk, 500.0, &mut DeskOutput::default());
        });
        output.textures_delta.clear();
        (desk, size)
    }

    #[test]
    fn cards_out_of_view_take_their_room_but_are_not_drawn() {
        let (narrow, narrow_size) = draw(1000.0);
        let scrolled: Vec<_> = narrow.cards.iter().map(|c| c.scroll_to).collect();
        assert_eq!(scrolled, [None, None, Some(5)], "the third card is not drawn yet");
        let (wide, wide_size) = draw(4000.0);
        assert!(wide.cards.iter().all(|c| c.scroll_to.is_none()), "all three drawn");
        assert_eq!(narrow_size, wide_size, "a card's room is the same drawn or not");
    }
}
