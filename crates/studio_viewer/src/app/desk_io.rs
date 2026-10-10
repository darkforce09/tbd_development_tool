//! Reads the files the Desk asks for, off the UI thread, and hands them back as card contents.

use std::io::Read;
use std::path::Path;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use studio_canvas::{content_for_text, CardContent, DeskRequest};

use super::StudioApp;

/// Files above this are not shown as text.
pub const MAX_DESK_BYTES: u64 = 4 * 1024 * 1024;

/// The channel finished reads come back on.
pub struct DeskReader {
    tx: Sender<(u64, CardContent)>,
    rx: Receiver<(u64, CardContent)>,
}

impl Default for DeskReader {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self { tx, rx }
    }
}

/// Reads a file the way the Desk shows it: text, Markdown, binary or too large.
pub fn read_for_desk(path: &Path) -> CardContent {
    let size = match std::fs::metadata(path) {
        Ok(meta) => meta.len(),
        Err(e) => return CardContent::Unreadable(e.to_string()),
    };
    if size > MAX_DESK_BYTES {
        return CardContent::TooLarge(size);
    }
    let mut bytes = Vec::with_capacity(size as usize);
    if let Err(e) = std::fs::File::open(path).and_then(|mut f| f.read_to_end(&mut bytes)) {
        return CardContent::Unreadable(e.to_string());
    }
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return CardContent::Binary(size);
    }
    let text: Arc<str> = String::from_utf8_lossy(&bytes).into();
    content_for_text(path, text)
}

impl StudioApp {
    /// Starts reading what the Desk asked for, and fills in what has been read. Called every frame.
    pub(crate) fn poll_desk_reads(&mut self, ctx: &eframe::egui::Context) {
        for request in std::mem::take(&mut self.canvas_state.desk.requests) {
            let DeskRequest::Read { card, path } = request;
            let tx = self.desk_reader.tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send((card, read_for_desk(&path)));
                ctx.request_repaint();
            });
        }
        while let Ok((card, content)) = self.desk_reader.rx.try_recv() {
            self.canvas_state.desk.fill(card, content);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_read_as_what_they_are() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, bytes: &[u8]| {
            let p = dir.path().join(name);
            std::fs::write(&p, bytes).unwrap();
            p
        };
        assert!(matches!(read_for_desk(&write("a.rs", b"fn a() {}\n")), CardContent::Code(d) if d.line_count() == 1));
        assert!(matches!(read_for_desk(&write("README.md", b"# Hi\n")), CardContent::Markdown(_)));
        assert!(matches!(read_for_desk(&write("blob.bin", &[1, 0, 2])), CardContent::Binary(3)));
        assert!(matches!(read_for_desk(&dir.path().join("missing.rs")), CardContent::Unreadable(_)));
        let big = write("big.log", &vec![b'x'; MAX_DESK_BYTES as usize + 1]);
        assert!(matches!(read_for_desk(&big), CardContent::TooLarge(_)));
    }
}
