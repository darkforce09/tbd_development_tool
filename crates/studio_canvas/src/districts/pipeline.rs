//! The Pipeline district, north of the map: the flows a run takes, one lane per entry point
//! (a route, a program's `main`, a command), its steps left to right by how far they are from the
//! entry, and the links between them in the line style of their evidence tier.
//!
//! The district is as tall as what it shows: the world grows it north, so the map never moves.

use std::collections::BTreeSet;
use std::path::PathBuf;

use egui::{pos2, vec2, Align2, Color32, CornerRadius, FontId, Painter, Pos2, Rect, Stroke, StrokeKind};
use studio_graph::EvidenceTier;
use studio_ui::color_tokens::*;
use studio_ui::{truncate_with_ellipsis, with_alpha};

use crate::transform::CanvasTransform;
use crate::view::gate::InputGate;
use crate::view::render_wires::{wire_shapes, WirePath};
use crate::world::WorldLayout;

/// A step of a flow, as its card shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineStep {
    pub title: String,
    /// One line under the title, e.g. a contract pointer.
    pub detail: String,
    /// "Rust", "Enforce", "JSON", ...
    pub language: String,
    /// Project-relative, as the source read it.
    pub path: PathBuf,
    /// 1-based.
    pub line: usize,
}

/// Where a step sits in its lane: the layer (distance from the entry) and its place in the layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StepSlot {
    pub layer: usize,
    pub index: usize,
}

/// A link between two steps of a lane.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineLink {
    pub from: StepSlot,
    pub to: StepSlot,
    pub tier: EvidenceTier,
    /// For a possible set: how many candidates there were.
    pub candidates: u16,
    /// The condition the link only holds under, e.g. a route registered inside `if dev`.
    pub conditional: Option<String>,
    /// The link goes from one language to another.
    pub crossing: bool,
    /// What the chip says besides the tier, e.g. the contract a crossing goes through.
    pub label: String,
}

/// One flow: an entry point and the steps it reaches.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineLane {
    pub name: String,
    pub crossing: bool,
    pub layers: Vec<Vec<PipelineStep>>,
    pub links: Vec<PipelineLink>,
    /// Steps the source left out beyond its cap.
    pub truncated: usize,
}

/// Flows that belong together, e.g. the endpoints of one router file.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineGroup {
    /// Stable across launches; what [`CanvasState::pipeline_open`](crate::CanvasState) holds.
    pub key: String,
    pub title: String,
    /// How many flows it holds.
    pub count: usize,
    /// Open unless the user toggled it.
    pub expanded: bool,
    pub lanes: Vec<PipelineLane>,
}

/// What the Pipeline district shows, in its final order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PipelineView {
    pub groups: Vec<PipelineGroup>,
}

impl PipelineView {
    /// No flow at all: the district says so plainly.
    pub fn is_empty(&self) -> bool {
        self.groups.iter().all(|g| g.lanes.is_empty())
    }
}

/// Whether a group is open: its default, flipped when the user toggled it.
pub fn is_expanded(group: &PipelineGroup, open: &BTreeSet<String>) -> bool {
    group.expanded != open.contains(&group.key)
}

/// What a click in the district asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum PipelineAction {
    /// Open this (project-relative) file at this line on the Desk.
    Open(PathBuf, usize),
    /// Open or close the group with this key.
    Toggle(String),
    /// Bring the group at this index into view.
    Reveal(usize),
}

pub const FOOTNOTE: &str =
    "Built from route tables, @route and @contract tags and resolved calls. Click a step to open its code.";
pub const EMPTY: &str = "No entry points found — no route tables, no @route tags, no programs.";

