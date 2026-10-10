//! The command registry: what the Dock, the View menu and the search palette can run, in one
//! place. `command_list` says what there is, in a fixed order; `run_command` is the one place a
//! command runs. Nothing here changes what the map says: commands move the camera, switch what
//! the canvas shows, or open panels.

use egui::{Pos2, Rect, Vec2};
use egui_phosphor::regular as icon;
use studio_canvas::districts::run::{section_world_rect, tile_world_rect};
use studio_canvas::shortcuts::ShortcutAction;
use studio_canvas::{CameraTarget, CanvasState, RunSection, Stop};

use super::view_menu::VIEW_TOGGLES;
use super::StudioApp;

/// Something the app can do on request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AppCommand {
    /// Fly to a district, or to the world view.
    Fly(Stop),
    /// Back to 100%.
    ActualSize,
    /// Switch a View toggle, by its index in `VIEW_TOGGLES`.
    ToggleView(usize),
    /// Open another project folder.
    OpenFolder,
    /// Show the keyboard-shortcut sheet.
    ShowShortcuts,
    /// Open or close the debug panel.
    ToggleDebug,
    /// Show the packages the project builds.
    Build,
    /// Show the tests, by package.
    Test,
    /// Show what the selection connects to, or the pipeline.
    Trace,
    /// Show every tool of the Run district.
    AllTools,
    /// Show where Studio finds tools.
    AddTool,
    /// Show a Run tool, by its index among the Run district's tools.
    Tool(usize),
}

/// How a command is shown: in the palette (title, detail), the Dock (title as label, detail as
/// tooltip) and on the shortcut sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    /// Stable and unique: "fly.files", "view.toggle.0", "run.tool.3".
    pub id: String,
    pub title: String,
    pub detail: String,
    pub shortcut: Option<ShortcutAction>,
    pub icon: &'static str,
}

/// The stops, in the palette's order.
const STOPS: [Stop; 7] = [Stop::World, Stop::Code, Stop::Desk, Stop::Files, Stop::Run, Stop::Changes, Stop::Pipeline];

/// Most tools pinned to the Dock.
pub const PINNED: usize = 8;

/// Every command, in a fixed order: the stops, 100%, the View toggles, the app's own, then the
/// Dock's. The Run tools are left out: the palette lists them as tools from the sources, and
/// `AppCommand::Tool` shows one.
pub fn command_list(canvas: &CanvasState) -> Vec<(AppCommand, CommandSpec)> {
    let mut commands: Vec<AppCommand> = STOPS.into_iter().map(AppCommand::Fly).collect();
    commands.push(AppCommand::ActualSize);
    commands.extend((0..VIEW_TOGGLES.len()).map(AppCommand::ToggleView));
    commands.extend([
        AppCommand::OpenFolder,
        AppCommand::ShowShortcuts,
        AppCommand::ToggleDebug,
        AppCommand::Build,
        AppCommand::Test,
        AppCommand::Trace,
        AppCommand::AllTools,
        AppCommand::AddTool,
    ]);
    with_specs(canvas, commands)
}

/// The Dock's commands, left to right: Build, Test and Trace, the first tools of the Run
/// district, All tools and Add a tool.
pub fn dock_commands(canvas: &CanvasState) -> Vec<(AppCommand, CommandSpec)> {
    let tools = canvas.districts.run.as_ref().map_or(0, |r| r.tools.len().min(PINNED));
    let mut commands = vec![AppCommand::Build, AppCommand::Test, AppCommand::Trace];
    commands.extend((0..tools).map(AppCommand::Tool));
    commands.extend([AppCommand::AllTools, AppCommand::AddTool]);
    with_specs(canvas, commands)
}

fn with_specs(canvas: &CanvasState, commands: Vec<AppCommand>) -> Vec<(AppCommand, CommandSpec)> {
    commands.into_iter().filter_map(|c| command_spec(canvas, c).map(|spec| (c, spec))).collect()
}

