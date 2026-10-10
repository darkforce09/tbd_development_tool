//! What floats over the canvas: the compass at the top right and the Dock at the bottom.

use eframe::egui;
use egui::{Align2, Id, Order, Pos2, Rect, Vec2};
use studio_canvas::districts::run::{section_world_rect, tile_world_rect};
use studio_canvas::{CameraTarget, RunSection, Stop};
use studio_ui::color_tokens::*;
use studio_ui::{compass, dock, CompassClick, CompassStop, DockItem};

use super::title_bar::TITLE_BAR_HEIGHT;
use super::StudioApp;

/// The compass's buttons, placed as the districts lie on the map.
const COMPASS: [(Stop, [u8; 2]); 7] = [
    (Stop::World, [0, 0]),
    (Stop::Pipeline, [1, 0]),
    (Stop::Files, [0, 1]),
    (Stop::Code, [1, 1]),
    (Stop::Run, [2, 1]),
    (Stop::Desk, [1, 2]),
    (Stop::Changes, [1, 3]),
];

impl StudioApp {
    pub(crate) fn render_compass(&mut self, ctx: &egui::Context) {
        if self.current_project_path.is_none() {
            return;
        }
        let stops: Vec<CompassStop<'_>> = COMPASS
            .iter()
            .map(|&(stop, cell)| CompassStop {
                label: stop.label(),
                color: stop.color(),
                cell,
                tooltip: stop.description(),
            })
            .collect();
        let current = COMPASS.iter().position(|&(stop, _)| stop == self.canvas_state.stop);
        let zoom = self.canvas_state.zoom_percent();
        let clicked = egui::Area::new(Id::new("compass"))
            .order(Order::Foreground)
            .anchor(Align2::RIGHT_TOP, [-14.0, TITLE_BAR_HEIGHT + 12.0])
            .show(ctx, |ui| compass(ui, &stops, current, zoom))
            .inner;
        match clicked {
            Some(CompassClick::Stop(i)) => self.canvas_state.fly_to(CameraTarget::Stop(COMPASS[i].0)),
            Some(CompassClick::ActualSize) => self.canvas_state.actual_size(),
            None => {}
        }
    }
}

/// Most tools pinned to the Dock.
const PINNED: usize = 8;

/// What a Dock icon does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DockAction {
    Build,
    Test,
    Trace,
    Tool(usize),
    AllTools,
    AddTool,
}

/// Where Studio looks for tools, for "Add a tool".
const TOOL_SOURCES: &[(&str, &str)] = &[
    ("Cargo binaries and examples", "src/main.rs, src/bin/*, [[bin]], examples/*"),
    ("Cargo aliases", ".cargo/config.toml [alias]"),
    ("Command-line subcommands", "clap #[derive(Subcommand)] enums, nested"),
    ("Scripts", "package.json \"scripts\""),
    ("Make and just", "Makefile targets, justfile recipes"),
    ("CI", ".github/workflows/*.yml jobs"),
];

