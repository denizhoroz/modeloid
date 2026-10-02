//! Geometry of the annotation view, in image pixels: hit testing, drawing,
//! moving and resizing boxes, and the zoom / pan transform. No UI here.

use crate::api::label::LabelObject;

/// A point in image pixels.
pub type Point = [f64; 2];

/// Smallest box a drag draws, in image pixels; smaller ones are dropped.
pub const MIN_BOX: f64 = 2.0;

/// How the image sits in the stage: stage px = image px × `scale` + (`x`, `y`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub scale: f64,
    pub x: f64,
    pub y: f64,
}

impl View {
    pub const ZOOM_MIN: f64 = 0.05;
    pub const ZOOM_MAX: f64 = 40.0;

    /// The whole image, centred in a stage of `stage` size, with a small margin.
    pub fn fit(stage: Point, image: Point) -> View {
        let (iw, ih) = (image[0].max(1.0), image[1].max(1.0));
        let scale = ((stage[0] / iw).min(stage[1] / ih) * 0.96).clamp(Self::ZOOM_MIN, Self::ZOOM_MAX);
        View { scale, x: (stage[0] - iw * scale) / 2.0, y: (stage[1] - ih * scale) / 2.0 }
    }

    /// Zoomed by `factor`, keeping the image point under stage point `at` in place.
    pub fn zoomed(self, factor: f64, at: Point) -> View {
        let scale = (self.scale * factor).clamp(Self::ZOOM_MIN, Self::ZOOM_MAX);
        let image = self.to_image(at);
        View { scale, x: at[0] - image[0] * scale, y: at[1] - image[1] * scale }
    }

    /// The image point at stage point `stage`.
    pub fn to_image(self, stage: Point) -> Point {
        [(stage[0] - self.x) / self.scale, (stage[1] - self.y) / self.scale]
    }
}

/// A box corner or edge that resizes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    NW,
    N,
    NE,
    E,
    SE,
    S,
    SW,
    W,
}

impl Handle {
    pub const ALL: [Handle; 8] = [Handle::NW, Handle::N, Handle::NE, Handle::E, Handle::SE, Handle::S, Handle::SW, Handle::W];

    /// Where the handle sits on box `b` (x, y, w, h).
    pub fn at(self, b: [f64; 4]) -> Point {
        let (x0, y0, x1, y1, xm, ym) = (b[0], b[1], b[0] + b[2], b[1] + b[3], b[0] + b[2] / 2.0, b[1] + b[3] / 2.0);
        match self {
            Handle::NW => [x0, y0],
            Handle::N => [xm, y0],
            Handle::NE => [x1, y0],
            Handle::E => [x1, ym],
            Handle::SE => [x1, y1],
            Handle::S => [xm, y1],
            Handle::SW => [x0, y1],
            Handle::W => [x0, ym],
        }
    }

    pub fn cursor(self) -> &'static str {
        match self {
            Handle::NW | Handle::SE => "nwse-resize",
            Handle::NE | Handle::SW => "nesw-resize",
            Handle::N | Handle::S => "ns-resize",
            Handle::E | Handle::W => "ew-resize",
        }
    }
}

/// Whether an object can be resized with handles: boxes, not outlines.
pub fn resizable(object: &LabelObject) -> bool {
    object.polygon.is_none()
}

/// The handle of a selected box within `tolerance` of `p`, if any.
pub fn hit_handle(objects: &[LabelObject], selected: &[usize], p: Point, tolerance: f64) -> Option<(usize, Handle)> {
    selected.iter().rev().filter_map(|&i| Some((i, objects.get(i)?))).filter(|(_, o)| resizable(o)).find_map(|(i, o)| {
        Handle::ALL.into_iter().find(|h| {
            let [hx, hy] = h.at(o.bbox);
            (p[0] - hx).abs() <= tolerance && (p[1] - hy).abs() <= tolerance
        })
        .map(|h| (i, h))
    })
}

/// The object under `p`: the smallest box containing it (so nested boxes stay reachable).
pub fn hit_object(objects: &[LabelObject], p: Point) -> Option<usize> {
    objects
        .iter()
        .enumerate()
        .filter(|(_, o)| {
            let b = o.bbox;
            p[0] >= b[0] && p[0] <= b[0] + b[2] && p[1] >= b[1] && p[1] <= b[1] + b[3]
        })
        .min_by(|(_, a), (_, b)| (a.bbox[2] * a.bbox[3]).total_cmp(&(b.bbox[2] * b.bbox[3])))
        .map(|(i, _)| i)
}

/// The box between two points, kept inside an image of `size`.
pub fn rect_between(a: Point, b: Point, size: Point) -> [f64; 4] {
    let x0 = a[0].min(b[0]).clamp(0.0, size[0]);
    let y0 = a[1].min(b[1]).clamp(0.0, size[1]);
    let x1 = a[0].max(b[0]).clamp(0.0, size[0]);
    let y1 = a[1].max(b[1]).clamp(0.0, size[1]);
    [x0, y0, x1 - x0, y1 - y0]
}