/// How `cmd` is shown, or `None` when it names a View toggle or tool that does not exist.
pub fn command_spec(canvas: &CanvasState, cmd: AppCommand) -> Option<CommandSpec> {
    let spec = |id: &str, title: &str, detail: &str, shortcut, icon| CommandSpec {
        id: id.to_string(),
        title: title.to_string(),
        detail: detail.to_string(),
        shortcut,
        icon,
    };
    Some(match cmd {
        AppCommand::Fly(Stop::World) => spec(
            "fly.world",
            "Go to the world view",
            Stop::World.description(),
            Some(ShortcutAction::World),
            icon::GLOBE,
        ),
        AppCommand::Fly(stop) => CommandSpec {
            id: format!("fly.{}", stop.label().to_lowercase()),
            title: format!("Go to {}", stop.label()),
            detail: stop.description().to_string(),
            shortcut: None,
            icon: icon::MAP_PIN,
        },
        AppCommand::ActualSize => spec(
            "view.actual_size",
            "Actual size",
            "Zoom to 100%, where text is crisp",
            Some(ShortcutAction::ActualSize),
            icon::FRAME_CORNERS,
        ),
        AppCommand::ToggleView(i) => {
            let toggle = VIEW_TOGGLES.get(i)?;
            let on = (toggle.get)(canvas);
            CommandSpec {
                id: format!("view.toggle.{i}"),
                title: toggle.title.to_string(),
                detail: format!("{} · {}", if on { "On" } else { "Off" }, toggle.tooltip),
                shortcut: None,
                icon: if on { icon::EYE } else { icon::EYE_SLASH },
            }
        }
        AppCommand::OpenFolder => {
            spec("app.open_folder", "Open folder…", "Open another project", None, icon::FOLDER_OPEN)
        }
        AppCommand::ShowShortcuts => {
            spec("app.shortcuts", "Keyboard shortcuts", "Every key and gesture", None, icon::KEYBOARD)
        }
        AppCommand::ToggleDebug => {
            spec("app.debug", "Debug panel", "Frame times and counters", Some(ShortcutAction::ToggleDebug), icon::BUG)
        }
        AppCommand::Build => spec("run.build", "Build", "Shows the packages the project builds", None, icon::HAMMER),
        AppCommand::Test => spec("run.test", "Test", "Shows the tests, by package", None, icon::TEST_TUBE),
        AppCommand::Trace => {
            spec("run.trace", "Trace", "Shows what the selection connects to, or the pipeline", None, icon::PATH)
        }
        AppCommand::AllTools => spec("run.all_tools", "All tools", "", None, icon::SQUARES_FOUR),
        AppCommand::AddTool => spec("run.add_tool", "Add a tool", "Where Studio finds tools", None, icon::PLUS),
        AppCommand::Tool(i) => {
            let tool = canvas.districts.run.as_ref()?.tools.get(i)?;
            CommandSpec {
                id: format!("run.tool.{i}"),
                title: tool.name.clone(),
                detail: if tool.invocation.is_empty() { tool.kind.to_string() } else { tool.invocation.clone() },
                shortcut: None,
                icon: tool.icon,
            }
        }
    })
}

/// Runs `cmd`: the one place commands execute.
pub fn run_command(app: &mut StudioApp, cmd: AppCommand) {
    let canvas = &mut app.canvas_state;
    let run = canvas.districts.run.clone();
    let world = canvas.world;
    let section = |s| run.as_ref().map(|r| section_world_rect(&world, r, s));
    match cmd {
        AppCommand::Fly(stop) => canvas.fly_to(CameraTarget::Stop(stop)),
        AppCommand::ActualSize => canvas.actual_size(),
        AppCommand::ToggleView(i) => {
            if let Some(toggle) = VIEW_TOGGLES.get(i) {
                let flag = (toggle.flag)(canvas);
                *flag = !*flag;
            }
        }
        AppCommand::OpenFolder => app.open_folder_dialog(),
        AppCommand::ShowShortcuts => app.shortcut_sheet_open = true,
        AppCommand::ToggleDebug => app.debug.open = !app.debug.open,
        AppCommand::Build => fly_or_run(canvas, section(RunSection::Builds)),
        AppCommand::Test => fly_or_run(canvas, section(RunSection::Tests)),
        AppCommand::AllTools => fly_or_run(canvas, section(RunSection::Tools)),
        AppCommand::Tool(i) => {
            canvas.run_selected = Some(i);
            let tile = run.as_ref().and_then(|r| tile_world_rect(&world, r, i));
            fly_or_run(canvas, tile.map(|t| t.expand(t.width() * 1.5)));
        }
        AppCommand::Trace => match traced_rect(app) {
            Some(rect) => app.canvas_state.fly_to(CameraTarget::Rect(rect)),
            None => app.canvas_state.fly_to(CameraTarget::Stop(Stop::Pipeline)),
        },
        AppCommand::AddTool => app.show_tool_sources = !app.show_tool_sources,
    }
}

