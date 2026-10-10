use studio_viewer::app::StudioApp;

fn main() -> eframe::Result<()> {
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([800.0, 600.0])
        .with_title("Studio")
        .with_app_id("tbd-studio");
    // The app draws its own title bar (app::title_bar). On macOS the native window buttons stay,
    // over the bar's left end; elsewhere the app draws its own (app::window_frame).
    let viewport = if cfg!(target_os = "macos") {
        viewport.with_fullsize_content_view(true).with_titlebar_shown(false).with_title_shown(false)
    } else {
        viewport.with_decorations(false)
    };
    let native_options = eframe::NativeOptions { viewport, renderer: eframe::Renderer::Wgpu, ..Default::default() };

    let initial_path = std::env::args().nth(1).map(std::path::PathBuf::from);

    // The app name also names the settings directory (e.g. ~/.local/share/tbd-studio).
    eframe::run_native("tbd-studio", native_options, Box::new(move |cc| Ok(Box::new(StudioApp::new(cc, initial_path)))))
}
