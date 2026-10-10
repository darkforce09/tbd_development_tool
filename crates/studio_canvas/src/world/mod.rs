//! The world: five districts that keep their places in every project (docs/VISUAL_LANGUAGE.md,
//! Districts, law L7). Pipeline north, Files west, Code in the centre with the Desk below it, Run
//! east, Changes south.
//!
//! Everything is anchored on the Code map's bounds, which the layout engine owns: a district
//! growing never moves the map; the map growing moves the districts, and the camera follows.
//!
//! Districts other than Code are laid out in *local* units (points at 100%) and drawn at
//! `scale` world units per local unit, so they stay in proportion to the map in the world view of
//! a project of any size, and read at 100% when the camera visits them.

pub mod paint;

#[cfg(test)]
mod tests;

use egui::{pos2, vec2, Color32, Pos2, Rect};
use studio_graph::Graph;
use studio_ui::color_tokens::*;

use crate::camera::Stop;

/// The Code map at its natural size, in local units: its bounds set the district scale.
const CODE_NATURAL: [f32; 2] = [2800.0, 1800.0];
/// Width of the centre column, at least, in local units.
const CENTRE_MIN_WIDTH: f32 = 2800.0;
const PIPELINE_HEIGHT: f32 = 900.0;
const DESK_HEIGHT: f32 = 1200.0;
const CHANGES_HEIGHT: f32 = 1000.0;
const SIDE_WIDTH: f32 = 1600.0;
/// Space between districts, in local units.
const GAP: f32 = 160.0;

impl Stop {
    /// What the district holds, in one line.
    pub fn description(self) -> &'static str {
        match self {
            Stop::World => "The whole project.",
            Stop::Pipeline => "The flows a run takes, across processes and languages.",
            Stop::Files => "The disk as it is: folders, sizes, ignore rules and settings.",
            Stop::Code => "Every package and folder, and how they connect.",
            Stop::Desk => "Open files and plans, side by side.",
            Stop::Run => "Tools, apps, builds, tests and the console.",
            Stop::Changes => "Commits, worktrees and agents at work.",
        }
    }

    /// The district's colour (docs/VISUAL_LANGUAGE.md, Districts).
    pub fn color(self) -> Color32 {
        match self {
            Stop::World => TEXT_SECONDARY,
            Stop::Pipeline => DISTRICT_PIPELINE,
            Stop::Files => DISTRICT_FILES,
            Stop::Code => DISTRICT_CODE,
            Stop::Desk => DISTRICT_DESK,
            Stop::Run => DISTRICT_RUN,
            Stop::Changes => DISTRICT_CHANGES,
        }
    }
}

/// How tall the content of the districts that grow with it is, in local units. `None` keeps the
/// district at its fixed height; content shorter than that never shrinks it. Pipeline grows north
/// and Changes south, so the Code map never moves.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DistrictExtents {
    pub pipeline: Option<f32>,
    pub changes: Option<f32>,
}

/// Where every district is, in world units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldLayout {
    /// World units per local unit for every district but Code.
    pub scale: f32,
    /// The Code map's own bounds.
    pub code: Rect,
    /// The centre column's row that holds the map (at least as wide as the other centre rows).
    pub code_cell: Rect,
    pub pipeline: Rect,
    pub files: Rect,
    pub desk: Rect,
    pub run: Rect,
    pub changes: Rect,
    /// Everything.
    pub bounds: Rect,
}

impl Default for WorldLayout {
    fn default() -> Self {
        Self::compute(
            Rect::from_min_size(Pos2::ZERO, vec2(CODE_NATURAL[0], CODE_NATURAL[1])),
            &DistrictExtents::default(),
        )
    }
}

impl WorldLayout {
    /// World units per local unit around a map with these bounds.
    fn scale_for(code: Rect) -> f32 {
        (code.width() / CODE_NATURAL[0]).max(code.height() / CODE_NATURAL[1]).max(1.0)
    }

    /// The width of the centre column (Pipeline, Desk, Changes) around a map with these bounds, in
    /// local units: what a district's content is laid out in before the world is.
    pub fn centre_width_local(code: Rect) -> f32 {
        let scale = Self::scale_for(code);
        code.width().max(CENTRE_MIN_WIDTH * scale) / scale
    }