/// Flies to a place in the Run district, or to the district while its tools are still being
/// read.
fn fly_or_run(canvas: &mut CanvasState, rect: Option<Rect>) {
    match rect.filter(|r| r.is_positive()) {
        Some(r) => canvas.fly_to(CameraTarget::Rect(r)),
        None => canvas.fly_to(CameraTarget::Stop(Stop::Run)),
    }
}

/// The cards the selection's trace lights, as one rectangle.
fn traced_rect(app: &StudioApp) -> Option<Rect> {
    let traced = app.canvas_state.trace_nodes.as_ref()?;
    traced
        .iter()
        .filter_map(|id| app.graph.nodes.get(id))
        .map(|n| Rect::from_min_size(Pos2::from(n.position), Vec2::new(n.size[0], n.size[1])))
        .reduce(|a, b| a.union(b))
}

#[cfg(test)]
impl StudioApp {
    /// An app with no project and no window, for tests. Every field starts empty.
    pub(crate) fn for_test() -> Self {
        Self {
            graph: Default::default(),
            canvas_state: Default::default(),
            frame_counter: Default::default(),
            last_frame_time: Default::default(),
            fps: Default::default(),
            gpu: Default::default(),
            debug: Default::default(),
            current_project_path: Default::default(),
            path_input: Default::default(),
            project_stats: Default::default(),
            is_from_cache: Default::default(),
            load_error: Default::default(),
            loader_rx: Default::default(),
            is_loading: Default::default(),
            loading_stage: Default::default(),
            loading_files_done: Default::default(),
            loading_total_files: Default::default(),
            loading_progress: Default::default(),
            search_index: Default::default(),
            folder_loads: Default::default(),
            palette: Default::default(),
            shortcut_sheet_open: Default::default(),
            activity: Default::default(),
            desk_reader: Default::default(),
            sources: Default::default(),
            egui_ctx: Default::default(),
            show_tool_sources: Default::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studio_canvas::{RunView, ToolTile};

    fn tool(name: &str) -> ToolTile {
        ToolTile {
            name: name.to_string(),
            kind: "cargo bin",
            icon: icon::TERMINAL,
            invocation: format!("cargo run --bin {name}"),
            description: None,
            file: "src/main.rs".into(),
            line: 1,
            children: Vec::new(),
        }
    }

    fn canvas_with_tools(n: usize) -> CanvasState {
        let mut canvas = CanvasState::default();
        let run = RunView { tools: (0..n).map(|i| tool(&format!("t{i}"))).collect(), ..Default::default() };
        canvas.districts.run = Some(std::sync::Arc::new(run));
        canvas
    }

    #[test]
    fn command_list_is_stable() {
        let canvas = canvas_with_tools(3);
        let first = command_list(&canvas);
        assert_eq!(first, command_list(&canvas), "same input, same list");
        let ids: Vec<&str> = first.iter().map(|(_, s)| s.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "fly.world",
                "fly.code",
                "fly.desk",
                "fly.files",
                "fly.run",
                "fly.changes",
                "fly.pipeline",
                "view.actual_size",
                "view.toggle.0",
                "view.toggle.1",
                "view.toggle.2",
                "view.toggle.3",
                "view.toggle.4",
                "view.toggle.5",
                "view.toggle.6",
                "view.toggle.7",
                "view.toggle.8",
                "app.open_folder",
                "app.shortcuts",
                "app.debug",
                "run.build",
                "run.test",
                "run.trace",
                "run.all_tools",
                "run.add_tool",
            ]
        );
        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "ids are unique");
    }