const MARGIN: f32 = 36.0;
const FOOTNOTE_Y: f32 = 104.0;
const PILLS_Y: f32 = 136.0;
const PILL_H: f32 = 30.0;
const PILL_GAP: f32 = 10.0;
const GROUP_HEADER: f32 = 48.0;
const GROUP_GAP: f32 = 24.0;
const LANE_PAD: f32 = 18.0;
const LANE_GAP: f32 = 14.0;
const LANE_MIN: f32 = 132.0;
const NAME_W: f32 = 220.0;
const CARD: [f32; 2] = [220.0, 96.0];
const GAP_X: f32 = 70.0;
const GAP_Y: f32 = 40.0;
const MORE_H: f32 = 44.0;
/// Cards stacked in one layer before a "+k more" card.
pub const MAX_STACK: usize = 3;
/// Layers shown in a lane before a "+k more" card.
pub const MAX_LAYERS: usize = 7;

/// A lane's cards, in the district's units.
#[derive(Debug, Clone, PartialEq)]
pub struct LaneLayout {
    pub rect: Rect,
    pub cards: Vec<(StepSlot, Rect)>,
    /// "+k more" cards: where, and how many steps they stand for.
    pub more: Vec<(Rect, usize)>,
}

impl LaneLayout {
    fn card(&self, slot: StepSlot) -> Option<Rect> {
        self.cards.iter().find(|(s, _)| *s == slot).map(|(_, r)| *r)
    }
}

/// A group: its header row, and its lanes when it is open.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupLayout {
    pub header: Rect,
    /// Header and lanes.
    pub rect: Rect,
    pub expanded: bool,
    pub lanes: Vec<LaneLayout>,
}

/// Where everything sits, in the district's units.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineLayout {
    /// One pill per group, at the same index.
    pub pills: Vec<Rect>,
    pub groups: Vec<GroupLayout>,
    /// How tall the content is, for the world's [`DistrictExtents`](crate::world::DistrictExtents).
    pub height: f32,
}

fn pill_text(group: &PipelineGroup) -> String {
    format!("{} · {}", truncate_with_ellipsis(&group.title, 34), group.count)
}

fn pill_width(group: &PipelineGroup) -> f32 {
    pill_text(group).chars().count() as f32 * 7.2 + 28.0
}

