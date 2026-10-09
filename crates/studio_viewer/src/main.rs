use studio_viewer::app::StudioApp;

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([800.0, 600.0])
            .with_title("Studio - Infinite Node Canvas")
            .with_app_id("tbd-studio"),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    let initial_path = std::env::args().nth(1).map(std::path::PathBuf::from);

    // The app name also names the settings directory (e.g. ~/.local/share/tbd-studio).
    eframe::run_native("tbd-studio", native_options, Box::new(move |cc| Ok(Box::new(StudioApp::new(cc, initial_path)))))
}
