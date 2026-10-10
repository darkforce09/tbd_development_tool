//! What floats over the canvas: the compass at the top right and the Dock at the bottom.

use eframe::egui;
use egui::{Align2, Id, Order};
use studio_canvas::Stop;
use studio_ui::color_tokens::*;
use studio_ui::{compass, dock, CompassClick, CompassStop, DockItem};

use super::commands::{dock_commands, run_command, AppCommand, CommandSpec};
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
            Some(CompassClick::Stop(i)) => run_command(self, AppCommand::Fly(COMPASS[i].0)),
            Some(CompassClick::ActualSize) => run_command(self, AppCommand::ActualSize),
            None => {}
        }
    }
}

/// A Dock icon for a command: the Run commands in the Run colour, Trace in the Pipeline's, the
/// tools after a divider, All tools after another.
fn dock_item(cmd: AppCommand, spec: &CommandSpec) -> DockItem {
    let (color, divider_before) = match cmd {
        AppCommand::Build | AppCommand::Test => (DISTRICT_RUN, false),
        AppCommand::Trace => (DISTRICT_PIPELINE, false),
        AppCommand::Tool(i) => (TEXT_SECONDARY, i == 0),
        AppCommand::AllTools => (TEXT_SECONDARY, true),
        _ => (TEXT_DIM, false),
    };
    DockItem {
        icon: spec.icon,
        label: spec.title.clone(),
        tooltip: spec.detail.clone(),
        color,
        divider_before,
        running: false,
    }
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
    /// tool, from the command registry. Running things from it comes later; for now each icon goes
    /// to what it would run.
    pub(crate) fn render_dock(&mut self, ctx: &egui::Context) {
        if self.current_project_path.is_none() {
            return;
        }
        let commands = dock_commands(&self.canvas_state);
        let items: Vec<DockItem> = commands.iter().map(|(cmd, spec)| dock_item(*cmd, spec)).collect();
        let clicked = egui::Area::new(Id::new("dock"))
            .order(Order::Foreground)
            .anchor(Align2::CENTER_BOTTOM, [0.0, -14.0])
            .show(ctx, |ui| dock(ui, &items))
            .inner;
        if let Some(i) = clicked {
            run_command(self, commands[i].0);
        }
        self.render_tool_sources(ctx);
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

#[cfg(test)]
mod tests {
    use super::*;
    use studio_canvas::{CanvasState, RunView, ToolTile};

    #[test]
    fn the_dock_shows_what_it_showed_before_the_registry() {
        let tool = |name: &str, invocation: &str| ToolTile {
            name: name.to_string(),
            kind: "script",
            icon: egui_phosphor::regular::TERMINAL,
            invocation: invocation.to_string(),
            description: None,
            file: "package.json".into(),
            line: 3,
            children: Vec::new(),
        };
        let mut canvas = CanvasState::default();
        let run = RunView { tools: vec![tool("dev", "npm run dev"), tool("ci", "")], ..Default::default() };
        canvas.districts.run = Some(std::sync::Arc::new(run));
        let items: Vec<(String, String, egui::Color32, bool)> = dock_commands(&canvas)
            .iter()
            .map(|(cmd, spec)| dock_item(*cmd, spec))
            .map(|i| (i.label, i.tooltip, i.color, i.divider_before))
            .collect();
        let row = |l: &str, t: &str, c, d| (l.to_string(), t.to_string(), c, d);
        assert_eq!(
            items,
            [
                row("Build", "Shows the packages the project builds", DISTRICT_RUN, false),
                row("Test", "Shows the tests, by package", DISTRICT_RUN, false),
                row("Trace", "Shows what the selection connects to, or the pipeline", DISTRICT_PIPELINE, false),
                row("dev", "npm run dev", TEXT_SECONDARY, true),
                row("ci", "script", TEXT_SECONDARY, false),
                row("All tools", "", TEXT_SECONDARY, true),
                row("Add a tool", "Where Studio finds tools", TEXT_DIM, false),
            ]
        );
    }
}