#[cfg(test)]
thread_local! {
    /// How many layouts this thread worked out, so tests can tell a cached layout from a new one.
    pub static LAYOUTS_BUILT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Lays the district out `width` local units wide: the footnote and a pill per group, then each
/// group's header and, when it is open, one lane per flow. The canvas keeps the result until the
/// view, the open groups or the width change.
pub fn layout(view: &PipelineView, width: f32, open: &BTreeSet<String>) -> PipelineLayout {
    #[cfg(test)]
    LAYOUTS_BUILT.with(|n| n.set(n.get() + 1));
    let right = width - MARGIN;
    let inner = right - MARGIN;
    let mut pills = Vec::with_capacity(view.groups.len());
    let (mut x, mut y) = (MARGIN, PILLS_Y);
    for group in &view.groups {
        let w = pill_width(group).min(inner);
        if x + w > right && x > MARGIN {
            x = MARGIN;
            y += PILL_H + PILL_GAP;
        }
        pills.push(Rect::from_min_size(pos2(x, y), vec2(w, PILL_H)));
        x += w + PILL_GAP;
    }
    let mut y = y + PILL_H + GROUP_GAP;

    let mut groups = Vec::with_capacity(view.groups.len());
    for group in &view.groups {
        let top = y;
        let header = Rect::from_min_size(pos2(MARGIN, y), vec2(inner, GROUP_HEADER));
        y += GROUP_HEADER;
        let expanded = is_expanded(group, open);
        let mut lanes = Vec::new();
        if expanded {
            for lane in &group.lanes {
                let l = lane_layout(lane, y, inner);
                y = l.rect.bottom() + LANE_GAP;
                lanes.push(l);
            }
        }
        groups.push(GroupLayout {
            header,
            rect: Rect::from_min_max(pos2(MARGIN, top), pos2(right, y)),
            expanded,
            lanes,
        });
        y += GROUP_GAP;
    }
    PipelineLayout { pills, groups, height: y + MARGIN }
}

fn lane_layout(lane: &PipelineLane, top: f32, inner: f32) -> LaneLayout {
    let cards_x = MARGIN + LANE_PAD + NAME_W + 24.0;
    let mut cards = Vec::new();
    let mut more = Vec::new();
    let mut tallest: f32 = 0.0;
    for (li, layer) in lane.layers.iter().take(MAX_LAYERS).enumerate() {
        let x = cards_x + li as f32 * (CARD[0] + GAP_X);
        let mut y = top + LANE_PAD;
        for index in 0..layer.len().min(MAX_STACK) {
            cards.push((StepSlot { layer: li, index }, Rect::from_min_size(pos2(x, y), vec2(CARD[0], CARD[1]))));
            y += CARD[1] + GAP_Y;
        }
        if layer.len() > MAX_STACK {
            more.push((Rect::from_min_size(pos2(x, y), vec2(CARD[0], MORE_H)), layer.len() - MAX_STACK));
            y += MORE_H + GAP_Y;
        }
        tallest = tallest.max(y - GAP_Y - top - LANE_PAD);
    }
    let hidden = lane.layers.iter().skip(MAX_LAYERS).map(Vec::len).sum::<usize>() + lane.truncated;
    if hidden > 0 {
        let x = cards_x + lane.layers.len().min(MAX_LAYERS) as f32 * (CARD[0] + GAP_X);
        more.push((Rect::from_min_size(pos2(x, top + LANE_PAD), vec2(CARD[0], MORE_H)), hidden));
        tallest = tallest.max(MORE_H);
    }
    let height = (tallest + LANE_PAD * 2.0).max(LANE_MIN);
    LaneLayout { rect: Rect::from_min_size(pos2(MARGIN, top), vec2(inner, height)), cards, more }
}

/// A group's place in the world, header and lanes, for the camera.
pub fn group_world_rect(world: &WorldLayout, layout: &PipelineLayout, group: usize) -> Option<Rect> {
    let local = layout.groups.get(group)?.rect;
    Some(Rect::from_min_size(world.pipeline.min + local.min.to_vec2() * world.scale, local.size() * world.scale))
}

/// What a view of the group at 100% covers: a screen's worth of the world from its top left corner.
pub fn reveal_rect(world: &WorldLayout, group: Rect, screen: Rect) -> Rect {
    Rect::from_min_size(group.min - vec2(24.0, 24.0) * world.scale, screen.size() * world.scale)
}

/// The two-letter mark and colour of a language on a step card.
pub fn language_mark(language: &str) -> (String, Color32) {
    match language {
        "Rust" => ("Rs".into(), LANG_RUST),
        "Enforce" => ("En".into(), LANG_ENFORCE),
        "JSON" => ("{}".into(), LANG_JSON),
        "TypeScript" => ("Ts".into(), FILE_TS),
        "JavaScript" => ("Js".into(), FILE_JS),
        "Python" => ("Py".into(), FILE_PY),
        "Go" => ("Go".into(), FILE_GO),
        other => (other.chars().take(2).collect(), TEXT_SECONDARY),
    }
}

/// What a link's chip says about its evidence: every tier by name, "1 of n" for one of a set.
pub fn tier_text(tier: EvidenceTier, candidates: u16) -> String {
    match tier {
        EvidenceTier::PossibleSet => format!("1 of {}", candidates.max(1)),
        tier => tier.label().to_string(),
    }
}

fn tier_color(tier: EvidenceTier) -> Color32 {
    match tier {
        EvidenceTier::Proven => TIER_PROVEN,
        EvidenceTier::PossibleSet => TIER_POSSIBLE,
        EvidenceTier::Observed => TIER_OBSERVED,
        EvidenceTier::Unresolved => TIER_UNRESOLVED,
    }
}

/// Everything a link's chips say, in one line.
fn link_detail(link: &PipelineLink) -> String {
    let mut parts = vec![tier_text(link.tier, link.candidates)];
    if let Some(c) = &link.conditional {
        parts.push(format!("conditional: {c}"));
    }
    if !link.label.is_empty() {
        parts.push(link.label.clone());
    } else if link.crossing {
        parts.push("crosses languages".into());
    }
    parts.join(" · ")
}

/// The path of a link between two cards, in local units, and where its chips go.
fn link_path(link: &PipelineLink, a: Rect, b: Rect) -> (WirePath, Pos2, bool) {
    let curve = |p0: Pos2, c1: Pos2, c2: Pos2, p3: Pos2| {
        (WirePath::Curve([p0, c1, c2, p3]), crate::wire::eval_cubic_bezier(p0, c1, c2, p3, 0.5), false)
    };
    if link.to.layer > link.from.layer {
        let (p0, p3) = (a.right_center(), b.left_center());
        let dx = (p3.x - p0.x) * 0.5;
        curve(p0, p0 + vec2(dx, 0.0), p3 - vec2(dx, 0.0), p3)
    } else if link.to.layer == link.from.layer {
        let adjacent = link.from.index.abs_diff(link.to.index) == 1;
        let (p0, p3) = if link.from.index < link.to.index {
            (a.center_bottom(), b.center_top())
        } else {
            (a.center_top(), b.center_bottom())
        };
        if adjacent {
            (WirePath::Segment([p0, p3]), p0.lerp(p3, 0.5), true)
        } else {
            let (p0, p3) = (a.left_center(), b.left_center());
            curve(p0, p0 - vec2(30.0, 0.0), p3 - vec2(30.0, 0.0), p3)
        }
    } else {
        let (p0, p3) = (a.center_bottom(), b.center_bottom());
        curve(p0, p0 + vec2(0.0, GAP_Y * 0.8), p3 + vec2(0.0, GAP_Y * 0.8), p3)
    }
}

/// A chip centred on `at`: rounded, on the panel colour, with its accent as border and text.
fn chip(
    painter: &Painter,
    at: Pos2,
    text: &str,
    font: FontId,
    accent: Color32,
    text_color: Color32,
    local: f32,
) -> Rect {
    let galley = painter.layout_no_wrap(text.to_string(), font, text_color);
    let pad = vec2(8.0, 3.0) * local;
    let r = Rect::from_center_size(at, galley.size() + pad * 2.0);
    painter.rect(
        r,
        CornerRadius::from((10.0 * local).clamp(2.0, 14.0)),
        PANEL_BG,
        Stroke::new(1.0, with_alpha(accent, 150)),
        StrokeKind::Inside,
    );
    painter.galley(r.min + pad, galley, text_color);
    r
}

/// Paints the district, laid out as `layout`, and returns what a click on it asks for. Clicks
/// come through `gate.click()`, as in the other districts.
#[allow(clippy::too_many_arguments)]
pub fn paint_pipeline(
    painter: &Painter,
    world: &WorldLayout,
    transform: &CanvasTransform,
    screen: Rect,
    view: &PipelineView,
    layout: &PipelineLayout,
    open: &BTreeSet<String>,
    gate: &InputGate,
) -> Option<PipelineAction> {
    let local = transform.zoom * world.scale;
    let origin = transform.world_to_screen(world.pipeline.min);
    let district = Rect::from_min_max(origin, transform.world_to_screen(world.pipeline.max));
    if !district.intersects(screen) || local < 0.08 {
        return None;
    }
    let clip = district.intersect(screen);
    let painter = painter.with_clip_rect(clip);
    let at = |r: Rect| Rect::from_min_max(origin + r.min.to_vec2() * local, origin + r.max.to_vec2() * local);
    let point = |p: Pos2| origin + p.to_vec2() * local;
    let font = |pt: f32| FontId::proportional((pt * local).min(140.0));
    let mono = |pt: f32| FontId::monospace((pt * local).min(140.0));
    let text_ok = local >= crate::world::paint::FAR_LOCAL_ZOOM;
    let radius = |r: f32| CornerRadius::from((r * local).clamp(1.0, 24.0));
    let accent = DISTRICT_PIPELINE;
    let click = gate.click();
    let mut action = None;

    if view.is_empty() {
        if text_ok {
            painter.text(district.center(), Align2::CENTER_CENTER, EMPTY, font(17.0), TEXT_SECONDARY);
        }
        return None;
    }

    if text_ok {
        painter.text(point(pos2(MARGIN, FOOTNOTE_Y)), Align2::LEFT_TOP, FOOTNOTE, font(13.0), TEXT_DIM);
    }

    // The pills: one per group, the open ones in the district's colour.
    for (i, (group, rect)) in view.groups.iter().zip(&layout.pills).enumerate() {
        let r = at(*rect);
        if !r.intersects(screen) {
            continue;
        }
        let expanded = is_expanded(group, open);
        let hovered = gate.hovers(r);
        let (fill, color) = match (expanded, hovered) {
            (true, _) => (with_alpha(accent, if hovered { 60 } else { 40 }), accent),
            (false, true) => (FLOATING_BTN_HOVER, TEXT_PRIMARY),
            (false, false) => (BADGE_BG, TEXT_SECONDARY),
        };
        painter.rect(r, radius(15.0), fill, Stroke::NONE, StrokeKind::Inside);
        if text_ok {
            painter.text(r.center(), Align2::CENTER_CENTER, pill_text(group), font(13.0), color);
        }
        if click.is_some_and(|p| r.contains(p)) {
            action = Some(PipelineAction::Reveal(i));
        }
    }

    let mut hover_detail: Option<(Pos2, String, bool)> = None;
    for (group, g) in view.groups.iter().zip(&layout.groups) {
        if !at(g.rect).intersects(screen) {
            continue;
        }
        // The header: a chevron, the name and how many flows.
        let header = at(g.header);
        if header.intersects(screen) {
            if gate.hovers(header) {
                painter.rect_filled(header, radius(10.0), FLOATING_BTN_HOVER);
            }
            if text_ok {
                use egui_phosphor::regular as icon;
                let chevron = if g.expanded { icon::CARET_DOWN } else { icon::CARET_RIGHT };
                let mid = header.left_center();
                painter.text(mid + vec2(10.0 * local, 0.0), Align2::LEFT_CENTER, chevron, font(18.0), TEXT_SECONDARY);
                let title = painter.text(
                    mid + vec2(40.0 * local, 0.0),
                    Align2::LEFT_CENTER,
                    truncate_with_ellipsis(&group.title, 80),
                    font(18.0),
                    TEXT_PRIMARY,
                );
                let count = if group.count == 1 { "1 flow".to_string() } else { format!("{} flows", group.count) };
                painter.text(
                    title.right_center() + vec2(14.0 * local, 0.0),
                    Align2::LEFT_CENTER,
                    count,
                    font(13.0),
                    TEXT_DIM,
                );
            }
            if click.is_some_and(|p| header.contains(p)) {
                action = Some(PipelineAction::Toggle(group.key.clone()));
            }
        }

        for (lane, l) in group.lanes.iter().zip(&g.lanes) {
            let lane_rect = at(l.rect);
            if !lane_rect.intersects(screen) {
                continue;
            }
            let tint = if lane.crossing { LANG_JSON } else { accent };
            painter.rect(
                lane_rect,
                radius(14.0),
                with_alpha(tint, 8),
                Stroke::new(1.0, with_alpha(tint, 45)),
                StrokeKind::Inside,
            );
            if text_ok {
                let name_at = lane_rect.min + vec2(LANE_PAD, LANE_PAD) * local;
                let galley = painter.layout(
                    truncate_with_ellipsis(&lane.name, 90),
                    font(15.0),
                    if lane.crossing { CONTRACT_CHIP_TEXT } else { TEXT_PRIMARY },
                    NAME_W * local,
                );
                let below = name_at.y + galley.size().y + 4.0 * local;
                painter.galley(name_at, galley, TEXT_PRIMARY);
                let steps: usize = lane.layers.iter().map(Vec::len).sum::<usize>() + lane.truncated;
                let sub = match (lane.crossing, steps) {
                    (true, n) => format!("crosses languages · {n} steps"),
                    (false, 1) => "1 step".to_string(),
                    (false, n) => format!("{n} steps"),
                };
                painter.text(pos2(name_at.x, below), Align2::LEFT_TOP, sub, font(12.5), TEXT_DIM);
            }

            // Links under the cards.
            let stroke_w = (2.0 * local).clamp(1.0, 3.0);
            let mut shapes = Vec::new();
            let mut chips = Vec::new();
            for link in &lane.links {
                let (Some(a), Some(b)) = (l.card(link.from), l.card(link.to)) else { continue };
                let (path, mid, vertical) = link_path(link, a, b);
                let path = match path {
                    WirePath::Segment([p, q]) => WirePath::Segment([point(p), point(q)]),
                    WirePath::Curve(ps) => WirePath::Curve(ps.map(point)),
                };
                let base = if link.crossing { LANG_JSON } else { TEXT_SECONDARY };
                let alpha = if link.tier == EvidenceTier::Unresolved { 130 } else { 220 };
                wire_shapes(path, link.tier, Stroke::new(stroke_w, with_alpha(base, alpha)), clip, &mut shapes);
                let end = match path {
                    WirePath::Segment([_, q]) => q,
                    WirePath::Curve([.., q]) => q,
                };
                shapes.push(egui::Shape::circle_filled(end, (3.5 * local).clamp(1.0, 5.0), with_alpha(base, alpha)));
                chips.push((link, point(mid), vertical));
            }
            painter.extend(shapes);

            // The cards: language mark and title, where it is, one line of detail.
            for (slot, rect) in &l.cards {
                let r = at(*rect);
                if !r.intersects(screen) {
                    continue;
                }
                let Some(step) = lane.layers.get(slot.layer).and_then(|layer| layer.get(slot.index)) else { continue };
                let hovered = gate.hovers(r);
                let (mark, color) = language_mark(&step.language);
                painter.rect(
                    r,
                    radius(12.0),
                    if hovered { CARD_BG_HOVER } else { CARD_BG },
                    Stroke::new(1.5, if hovered { with_alpha(color, 200) } else { CARD_BORDER_NORMAL }),
                    StrokeKind::Inside,
                );
                if text_ok {
                    let x = r.min.x + 12.0 * local;
                    painter.text(pos2(x, r.min.y + 14.0 * local), Align2::LEFT_TOP, mark, mono(10.5), color);
                    painter.text(
                        pos2(x + 24.0 * local, r.min.y + 10.0 * local),
                        Align2::LEFT_TOP,
                        truncate_with_ellipsis(&step.title, 22),
                        font(15.0),
                        TEXT_PRIMARY,
                    );
                    let file =
                        step.path.file_name().map_or_else(|| step.path.to_string_lossy(), |f| f.to_string_lossy());
                    let where_ = format!("{file}:{}", step.line);
                    painter.text(
                        pos2(x, r.min.y + 38.0 * local),
                        Align2::LEFT_TOP,
                        truncate_with_ellipsis(&where_, 28),
                        mono(12.0),
                        TEXT_SECONDARY,
                    );
                    if !step.detail.is_empty() && step.detail != where_ {
                        painter.text(
                            pos2(x, r.min.y + 62.0 * local),
                            Align2::LEFT_TOP,
                            truncate_with_ellipsis(&step.detail, 30),
                            font(12.5),
                            TEXT_DIM,
                        );
                    }
                }
                if click.is_some_and(|p| r.contains(p)) {
                    action = Some(PipelineAction::Open(step.path.clone(), step.line));
                }
            }
            for (rect, count) in &l.more {
                let r = at(*rect);
                if !r.intersects(screen) {
                    continue;
                }
                painter.rect(r, radius(12.0), PANEL_BG, Stroke::new(1.0, PANEL_BORDER), StrokeKind::Inside);
                if text_ok {
                    painter.text(r.center(), Align2::CENTER_CENTER, format!("+{count} more"), font(13.0), TEXT_DIM);
                }
            }

            // The chips over everything in the lane: the tier always, "conditional" when the link
            // only holds under a condition, and the contract a crossing goes through.
            if text_ok {
                for (link, mid, vertical) in chips {
                    if !clip.expand(40.0).contains(mid) {
                        continue;
                    }
                    let detail = link_detail(link);
                    let shown = if vertical {
                        let (accent, text) = if link.crossing {
                            (LANG_JSON, CONTRACT_CHIP_TEXT)
                        } else {
                            (tier_color(link.tier), tier_color(link.tier))
                        };
                        vec![chip(&painter, mid, &truncate_with_ellipsis(&detail, 60), mono(11.0), accent, text, local)]
                    } else {
                        let color = tier_color(link.tier);
                        let mut shown = vec![chip(
                            &painter,
                            mid - vec2(0.0, 11.0 * local),
                            &tier_text(link.tier, link.candidates),
                            font(11.0),
                            color,
                            color,
                            local,
                        )];
                        if link.conditional.is_some() {
                            shown.push(chip(
                                &painter,
                                mid + vec2(0.0, 11.0 * local),
                                "conditional",
                                font(11.0),
                                TIER_POSSIBLE,
                                TIER_POSSIBLE,
                                local,
                            ));
                        } else if link.crossing {
                            shown.push(chip(
                                &painter,
                                mid + vec2(0.0, 11.0 * local),
                                "contract",
                                font(11.0),
                                LANG_JSON,
                                CONTRACT_CHIP_TEXT,
                                local,
                            ));
                        }
                        shown
                    };
                    if shown.iter().any(|r| gate.hovers(*r)) {
                        let bottom = shown.iter().map(|r| r.bottom()).fold(f32::MIN, f32::max);
                        hover_detail = Some((pos2(mid.x, bottom + 14.0 * local), detail, link.crossing));
                    }
                }
            }
        }
    }

    // The hovered chip says everything about its link, over everything else.
    if let Some((at, detail, crossing)) = hover_detail {
        let (accent, text) = if crossing { (LANG_JSON, CONTRACT_CHIP_TEXT) } else { (TEXT_SECONDARY, TEXT_PRIMARY) };
        chip(&painter, at, &detail, mono(11.5), accent, text, local);
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(title: &str, language: &str) -> PipelineStep {
        PipelineStep {
            title: title.into(),
            detail: String::new(),
            language: language.into(),
            path: PathBuf::from(format!("src/{title}.rs")),
            line: 3,
        }
    }

    fn lane(name: &str, layers: &[usize]) -> PipelineLane {
        PipelineLane {
            name: name.into(),
            crossing: false,
            layers: layers
                .iter()
                .enumerate()
                .map(|(l, &n)| (0..n).map(|i| step(&format!("{name}-{l}-{i}"), "Rust")).collect())
                .collect(),
            links: Vec::new(),
            truncated: 0,
        }
    }

    fn group(key: &str, expanded: bool, lanes: Vec<PipelineLane>) -> PipelineGroup {
        PipelineGroup { key: key.into(), title: key.into(), count: lanes.len(), expanded, lanes }
    }

    fn view() -> PipelineView {
        PipelineView {
            groups: vec![
                group("cross", true, vec![lane("a", &[2, 1, 1]), lane("b", &[1, 5, 1, 1, 1, 1, 1, 1, 1, 1])]),
                group("routes.rs", false, vec![lane("c", &[1, 1]), lane("d", &[1])]),
                group("Programs", false, vec![lane("e", &[1, 2, 3])]),
            ],
        }
    }

    fn all_rects(l: &PipelineLayout) -> Vec<Rect> {
        let mut rects: Vec<Rect> = l.pills.clone();
        for g in &l.groups {
            rects.push(g.header);
            for lane in &g.lanes {
                rects.extend(lane.cards.iter().map(|c| c.1));
                rects.extend(lane.more.iter().map(|m| m.0));
            }
        }
        rects
    }

    #[test]
    fn nothing_overlaps_and_everything_stays_inside_the_district() {
        let open: BTreeSet<String> = ["routes.rs".to_string(), "Programs".to_string()].into();
        for width in [2800.0, 4000.0] {
            let l = layout(&view(), width, &open);
            let rects = all_rects(&l);
            for (i, a) in rects.iter().enumerate() {
                assert!(a.min.x >= 0.0 && a.max.x <= width && a.max.y <= l.height, "{a:?} outside");
                for b in &rects[i + 1..] {
                    assert!(!a.intersects(*b), "{a:?} overlaps {b:?}");
                }
            }
            for pair in l.groups.windows(2) {
                assert!(pair[0].rect.bottom() <= pair[1].rect.top());
            }
            for g in &l.groups {
                for lane in &g.lanes {
                    assert!(g.rect.contains_rect(lane.rect));
                    assert!(lane.cards.iter().all(|c| lane.rect.contains_rect(c.1)));
                    assert!(lane.more.iter().all(|m| lane.rect.contains_rect(m.0)));
                }
            }
        }
    }

    #[test]
    fn layout_is_deterministic_and_capped() {
        let open = BTreeSet::new();
        let a = layout(&view(), 2800.0, &open);
        assert_eq!(a, layout(&view(), 2800.0, &open));
        let b = &a.groups[0].lanes[1];
        // 10 layers: 7 shown; the second layer holds 5, so 3 cards and "+2 more"; 3 layers hidden.
        assert_eq!(b.cards.iter().map(|c| c.0.layer).max(), Some(MAX_LAYERS - 1));
        assert_eq!(b.cards.iter().filter(|c| c.0.layer == 1).count(), MAX_STACK);
        assert_eq!(b.more.iter().map(|m| m.1).collect::<Vec<_>>(), [2, 3]);
        assert_eq!(b.cards.len(), 1 + 3 + 5);
    }

    #[test]
    fn content_grows_with_the_open_groups_and_only_open_groups_have_lanes() {
        let v = view();
        let none = BTreeSet::new();
        let closed = layout(&v, 2800.0, &none);
        assert_eq!(closed.groups.iter().map(|g| g.expanded).collect::<Vec<_>>(), [true, false, false]);
        assert_eq!(closed.groups.iter().map(|g| g.lanes.len()).collect::<Vec<_>>(), [2, 0, 0]);
        let one: BTreeSet<String> = ["routes.rs".to_string()].into();
        let opened = layout(&v, 2800.0, &one);
        assert!(opened.height > closed.height);
        assert_eq!(opened.groups[1].lanes.len(), 2);
        // Toggling a group open by default closes it.
        let cross: BTreeSet<String> = ["cross".to_string()].into();
        let fewer = layout(&v, 2800.0, &cross);
        assert!(fewer.height < closed.height);
        assert!(!fewer.groups[0].expanded && fewer.groups[0].lanes.is_empty());
    }

    #[test]
    fn chips_name_every_tier() {
        assert_eq!(tier_text(EvidenceTier::Proven, 1), "proven");
        assert_eq!(tier_text(EvidenceTier::PossibleSet, 3), "1 of 3");
        assert_eq!(tier_text(EvidenceTier::Unresolved, 0), "unresolved");
        let link = PipelineLink {
            from: StepSlot { layer: 0, index: 0 },
            to: StepSlot { layer: 0, index: 1 },
            tier: EvidenceTier::Proven,
            candidates: 1,
            conditional: Some("dev".into()),
            crossing: true,
            label: "fleet-command.schema.json#/definitions/ExecutionStart".into(),
        };
        assert_eq!(
            link_detail(&link),
            "proven · conditional: dev · fleet-command.schema.json#/definitions/ExecutionStart"
        );
        assert_eq!(language_mark("Enforce").0, "En");
        assert_eq!(language_mark("JSON").0, "{}");
    }

    #[test]
    fn a_group_can_be_found_in_the_world() {
        let world = WorldLayout::compute(
            Rect::from_min_size(Pos2::ZERO, vec2(2800.0, 1800.0)),
            &crate::world::DistrictExtents {
                pipeline: Some(layout(&view(), 2800.0, &BTreeSet::new()).height),
                changes: None,
            },
        );
        let l = layout(&view(), 2800.0, &BTreeSet::new());
        let r = group_world_rect(&world, &l, 2).unwrap();
        assert!(world.pipeline.contains_rect(r));
        assert!(group_world_rect(&world, &l, 3).is_none());
    }
}
