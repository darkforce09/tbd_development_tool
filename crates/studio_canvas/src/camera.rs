//! The camera. Every move of the view that is not the user's own wheel, pinch or drag goes
//! through here as a [`CameraTarget`]: a district, a folder, a card or a rectangle.
//!
//! A flight takes [`FLIGHT_SECONDS`] and eases out. It follows the smooth zoom-and-pan path of van
//! Wijk and Nuij (the one map apps use): a short hop zooms in or out about the point that stays
//! put, a long one rises, glides and settles, so the eye never loses where it is. Any input from
//! the user cancels it on the spot. A flight that lands at 100% snaps to whole pixels so text is
//! crisp.

use egui::{Pos2, Rect, Vec2};
use studio_graph::NodeId;

/// How long every flight takes.
pub const FLIGHT_SECONDS: f64 = 0.85;
/// Path curvature: higher rises further on long flights (van Wijk's rho; √2 as in d3 and Mapbox).
const RHO: f64 = std::f64::consts::SQRT_2;

/// The places of the world, the same in every project (docs/VISUAL_LANGUAGE.md, Districts).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Stop {
    /// All five districts.
    World,
    /// North: the flows a run takes.
    Pipeline,
    /// West: the disk as it is, and the project's settings.
    Files,
    /// Centre: the map of the code.
    Code,
    /// Below the map: open files and plans.
    Desk,
    /// East: tools, builds, tests and the console.
    Run,
    /// South: commits, worktrees and agents at work.
    Changes,
}

impl Stop {
    /// Every district, in reading order.
    pub const DISTRICTS: [Stop; 6] = [Stop::Pipeline, Stop::Files, Stop::Code, Stop::Desk, Stop::Run, Stop::Changes];

    pub fn label(self) -> &'static str {
        match self {
            Stop::World => "All",
            Stop::Pipeline => "Pipeline",
            Stop::Files => "Files",
            Stop::Code => "Code",
            Stop::Desk => "Desk",
            Stop::Run => "Run",
            Stop::Changes => "Changes",
        }
    }
}

/// Where the camera should go.
#[derive(Debug, Clone, PartialEq)]
pub enum CameraTarget {
    Stop(Stop),
    /// A folder of the code map (cluster id), filling the view.
    Folder(String),
    /// A card, centred; the zoom only grows if the card would be too small to read.
    Node(NodeId),
    /// A world rectangle, filling the view.
    Rect(Rect),
}

/// A view of the world: `screen = world * zoom + pan`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub pan: Vec2,
    pub zoom: f32,
}

impl View {
    /// The world point at the middle of `screen`.
    fn centre(&self, screen: Rect) -> Pos2 {
        ((screen.center().to_vec2() - self.pan) / self.zoom).to_pos2()
    }

    fn from_centre(centre: Pos2, zoom: f32, screen: Rect) -> Self {
        View { pan: screen.center().to_vec2() - centre.to_vec2() * zoom, zoom }
    }
}

/// A view that fits `world` inside `screen`, `margin` points in from each side, zoom clamped.
pub fn fit(world: Rect, screen: Rect, margin: f32, zoom_range: (f32, f32)) -> View {
    let usable = (screen.size() - Vec2::splat(margin * 2.0)).max(Vec2::splat(1.0));
    let zoom = (usable.x / world.width().max(1.0)).min(usable.y / world.height().max(1.0));
    View::from_centre(world.center(), zoom.clamp(zoom_range.0, zoom_range.1), screen)
}

/// A view centring `world` at `zoom`.
pub fn centre_on(world: Pos2, zoom: f32, screen: Rect) -> View {
    View::from_centre(world, zoom, screen)
}

/// Moves `view` by less than a pixel so the world point `anchor` lands on a whole device pixel;
/// at 100% everything drawn from it is then crisp.
pub fn snap(view: View, anchor: Pos2, pixels_per_point: f32) -> View {
    let at = anchor.to_vec2() * view.zoom + view.pan;
    let rounded = (at * pixels_per_point).round() / pixels_per_point;
    View { pan: view.pan + (rounded - at), zoom: view.zoom }
}

