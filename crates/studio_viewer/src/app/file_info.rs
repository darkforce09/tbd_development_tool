use std::io::Read;
use std::path::Path;

use eframe::egui;
use egui::{Color32, FontFamily, FontId, RichText, Vec2};
use studio_graph::{human_bytes, FileContent, Node};
use studio_ui::color_tokens::*;

use super::StudioApp;

/// Bytes of a too-large text file shown in the read-only preview.
const TEXT_PREVIEW_BYTES: usize = 64 * 1024;
/// Bytes shown in a binary file's hex preview.
const HEX_PREVIEW_BYTES: usize = 512;
/// Images above this size are not decoded for preview.
const MAX_PREVIEW_IMAGE_BYTES: u64 = 64 * 1024 * 1024;

impl StudioApp {
    /// Inspector body for files that are not editable code: images, binaries, oversized text,
    /// symlinks and unreadable files.
    pub(crate) fn render_file_info(&mut self, ui: &mut egui::Ui, node: &Node) {
        let Some(path) = node.file_path.as_deref() else { return };
        let path = Path::new(path);

        let kind = node.content.label().unwrap_or_default();
        let size = node.size_bytes.map(human_bytes).unwrap_or_default();
        ui.label(
            RichText::new(format!("{kind} · {size}"))
                .font(FontId::new(12.0, FontFamily::Proportional))
                .color(TEXT_PRIMARY),
        );
        if let Ok(modified) = path.symlink_metadata().and_then(|m| m.modified()) {
            if let Ok(age) = modified.elapsed() {
                ui.label(RichText::new(format!("modified {} ago", human_duration(age.as_secs()))).color(TEXT_DIM));
            }
        }
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            if ui.button(format!("{} Open in system app", egui_phosphor::regular::ARROW_SQUARE_OUT)).clicked() {
                if let Err(e) = open::that_detached(path) {
                    self.canvas_state.status_message = Some(format!("Could not open {}: {e}", path.display()));
                }
            }
            if let Some(dir) = path.parent() {
                if ui.button(format!("{} Show folder", egui_phosphor::regular::FOLDER_OPEN)).clicked() {
                    if let Err(e) = open::that_detached(dir) {
                        self.canvas_state.status_message = Some(format!("Could not open {}: {e}", dir.display()));
                    }
                }
            }
        });
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(6.0);

        match &node.content {
            FileContent::Image => image_preview(ui, path, node.size_bytes.unwrap_or(0)),
            FileContent::TooLarge => {
                ui.label(
                    RichText::new(format!("First {} (read-only):", human_bytes(TEXT_PREVIEW_BYTES as u64)))
                        .color(TEXT_DIM),
                );
                match read_head(path, TEXT_PREVIEW_BYTES) {
                    Ok(bytes) if !bytes.contains(&0) => text_preview(ui, &String::from_utf8_lossy(&bytes)),
                    Ok(bytes) => text_preview(ui, &hex_dump(&bytes[..bytes.len().min(HEX_PREVIEW_BYTES)])),
                    Err(e) => error_line(ui, &e.to_string()),
                }
            }
            FileContent::Binary => {
                ui.label(RichText::new(format!("First {} bytes:", HEX_PREVIEW_BYTES)).color(TEXT_DIM));
                match read_head(path, HEX_PREVIEW_BYTES) {
                    Ok(bytes) => text_preview(ui, &hex_dump(&bytes)),
                    Err(e) => error_line(ui, &e.to_string()),
                }
            }
            FileContent::Symlink(target) => {
                ui.label(RichText::new(format!("Points to {target}")).color(TEXT_PRIMARY));
                let resolved = path.parent().map_or_else(|| Path::new(target).to_path_buf(), |p| p.join(target));
                if !resolved.exists() {
                    error_line(ui, "target does not exist (broken link)");
                }
            }
            FileContent::Unreadable(err) => error_line(ui, err),
            FileContent::Code => {}
        }
    }
}

fn image_preview(ui: &mut egui::Ui, path: &Path, size: u64) {
    if size > MAX_PREVIEW_IMAGE_BYTES {
        ui.label(
            RichText::new(format!(
                "Larger than {}; open it in a system app to view.",
                human_bytes(MAX_PREVIEW_IMAGE_BYTES)
            ))
            .color(TEXT_DIM),
        );
        return;
    }
    let uri = format!("file://{}", path.display());
    let max = Vec2::new(ui.available_width(), 420.0);
    let response =
        ui.add(egui::Image::new(uri.as_str()).max_size(max).maintain_aspect_ratio(true).show_loading_spinner(true));
    if let Ok(egui::load::TexturePoll::Ready { texture }) =
        ui.ctx().try_load_texture(&uri, egui::TextureOptions::default(), egui::SizeHint::default())
    {
        ui.label(RichText::new(format!("{} × {} px", texture.size.x as u32, texture.size.y as u32)).color(TEXT_DIM));
    }
    response.on_hover_text(path.display().to_string());
}

fn text_preview(ui: &mut egui::Ui, text: &str) {
    egui::ScrollArea::vertical().id_salt("file_info_preview").max_height(420.0).show(ui, |ui| {
        ui.add(
            egui::Label::new(RichText::new(text).font(FontId::new(10.5, FontFamily::Monospace)).color(TEXT_SECONDARY))
                .wrap(),
        );
    });
}

fn error_line(ui: &mut egui::Ui, msg: &str) {
    ui.label(RichText::new(format!("⚠ {msg}")).color(Color32::from_rgb(251, 146, 60)));
}

fn read_head(path: &Path, max: usize) -> std::io::Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(max);
    std::fs::File::open(path)?.take(max as u64).read_to_end(&mut buf)?;
    Ok(buf)
}

/// `00000000  89 50 4e 47 …  |.PNG…|` lines, 16 bytes each.
pub fn hex_dump(bytes: &[u8]) -> String {
    bytes
        .chunks(16)
        .enumerate()
        .map(|(i, chunk)| {
            let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
            let ascii: String =
                chunk.iter().map(|&b| if b.is_ascii_graphic() || b == b' ' { b as char } else { '.' }).collect();
            format!("{:08x}  {:<47}  |{}|", i * 16, hex.join(" "), ascii)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn human_duration(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_dump_formats_offsets_and_ascii() {
        let dump = hex_dump(b"\x89PNG\r\n\x1a\n0123456789ABCDEF");
        let lines: Vec<&str> = dump.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("00000000  89 50 4e 47 0d 0a 1a 0a 30 31"));
        assert!(lines[0].ends_with("|.PNG....01234567|"));
        assert!(lines[1].starts_with("00000010  38 39"));
    }

    #[test]
    fn read_head_caps_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("big.txt");
        std::fs::write(&p, vec![b'x'; 10_000]).unwrap();
        assert_eq!(read_head(&p, 100).unwrap().len(), 100);
    }
}