impl StudioApp {
    /// The Dock: Build, Test and Trace, the first tools of the Run district, All tools and Add a
    /// tool. Running things from it comes later; for now each icon goes to what it would run.
    pub(crate) fn render_dock(&mut self, ctx: &egui::Context) {
        if self.current_project_path.is_none() {
            return;
        }
        use egui_phosphor::regular as icon;
        let run = self.canvas_state.districts.run.clone();
        let mut items = vec![
            DockItem {
                icon: icon::HAMMER,
                label: "Build".into(),
                tooltip: "Shows the packages the project builds".into(),
                color: DISTRICT_RUN,
                divider_before: false,
                running: false,
            },
            DockItem {
                icon: icon::TEST_TUBE,
                label: "Test".into(),
                tooltip: "Shows the tests, by package".into(),
                color: DISTRICT_RUN,
                divider_before: false,
                running: false,
            },
            DockItem {
                icon: icon::PATH,
                label: "Trace".into(),
                tooltip: "Shows what the selection connects to, or the pipeline".into(),
                color: DISTRICT_PIPELINE,
                divider_before: false,
                running: false,
            },
        ];
        let mut actions = vec![DockAction::Build, DockAction::Test, DockAction::Trace];
        if let Some(run) = &run {
            for (i, tool) in run.tools.iter().take(PINNED).enumerate() {
                items.push(DockItem {
                    icon: tool.icon,
                    label: tool.name.clone(),
                    tooltip: if tool.invocation.is_empty() { tool.kind.to_string() } else { tool.invocation.clone() },
                    color: TEXT_SECONDARY,
                    divider_before: i == 0,
                    running: false,
                });
                actions.push(DockAction::Tool(i));
            }
        }
        items.push(DockItem {
            icon: icon::SQUARES_FOUR,
            label: "All tools".into(),
            tooltip: String::new(),
            color: TEXT_SECONDARY,
            divider_before: true,
            running: false,
        });
        items.push(DockItem {
            icon: icon::PLUS,
            label: "Add a tool".into(),
            tooltip: "Where Studio finds tools".into(),
            color: TEXT_DIM,
            divider_before: false,
            running: false,
        });
        actions.extend([DockAction::AllTools, DockAction::AddTool]);

        let clicked = egui::Area::new(Id::new("dock"))
            .order(Order::Foreground)
            .anchor(Align2::CENTER_BOTTOM, [0.0, -14.0])
            .show(ctx, |ui| dock(ui, &items))
            .inner;
        let Some(action) = clicked.map(|i| actions[i]) else {
            self.render_tool_sources(ctx);
            return;
        };
        let world = self.canvas_state.world;
        let section = |s| run.as_ref().map(|r| section_world_rect(&world, r, s));
        match action {
            DockAction::Build => self.fly_or_run(section(RunSection::Builds)),
            DockAction::Test => self.fly_or_run(section(RunSection::Tests)),
            DockAction::AllTools => self.fly_or_run(section(RunSection::Tools)),
            DockAction::Tool(i) => {
                self.canvas_state.run_selected = Some(i);
                let tile = run.as_ref().and_then(|r| tile_world_rect(&world, r, i));
                self.fly_or_run(tile.map(|t| t.expand(t.width() * 1.5)));
            }
            DockAction::Trace => match self.traced_rect() {
                Some(rect) => self.canvas_state.fly_to(CameraTarget::Rect(rect)),
                None => self.canvas_state.fly_to(CameraTarget::Stop(Stop::Pipeline)),
            },
            DockAction::AddTool => self.show_tool_sources = !self.show_tool_sources,
        }
        self.render_tool_sources(ctx);
    }

    /// Flies to a place in the Run district, or to the district while its tools are still being
    /// read.
    fn fly_or_run(&mut self, rect: Option<Rect>) {
        match rect.filter(|r| r.is_positive()) {
            Some(r) => self.canvas_state.fly_to(CameraTarget::Rect(r)),
            None => self.canvas_state.fly_to(CameraTarget::Stop(Stop::Run)),
        }
    }

    /// The cards the selection's trace lights, as one rectangle.
    fn traced_rect(&self) -> Option<Rect> {
        let traced = self.canvas_state.trace_nodes.as_ref()?;
        traced
            .iter()
            .filter_map(|id| self.graph.nodes.get(id))
            .map(|n| Rect::from_min_size(Pos2::from(n.position), Vec2::new(n.size[0], n.size[1])))
            .reduce(|a, b| a.union(b))
    }

    /// "Add a tool": where Studio finds tools on its own.
    fn render_tool_sources(&mut self, ctx: &egui::Context) {
        if !self.show_tool_sources {
            return;
        }
        let mut open = true;
        egui::Window::new("Where Studio finds tools")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_BOTTOM, [0.0, -96.0])
            .show(ctx, |ui| {
                ui.label("Studio finds a project's tools by reading its files. Add one of these and it appears here:");
                ui.add_space(6.0);
                egui::Grid::new("tool_sources").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
                    for (what, where_) in TOOL_SOURCES {
                        ui.label(egui::RichText::new(*what).strong());
                        ui.label(egui::RichText::new(*where_).monospace().color(TEXT_SECONDARY));
                        ui.end_row();
                    }
                });
            });
        self.show_tool_sources = open;
    }
}
