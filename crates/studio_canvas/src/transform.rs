use egui::{pos2, Pos2, Vec2};

/// Strict coordinate conversion helpers as specified in the functional requirements.
#[inline]
pub fn world_to_screen(world_pos: Pos2, pan: Vec2, zoom: f32) -> Pos2 {
    pos2(world_pos.x * zoom + pan.x, world_pos.y * zoom + pan.y)
}

#[inline]
pub fn screen_to_world(screen_pos: Pos2, pan: Vec2, zoom: f32) -> Pos2 {
    pos2((screen_pos.x - pan.x) / zoom, (screen_pos.y - pan.y) / zoom)
}

/// Canvas 2D Camera transformation holding pan offset and zoom level.
#[derive(Debug, Clone, Copy)]
pub struct CanvasTransform {
    pub pan: Vec2,
    pub zoom: f32,
    pub min_zoom: f32,
    pub max_zoom: f32,
}

impl Default for CanvasTransform {
    fn default() -> Self {
        Self {
            pan: Vec2::new(120.0, 100.0),
            zoom: 1.0,
            min_zoom: 0.0005,
            max_zoom: 4.0,
        }
    }
}

impl CanvasTransform {
    pub fn new(pan: Vec2, zoom: f32) -> Self {
        Self {
            pan,
            zoom: zoom.clamp(0.0005, 4.0),
            min_zoom: 0.0005,
            max_zoom: 4.0,
        }
    }

    #[inline]
    pub fn world_to_screen(&self, world_pos: Pos2) -> Pos2 {
        world_to_screen(world_pos, self.pan, self.zoom)
    }

    #[inline]
    pub fn screen_to_world(&self, screen_pos: Pos2) -> Pos2 {
        screen_to_world(screen_pos, self.pan, self.zoom)
    }

    #[inline]
    pub fn screen_to_world_rect(&self, screen_rect: egui::Rect) -> egui::Rect {
        let min_world = self.screen_to_world(screen_rect.min);
        let max_world = self.screen_to_world(screen_rect.max);
        egui::Rect::from_min_max(min_world, max_world)
    }


    /// Zoom centered accurately around the mouse cursor position.
    pub fn zoom_at_pointer(&mut self, pointer_screen: Pos2, factor: f32) {
        let world_before = self.screen_to_world(pointer_screen);
        let new_zoom = (self.zoom * factor).clamp(self.min_zoom, self.max_zoom);
        self.pan = pointer_screen.to_vec2() - world_before.to_vec2() * new_zoom;
        self.zoom = new_zoom;
    }

    pub fn pan_by(&mut self, delta: Vec2) {
        self.pan += delta;
    }

    pub fn reset(&mut self) {
        self.pan = Vec2::new(120.0, 100.0);
        self.zoom = 1.0;
    }

    /// Centers camera smoothly on the given world coordinate.
    pub fn center_on_world_pos(&mut self, target_world: Pos2, screen_rect: egui::Rect, target_zoom: Option<f32>) {
        if let Some(z) = target_zoom {
            self.zoom = z.clamp(self.min_zoom, self.max_zoom);
        }
        let screen_center = screen_rect.center();
        self.pan = Vec2::new(
            screen_center.x - target_world.x * self.zoom,
            screen_center.y - target_world.y * self.zoom,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coordinate_roundtrip() {
        let pan = Vec2::new(250.0, -130.0);
        let zoom = 1.45;
        let original_world = Pos2::new(123.4, 567.8);
        let screen = world_to_screen(original_world, pan, zoom);
        let reconstructed_world = screen_to_world(screen, pan, zoom);

        assert!((original_world.x - reconstructed_world.x).abs() < 1e-4);
        assert!((original_world.y - reconstructed_world.y).abs() < 1e-4);
    }
}