    #[test]
    fn titles_read_naturally_and_toggles_show_their_state() {
        let mut canvas = CanvasState::default();
        let title = |c: &CanvasState, cmd| command_spec(c, cmd).unwrap();
        assert_eq!(title(&canvas, AppCommand::Fly(Stop::Files)).title, "Go to Files");
        assert_eq!(title(&canvas, AppCommand::Fly(Stop::Desk)).title, "Go to Desk");
        assert_eq!(title(&canvas, AppCommand::ActualSize).title, "Actual size");
        assert_eq!(title(&canvas, AppCommand::OpenFolder).title, "Open folder…");
        assert_eq!(title(&canvas, AppCommand::ShowShortcuts).title, "Keyboard shortcuts");
        let wires = title(&canvas, AppCommand::ToggleView(0));
        assert_eq!(wires.title, "Show wires");
        assert!(wires.detail.starts_with("On · "));
        canvas.show_wires = false;
        assert!(title(&canvas, AppCommand::ToggleView(0)).detail.starts_with("Off · "));
        assert_eq!(command_spec(&canvas, AppCommand::ToggleView(VIEW_TOGGLES.len())), None);
        assert_eq!(command_spec(&canvas, AppCommand::Tool(0)), None, "no Run district yet");
    }

    #[test]
    fn every_dock_item_maps_to_a_command() {
        let canvas = canvas_with_tools(PINNED + 3);
        let dock = dock_commands(&canvas);
        let commands: Vec<AppCommand> = dock.iter().map(|(c, _)| *c).collect();
        let mut want = vec![AppCommand::Build, AppCommand::Test, AppCommand::Trace];
        want.extend((0..PINNED).map(AppCommand::Tool));
        want.extend([AppCommand::AllTools, AppCommand::AddTool]);
        assert_eq!(commands, want, "the first {PINNED} tools are pinned");
        let listed = command_list(&canvas);
        for (cmd, spec) in &dock {
            match cmd {
                AppCommand::Tool(i) => assert_eq!(spec.title, format!("t{i}")),
                other => assert!(listed.iter().any(|(c, s)| c == other && s == spec), "{other:?} is in the list"),
            }
        }
        let no_run = dock_commands(&CanvasState::default());
        assert_eq!(no_run.len(), 5, "Build, Test, Trace, All tools, Add a tool");
    }

    #[test]
    fn flying_to_files_sets_the_camera_target() {
        let mut app = StudioApp::for_test();
        run_command(&mut app, AppCommand::Fly(Stop::Files));
        assert_eq!(app.canvas_state.camera.pending, Some((CameraTarget::Stop(Stop::Files), true)));
    }

    #[test]
    fn commands_do_what_the_dock_and_menu_did() {
        let mut app = StudioApp::for_test();
        run_command(&mut app, AppCommand::ToggleView(0));
        assert!(!app.canvas_state.show_wires);
        run_command(&mut app, AppCommand::ToggleView(0));
        assert!(app.canvas_state.show_wires);

        run_command(&mut app, AppCommand::AddTool);
        assert!(app.show_tool_sources);
        run_command(&mut app, AppCommand::ShowShortcuts);
        assert!(app.shortcut_sheet_open);
        run_command(&mut app, AppCommand::ToggleDebug);
        assert!(app.debug.open);

        // With no Run district yet, the Run commands fly to the district.
        for cmd in [AppCommand::Build, AppCommand::Test, AppCommand::AllTools, AppCommand::Tool(2)] {
            run_command(&mut app, cmd);
            assert_eq!(app.canvas_state.camera.pending, Some((CameraTarget::Stop(Stop::Run), true)), "{cmd:?}");
        }
        assert_eq!(app.canvas_state.run_selected, Some(2));
        run_command(&mut app, AppCommand::Trace);
        assert_eq!(app.canvas_state.camera.pending, Some((CameraTarget::Stop(Stop::Pipeline), true)));
    }
}
