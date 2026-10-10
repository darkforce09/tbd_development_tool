use egui::{pos2, vec2, Rect};

use super::{DistrictExtents, WorldLayout};
use crate::camera::Stop;

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::from_min_size(pos2(x, y), vec2(w, h))
}

#[test]
fn districts_keep_their_places_around_the_map_and_never_overlap() {
    for code in
        [rect(0.0, 0.0, 1200.0, 500.0), rect(-5_000.0, 300.0, 90_000.0, 40_000.0), rect(10.0, 10.0, 600.0, 9_000.0)]
    {
        let w = WorldLayout::compute(code, &DistrictExtents::default());
        assert_eq!(w.code, code, "the map is never moved");
        assert!(w.code_cell.contains_rect(code));
        assert!(w.pipeline.bottom() < code.top(), "Pipeline is north");
        assert!(w.files.right() < w.code_cell.left(), "Files is west");
        assert!(w.run.left() > w.code_cell.right(), "Run is east");
        assert!(w.desk.top() > code.bottom(), "the Desk is below the map");
        assert!(w.changes.top() > w.desk.bottom(), "Changes is south");
        let all = [w.pipeline, w.files, w.code_cell, w.desk, w.run, w.changes];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert!(!a.intersects(*b), "{a:?} overlaps {b:?}");
            }
            assert!(w.bounds.contains_rect(*a));
        }
    }
}

#[test]
fn districts_grow_with_the_map_so_the_world_view_stays_in_proportion() {
    let small = WorldLayout::compute(rect(0.0, 0.0, 1000.0, 600.0), &DistrictExtents::default());
    let large = WorldLayout::compute(rect(0.0, 0.0, 84_000.0, 36_000.0), &DistrictExtents::default());
    assert_eq!(small.scale, 1.0, "a small project's districts are at their natural size");
    assert!((large.scale - 30.0).abs() < 1e-3);
    let ratio = |w: &WorldLayout| w.files.width() / w.code_cell.width();
    assert!(ratio(&large) > 0.3 && ratio(&large) < 1.0, "Files beside a huge map is still visible: {}", ratio(&large));
}

#[test]
fn a_point_belongs_to_the_district_it_is_in() {
    let w = WorldLayout::compute(rect(0.0, 0.0, 2800.0, 1800.0), &DistrictExtents::default());
    for stop in Stop::DISTRICTS {
        assert_eq!(w.region_at(w.rect(stop).center()), Some(stop));
    }
    assert_eq!(w.region_at(pos2(w.code_cell.center().x, w.code_cell.top() - 1.0)), None, "the gaps are no district");
}

#[test]
fn the_view_tells_which_stop_the_camera_is_at() {
    let w = WorldLayout::compute(rect(0.0, 0.0, 2800.0, 1800.0), &DistrictExtents::default());
    assert_eq!(w.stop_for_view(w.bounds.expand(100.0)), Stop::World);
    for stop in Stop::DISTRICTS {
        let r = w.rect(stop);
        assert_eq!(w.stop_for_view(Rect::from_center_size(r.center(), r.size() * 0.8)), stop);
    }
    // Centred on a gap, the district shown most wins.
    let gap = Rect::from_center_size(pos2(w.files.right() + 80.0, w.files.center().y), vec2(400.0, 300.0));
    assert!(matches!(w.stop_for_view(gap), Stop::Files | Stop::Code));
}

#[test]
fn a_taller_pipeline_grows_north_and_moves_nothing_else() {
    for code in [rect(0.0, 0.0, 2800.0, 1800.0), rect(-5_000.0, 300.0, 90_000.0, 40_000.0)] {
        let fixed = WorldLayout::compute(code, &DistrictExtents::default());
        let short = WorldLayout::compute(code, &DistrictExtents { pipeline: Some(200.0), changes: None });
        assert_eq!(short, fixed, "content shorter than the fixed height keeps it");
        let mut last_top = fixed.pipeline.top();
        for content in [1_500.0, 4_000.0, 25_000.0] {
            let w = WorldLayout::compute(code, &DistrictExtents { pipeline: Some(content), changes: None });
            assert_eq!(w.code, fixed.code, "the map is never moved");
            assert_eq!(w.code_cell, fixed.code_cell);
            assert_eq!(w.pipeline.bottom(), fixed.pipeline.bottom(), "Pipeline's bottom edge stays put");
            assert_eq!(w.pipeline.width(), fixed.pipeline.width());
            assert!((w.pipeline.height() - content * w.scale).abs() < 1e-2 * w.scale);
            assert!(w.pipeline.top() < last_top, "it grows north");
            last_top = w.pipeline.top();
            assert_eq!((w.files, w.run, w.desk, w.changes), (fixed.files, fixed.run, fixed.desk, fixed.changes));
            let all = [w.pipeline, w.files, w.code_cell, w.desk, w.run, w.changes];
            for (i, a) in all.iter().enumerate() {
                for b in &all[i + 1..] {
                    assert!(!a.intersects(*b), "{a:?} overlaps {b:?}");
                }
                assert!(w.bounds.contains_rect(*a));
            }
        }
    }
}

#[test]
fn a_taller_changes_district_grows_south() {
    let code = rect(0.0, 0.0, 2800.0, 1800.0);
    let fixed = WorldLayout::compute(code, &DistrictExtents::default());
    let w = WorldLayout::compute(code, &DistrictExtents { pipeline: None, changes: Some(3_000.0) });
    assert_eq!((w.code, w.code_cell, w.pipeline, w.desk), (fixed.code, fixed.code_cell, fixed.pipeline, fixed.desk));
    assert_eq!(w.changes.top(), fixed.changes.top());
    assert!(w.changes.bottom() > fixed.changes.bottom());
}

#[test]
fn the_centre_width_is_known_before_the_world_is() {
    for code in [rect(0.0, 0.0, 1200.0, 500.0), rect(-5_000.0, 300.0, 90_000.0, 40_000.0)] {
        let w = WorldLayout::compute(code, &DistrictExtents::default());
        let width = WorldLayout::centre_width_local(code);
        assert!((width * w.scale - w.pipeline.width()).abs() < 1e-2 * w.scale);
    }
}
