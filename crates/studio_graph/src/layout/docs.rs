//! Routes for documentation wires inside one container (docs/VISUAL_LANGUAGE.md L2, L3).
//!
//! Documentation never changes the order or the columns: it is routed through free space the
//! code layout already leaves. Every container has a documentation strip across its top, under
//! the header. A documentation wire climbs from its source to the strip, runs along the strip on
//! its provider's own track, and descends to its target:
//!
//! - an item in a column leaves into the gap right of its column and is entered from the gap left
//!   of its column; those gaps are free from the strip down to the bottom of the columns;
//! - an item on the shelf of unwired items is reached through the free band above its row and
//!   the gaps between items in the row, and the left margin, which is free full height;
//! - the container's own documentation gates sit on its left (input) and right (output) edges,
//!   on the provider's strip track.
//!
//! Vertical documentation tracks sit 6 units into a channel and then every 8, which never lands on
//! the grid code tracks use (multiples of 8 from 12 or 24 into a gap), so a documentation wire is
//! never drawn on top of a code wire.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::{From, Item, To};
use crate::model::NodeId;

/// Distance of the documentation port below a card's top edge.
pub const DOC_PORT_OFFSET_Y: f32 = 9.0;
/// Spacing of the documentation strip's horizontal tracks and of vertical tracks in a channel.
const DOC_PITCH: f32 = 8.0;
/// First vertical track, from the left of a channel.
const DOC_INSET: f32 = 6.0;
/// Spacing of horizontal tracks in the bands between shelf rows.
const BAND_PITCH: f32 = 4.0;

/// Height of a documentation strip carrying `providers` tracks, at least `min`.
pub(super) fn strip_height(providers: usize, min: f32) -> f32 {
    if providers == 0 {
        min
    } else {
        (providers as f32 * DOC_PITCH + DOC_PITCH).max(min)
    }
}

/// The y of each provider's track in a strip from `top`, `height` high, centred.
pub(super) fn strip_tracks(providers: &BTreeSet<NodeId>, top: f32, height: f32) -> BTreeMap<NodeId, f32> {
    let start = top + (height - providers.len() as f32 * DOC_PITCH) * 0.5;
    providers.iter().enumerate().map(|(i, &k)| (k, start + (i as f32 + 0.5) * DOC_PITCH)).collect()
}

/// One row of the shelf: its vertical extent and its items left to right with their x ranges.
#[derive(Debug, Clone, Default)]
pub(super) struct ShelfRow {
    pub top: f32,
    pub items: Vec<(Item, f32, f32)>,
}

/// The free space of a laid-out container, relative to its top-left corner.
pub(super) struct Channels<'a> {
    pub width: f32,
    /// Provider → y of its strip track.
    pub tracks: &'a BTreeMap<NodeId, f32>,
    /// x range of each column's items, left to right.
    pub columns: Vec<(f32, f32)>,
    /// Column of every item placed in a column.
    pub column_of: HashMap<Item, usize>,
    pub rows: Vec<ShelfRow>,
    /// Top of the shelf and whether anything (columns, gates, backward wires) sits above it.
    pub shelf_top: f32,
    pub content_above_shelf: bool,
    /// Width of the left margin, free from top to bottom.
    pub margin: f32,
    /// Top-left corner and size of every item.
    pub rects: HashMap<Item, ([f32; 2], [f32; 2])>,
}

/// Gaps between shelf rows and between columns and the shelf.
const ITEM_GAP: f32 = super::ITEM_GAP;
const SHELF_GAP: f32 = super::SHELF_GAP;