/// A flight from one view to another.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Flight {
    from: View,
    to: View,
    screen: Rect,
    started: f64,
    /// World point to snap to a whole pixel on arrival, for flights that land at 100%.
    snap_anchor: Option<Pos2>,
}

impl Flight {
    pub fn new(from: View, to: View, screen: Rect, started: f64, snap_anchor: Option<Pos2>) -> Self {
        Self { from, to, screen, started, snap_anchor }
    }

    pub fn destination(&self) -> View {
        self.to
    }

    pub fn snap_anchor(&self) -> Option<Pos2> {
        self.snap_anchor
    }

    /// The view at time `now`, and whether the flight has landed.
    pub fn at(&self, now: f64) -> (View, bool) {
        let t = ((now - self.started) / FLIGHT_SECONDS).clamp(0.0, 1.0);
        // Within a nanosecond counts as there: `start + duration - start` need not be exact.
        if t >= 1.0 - 1e-9 {
            return (self.to, true);
        }
        (path(self.from, self.to, self.screen, ease_out(t)), false)
    }
}

/// Fast at first, gentle on arrival.
fn ease_out(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(3)
}

/// The view a fraction `t` of the way along the smooth zoom-and-pan path from `a` to `b`
/// (van Wijk & Nuij 2003, as in d3's `interpolateZoom`). Works in world centre and visible world
/// width; a pure zoom keeps the centre fixed and changes zoom in log space.
fn path(a: View, b: View, screen: Rect, t: f64) -> View {
    let width = screen.width().max(1.0) as f64;
    let (c0, c1) = (a.centre(screen), b.centre(screen));
    let (w0, w1) = (width / a.zoom as f64, width / b.zoom as f64);
    let (dx, dy) = ((c1.x - c0.x) as f64, (c1.y - c0.y) as f64);
    let d2 = dx * dx + dy * dy;
    let (u, w) = if d2 < 1e-9 * w0 * w0 {
        // Same centre: zoom in log space.
        (t, w0 * (w1 / w0).powf(t))
    } else {
        let d = d2.sqrt();
        let (rho2, rho4) = (RHO * RHO, RHO.powi(4));
        let b0 = (w1 * w1 - w0 * w0 + rho4 * d2) / (2.0 * w0 * rho2 * d);
        let b1 = (w1 * w1 - w0 * w0 - rho4 * d2) / (2.0 * w1 * rho2 * d);
        let r0 = ((b0 * b0 + 1.0).sqrt() - b0).ln();
        let r1 = ((b1 * b1 + 1.0).sqrt() - b1).ln();
        let s = t * (r1 - r0) / RHO;
        let u = w0 / (rho2 * d) * (r0.cosh() * (RHO * s + r0).tanh() - r0.sinh());
        (u, w0 * r0.cosh() / (RHO * s + r0).cosh())
    };
    let centre = Pos2::new((c0.x as f64 + u * dx) as f32, (c0.y as f64 + u * dy) as f32);
    View::from_centre(centre, (width / w) as f32, screen)
}

/// The camera's state: a target waiting for the canvas to know its size, and the flight under way.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Camera {
    /// Where to go next, and whether to fly there (`false` jumps).
    pub pending: Option<(CameraTarget, bool)>,
    pub flight: Option<Flight>,
}

impl Camera {
    pub fn fly_to(&mut self, target: CameraTarget) {
        self.pending = Some((target, true));
    }

    pub fn jump_to(&mut self, target: CameraTarget) {
        self.pending = Some((target, false));
        self.flight = None;
    }

    /// Stops wherever the camera is: the user took over.
    pub fn cancel(&mut self) {
        self.pending = None;
        self.flight = None;
    }