/// Whether a drawn box is big enough to keep.
pub fn big_enough(b: [f64; 4]) -> bool {
    b[2] >= MIN_BOX && b[3] >= MIN_BOX
}

/// Box `b` with `handle` dragged to `p`; edges that cross over swap.
pub fn resized(b: [f64; 4], handle: Handle, p: Point, size: Point) -> [f64; 4] {
    let (mut x0, mut y0, mut x1, mut y1) = (b[0], b[1], b[0] + b[2], b[1] + b[3]);
    match handle {
        Handle::NW | Handle::W | Handle::SW => x0 = p[0],
        Handle::NE | Handle::E | Handle::SE => x1 = p[0],
        Handle::N | Handle::S => {}
    }
    match handle {
        Handle::NW | Handle::N | Handle::NE => y0 = p[1],
        Handle::SW | Handle::S | Handle::SE => y1 = p[1],
        Handle::E | Handle::W => {}
    }
    rect_between([x0, y0], [x1, y1], size)
}

/// `objects` with the `selected` ones moved by `delta`, as far as they all stay inside the image.
pub fn moved(objects: &[LabelObject], selected: &[usize], delta: Point, size: Point) -> Vec<LabelObject> {
    let chosen: Vec<&LabelObject> = selected.iter().filter_map(|&i| objects.get(i)).collect();
    if chosen.is_empty() {
        return objects.to_vec();
    }
    let min_x = chosen.iter().map(|o| o.bbox[0]).fold(f64::MAX, f64::min);
    let min_y = chosen.iter().map(|o| o.bbox[1]).fold(f64::MAX, f64::min);
    let max_x = chosen.iter().map(|o| o.bbox[0] + o.bbox[2]).fold(f64::MIN, f64::max);
    let max_y = chosen.iter().map(|o| o.bbox[1] + o.bbox[3]).fold(f64::MIN, f64::max);
    let dx = delta[0].clamp(-min_x, (size[0] - max_x).max(-min_x));
    let dy = delta[1].clamp(-min_y, (size[1] - max_y).max(-min_y));

    objects
        .iter()
        .enumerate()
        .map(|(i, o)| {
            if !selected.contains(&i) {
                return o.clone();
            }
            let b = o.bbox;
            LabelObject {
                class: o.class,
                bbox: [b[0] + dx, b[1] + dy, b[2], b[3]],
                polygon: o.polygon.as_ref().map(|points| points.iter().map(|[x, y]| [x + dx, y + dy]).collect()),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(x: f64, y: f64, w: f64, h: f64) -> LabelObject {
        LabelObject { class: 0, bbox: [x, y, w, h], polygon: None }
    }

    #[test]
    fn fits_and_zooms_around_a_point() {
        let view = View::fit([200.0, 100.0], [100.0, 100.0]);
        assert!((view.scale - 0.96).abs() < 1e-9);
        assert!((view.x - 52.0).abs() < 1e-9, "centred: {view:?}");
        let at = [80.0, 40.0];
        let before = view.to_image(at);
        let after = view.zoomed(2.0, at).to_image(at);
        assert!((before[0] - after[0]).abs() < 1e-9 && (before[1] - after[1]).abs() < 1e-9);
    }

    #[test]
    fn hits_the_smallest_box_and_handles() {
        let objects = vec![boxed(0.0, 0.0, 100.0, 100.0), boxed(10.0, 10.0, 20.0, 20.0)];
        assert_eq!(hit_object(&objects, [15.0, 15.0]), Some(1));
        assert_eq!(hit_object(&objects, [50.0, 50.0]), Some(0));
        assert_eq!(hit_object(&objects, [150.0, 50.0]), None);
        assert_eq!(hit_handle(&objects, &[1], [30.5, 29.0], 2.0), Some((1, Handle::SE)));
        assert_eq!(hit_handle(&objects, &[], [30.0, 30.0], 2.0), None, "only selected boxes");
    }

    #[test]
    fn draws_resizes_and_moves_inside_the_image() {
        let size = [100.0, 50.0];
        assert_eq!(rect_between([90.0, 40.0], [120.0, -5.0], size), [90.0, 0.0, 10.0, 40.0]);
        assert!(!big_enough([0.0, 0.0, 1.0, 5.0]));
        assert_eq!(resized([10.0, 10.0, 20.0, 20.0], Handle::E, [5.0, 99.0], size), [5.0, 10.0, 5.0, 20.0], "edges swap");
        assert_eq!(resized([10.0, 10.0, 20.0, 20.0], Handle::N, [0.0, 0.0], size), [10.0, 0.0, 20.0, 30.0]);

        let objects = vec![boxed(10.0, 10.0, 20.0, 20.0), boxed(60.0, 10.0, 10.0, 10.0)];
        let out = moved(&objects, &[0, 1], [50.0, -30.0], size);
        assert_eq!(out[1].bbox, [90.0, 0.0, 10.0, 10.0], "stops at the right and top edge");
        assert_eq!(out[0].bbox, [40.0, 0.0, 20.0, 20.0], "both move by the same amount");
    }
}