impl Channels<'_> {
    /// x of a vertical track in the channel `[left, right]` for the provider ranked `k`.
    fn track_x(left: f32, right: f32, k: usize) -> f32 {
        let capacity = (((right - left - DOC_INSET) / DOC_PITCH).floor() as usize).max(1);
        left + DOC_INSET + (k % capacity) as f32 * DOC_PITCH
    }

    /// y of a horizontal track in the free band above shelf row `r`, or `None` when the strip
    /// itself is directly above the row.
    fn band_above(&self, r: usize, k: usize) -> Option<f32> {
        let (bottom, height) = if r > 0 {
            (self.rows[r].top, ITEM_GAP)
        } else if self.content_above_shelf {
            (self.shelf_top, SHELF_GAP)
        } else {
            return None;
        };
        let capacity = (((height - 2.0 * BAND_PITCH) / BAND_PITCH).floor() as usize).max(1);
        Some(bottom - height + BAND_PITCH + (k % capacity) as f32 * BAND_PITCH)
    }

    /// Where the shelf item `item` sits: row, index in the row.
    fn shelf_slot(&self, item: Item) -> Option<(usize, usize)> {
        self.rows.iter().enumerate().find_map(|(r, row)| row.items.iter().position(|it| it.0 == item).map(|i| (r, i)))
    }

    fn left_margin_x(&self, k: usize) -> f32 {
        Self::track_x(0.0, self.margin, k)
    }

    /// Points from the item's right side at `y` (relative to the item) up to the strip track
    /// `track`, ending on the track.
    fn exit(&self, item: Item, y: f32, track: f32, k: usize) -> Vec<[f32; 2]> {
        let ([x, top], [w, _]) = self.rects[&item];
        let (right, sy) = (x + w, top + y);
        if let Some(&l) = self.column_of.get(&item) {
            let gap_right = self.columns.get(l + 1).map_or(self.width, |c| c.0);
            let tx = Self::track_x(self.columns[l].1, gap_right, k);
            return vec![[right, sy], [tx, sy], [tx, track]];
        }
        let Some((r, i)) = self.shelf_slot(item) else { return vec![[right, sy], [right, track]] };
        let row = &self.rows[r];
        let gap_right = row.items.get(i + 1).map_or(self.width, |it| it.1);
        let tx = Self::track_x(right, gap_right, k);
        match self.band_above(r, k) {
            None => vec![[right, sy], [tx, sy], [tx, track]],
            Some(band) => {
                let mx = self.left_margin_x(k);
                vec![[right, sy], [tx, sy], [tx, band], [mx, band], [mx, track]]
            }
        }
    }

    /// Points from the strip track `track` down to the item's left side at `y` (relative to the
    /// item), starting on the track.
    fn entry(&self, item: Item, y: f32, track: f32, k: usize) -> Vec<[f32; 2]> {
        let ([x, top], _) = self.rects[&item];
        let ey = top + y;
        if let Some(&l) = self.column_of.get(&item) {
            let gap_left = if l == 0 { 0.0 } else { self.columns[l - 1].1 };
            let tx = Self::track_x(gap_left, self.columns[l].0, k);
            return vec![[tx, track], [tx, ey], [x, ey]];
        }
        let Some((r, i)) = self.shelf_slot(item) else { return vec![[x, track], [x, ey]] };
        if i == 0 {
            // First in its row: straight down the left margin.
            let mx = self.left_margin_x(k);
            return vec![[mx, track], [mx, ey], [x, ey]];
        }
        let tx = Self::track_x(self.rows[r].items[i - 1].2, x, k);
        match self.band_above(r, k) {
            None => vec![[tx, track], [tx, ey], [x, ey]],
            Some(band) => {
                let mx = self.left_margin_x(k);
                vec![[mx, track], [mx, band], [tx, band], [tx, ey], [x, ey]]
            }
        }
    }

    /// Routes every documentation piece of the container. `exit_y(item, provider)` and
    /// `entry_y(item, provider)` give the port offsets from an item's top.
    pub fn route(
        &self,
        links: &BTreeSet<(From, To)>,
        exit_y: &dyn Fn(Item, NodeId) -> f32,
        entry_y: &dyn Fn(Item, NodeId) -> f32,
    ) -> BTreeMap<(From, To), Vec<[f32; 2]>> {
        let rank: HashMap<NodeId, usize> = self.tracks.keys().enumerate().map(|(i, &k)| (k, i)).collect();
        let mut out = BTreeMap::new();
        for &(from, to) in links {
            let key = from.key();
            let (Some(&track), Some(&k)) = (self.tracks.get(&key), rank.get(&key)) else { continue };
            let mut points = match from {
                From::Gate(_) => vec![[0.0, track]],
                From::Item(item, _) => self.exit(item, exit_y(item, key), track, k),
            };
            match to {
                To::Gate(_) => points.push([self.width, track]),
                To::Item(item, _) => points.extend(self.entry(item, entry_y(item, key), track, k)),
            }
            out.insert((from, to), super::simplify(points));
        }
        out
    }
}