    /// Lays the districts out around the Code map's bounds, each as tall as `extents` asks.
    pub fn compute(code: Rect, extents: &DistrictExtents) -> Self {
        let scale = Self::scale_for(code);
        let gap = GAP * scale;
        let width = code.width().max(CENTRE_MIN_WIDTH * scale);
        let (left, right) = (code.center().x - width * 0.5, code.center().x + width * 0.5);
        let row = |top: f32, height: f32| Rect::from_min_size(pos2(left, top), vec2(width, height * scale));

        let code_cell = Rect::from_x_y_ranges(left..=right, code.y_range());
        let height = |content: Option<f32>, fixed: f32| content.map_or(fixed, |h| h.max(fixed));
        let pipeline_height = height(extents.pipeline, PIPELINE_HEIGHT);
        // Built up from its bottom edge, which stays put however tall it grows.
        let pipeline_bottom = code.top() - gap;
        let pipeline =
            Rect::from_x_y_ranges(left..=right, (pipeline_bottom - pipeline_height * scale)..=pipeline_bottom);
        let desk = row(code.bottom() + gap, DESK_HEIGHT);
        let changes = row(desk.bottom() + gap, height(extents.changes, CHANGES_HEIGHT));
        let side = SIDE_WIDTH * scale;
        let files = Rect::from_min_max(pos2(left - gap - side, code.top()), pos2(left - gap, desk.bottom()));
        let run = Rect::from_min_max(pos2(right + gap, code.top()), pos2(right + gap + side, desk.bottom()));
        let bounds = [pipeline, files, desk, run, changes].into_iter().fold(code_cell, |all, r| all.union(r));
        Self { scale, code, code_cell, pipeline, files, desk, run, changes, bounds }
    }

    /// Lays the districts out around the graph's map: the root folder, else every card. `extents`
    /// gets the centre column's width in local units and says how tall the districts' content is.
    pub fn for_graph(graph: &Graph, extents: impl FnOnce(f32) -> DistrictExtents) -> Self {
        let code = code_bounds(graph).unwrap_or_else(|| Self::default().code);
        Self::compute(code, &extents(Self::centre_width_local(code)))
    }

    /// The district's rectangle; the whole world for [`Stop::World`], the map's row for Code.
    pub fn rect(&self, stop: Stop) -> Rect {
        match stop {
            Stop::World => self.bounds,
            Stop::Pipeline => self.pipeline,
            Stop::Files => self.files,
            Stop::Code => self.code_cell,
            Stop::Desk => self.desk,
            Stop::Run => self.run,
            Stop::Changes => self.changes,
        }
    }

    /// The district at a world point, if any.
    pub fn region_at(&self, world: Pos2) -> Option<Stop> {
        Stop::DISTRICTS.into_iter().find(|&stop| self.rect(stop).contains(world))
    }

    /// Where a view of `view` (world rectangle) is: the world while it shows most of it, else the
    /// district under its middle, else the one it shows most of.
    pub fn stop_for_view(&self, view: Rect) -> Stop {
        let area = |r: Rect| r.width().max(0.0) * r.height().max(0.0);
        if area(view.intersect(self.bounds)) >= 0.45 * area(self.bounds) {
            return Stop::World;
        }
        if let Some(stop) = self.region_at(view.center()) {
            return stop;
        }
        Stop::DISTRICTS
            .into_iter()
            .map(|stop| (stop, area(view.intersect(self.rect(stop)))))
            .filter(|&(_, shown)| shown > 0.0)
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(Stop::World, |(stop, _)| stop)
    }

    /// The camera zoom at which a district other than Code reads at 100%.
    pub fn local_zoom_one(&self) -> f32 {
        1.0 / self.scale
    }
}

/// The map's bounds: the root folder's frame when there is one, else every card's.
pub fn code_bounds(graph: &Graph) -> Option<Rect> {
    let frame = |position: [f32; 2], size: [f32; 2]| Rect::from_min_size(Pos2::from(position), vec2(size[0], size[1]));
    if let Some(root) = graph.clusters.iter().find(|c| c.parent_id.is_none() && c.size[0] > 1.0) {
        return Some(frame(root.position, root.size));
    }
    graph.nodes.values().map(|n| frame(n.position, n.size)).reduce(|a, b| a.union(b))
}