    pub fn is_flying(&self) -> bool {
        self.flight.is_some() || self.pending.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> Rect {
        Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 1000.0))
    }

    fn close(a: View, b: View) -> bool {
        (a.pan - b.pan).length() < 0.01 && (a.zoom - b.zoom).abs() < 1e-5
    }

    #[test]
    fn a_flight_starts_where_the_view_is_and_lands_on_the_target() {
        let from = View { pan: Vec2::new(10.0, 20.0), zoom: 0.02 };
        let to =
            fit(Rect::from_min_size(Pos2::new(50_000.0, 9_000.0), Vec2::splat(800.0)), screen(), 40.0, (1e-4, 4.0));
        let flight = Flight::new(from, to, screen(), 100.0, None);
        assert!(close(flight.at(100.0).0, from));
        assert_eq!(flight.at(100.0 + FLIGHT_SECONDS), (to, true));
        assert!(!flight.at(100.0 + FLIGHT_SECONDS * 0.5).1);
    }

    #[test]
    fn zooming_into_a_point_keeps_it_still_and_zoom_changes_smoothly() {
        let from = View { pan: Vec2::ZERO, zoom: 0.1 };
        let middle = from.centre(screen());
        let to = centre_on(middle, 1.6, screen());
        let flight = Flight::new(from, to, screen(), 0.0, None);
        let mut last = from.zoom;
        for i in 1..=20 {
            let (view, _) = flight.at(FLIGHT_SECONDS * i as f64 / 20.0);
            assert!(view.zoom >= last, "zoom only grows");
            assert!((view.centre(screen()) - middle).length() < 0.01, "the point stays in the middle");
            last = view.zoom;
        }
    }

    #[test]
    fn a_long_glide_rises_above_both_ends() {
        let a = centre_on(Pos2::new(0.0, 0.0), 0.5, screen());
        let b = centre_on(Pos2::new(200_000.0, 0.0), 0.5, screen());
        let flight = Flight::new(a, b, screen(), 0.0, None);
        let (mid, _) = flight.at(FLIGHT_SECONDS * 0.3);
        assert!(mid.zoom < 0.5 * 0.2, "zoomed out to see both ends on the way, got {}", mid.zoom);
    }

    #[test]
    fn fit_keeps_the_margin_and_the_zoom_range() {
        let world = Rect::from_min_size(Pos2::new(100.0, 100.0), Vec2::new(1520.0, 400.0));
        let view = fit(world, screen(), 40.0, (1e-4, 4.0));
        assert!((view.zoom - 1.0).abs() < 1e-6);
        let left = world.min.x * view.zoom + view.pan.x;
        assert!((left - 40.0).abs() < 1e-3);
        assert_eq!(fit(world, screen(), 40.0, (1e-4, 0.5)).zoom, 0.5);
    }

    #[test]
    fn snapping_puts_the_anchor_on_a_whole_pixel() {
        let view = View { pan: Vec2::new(10.3, -4.6), zoom: 1.0 };
        let anchor = Pos2::new(100.25, 7.0);
        let snapped = snap(view, anchor, 2.0);
        let at = anchor.to_vec2() * snapped.zoom + snapped.pan;
        assert_eq!((at * 2.0).round(), at * 2.0);
        assert!((snapped.pan - view.pan).length() < 0.5);
    }

    #[test]
    fn user_input_cancels_and_jumps_drop_the_flight() {
        let mut camera = Camera::default();
        camera.fly_to(CameraTarget::Stop(Stop::Run));
        assert!(camera.is_flying());
        camera.cancel();
        assert!(!camera.is_flying());
        camera.flight = Some(Flight::new(
            View { pan: Vec2::ZERO, zoom: 1.0 },
            View { pan: Vec2::ZERO, zoom: 2.0 },
            screen(),
            0.0,
            None,
        ));
        camera.jump_to(CameraTarget::Stop(Stop::World));
        assert!(camera.flight.is_none() && matches!(camera.pending, Some((_, false))));
    }
}
