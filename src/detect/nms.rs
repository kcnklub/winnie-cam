//! Greedy IoU non-maximum suppression: a detection head proposes many
//! overlapping boxes for the same real object, and this collapses them down
//! to one per object, keeping the highest-confidence box in each cluster.

/// A candidate detection in corner form, in whatever coordinate space the
/// caller is working in (model-input pixels, here).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BBox {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub score: f32,
}

impl BBox {
    pub fn area(&self) -> f32 {
        (self.x2 - self.x1).max(0.0) * (self.y2 - self.y1).max(0.0)
    }
}

/// Intersection-over-union of two boxes, in `[0, 1]`.
pub fn iou(a: &BBox, b: &BBox) -> f32 {
    let ix = (a.x2.min(b.x2) - a.x1.max(b.x1)).max(0.0);
    let iy = (a.y2.min(b.y2) - a.y1.max(b.y1)).max(0.0);
    let inter = ix * iy;
    let union = a.area() + b.area() - inter;
    if union <= 0.0 { 0.0 } else { inter / union }
}

/// Greedy IoU non-maximum suppression. `boxes` need not be sorted going in.
/// Keeps at most `max_out` boxes, highest score first.
pub fn nms(mut boxes: Vec<BBox>, iou_thresh: f32, max_out: usize) -> Vec<BBox> {
    // total_cmp, not partial_cmp().unwrap(): a NaN score (shouldn't happen,
    // but inference output is not something to trust blindly) must not
    // panic the sort.
    boxes.sort_unstable_by(|a, b| b.score.total_cmp(&a.score));

    let mut kept: Vec<BBox> = Vec::new();
    for cand in boxes {
        if kept.len() >= max_out {
            break;
        }
        if kept.iter().all(|k| iou(k, &cand) <= iou_thresh) {
            kept.push(cand);
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bbox(x1: f32, y1: f32, x2: f32, y2: f32, score: f32) -> BBox {
        BBox { x1, y1, x2, y2, score }
    }

    #[test]
    fn iou_of_a_box_with_itself_is_one() {
        let b = bbox(0.0, 0.0, 10.0, 10.0, 0.9);
        assert_eq!(iou(&b, &b), 1.0);
    }

    #[test]
    fn iou_of_disjoint_boxes_is_zero() {
        let a = bbox(0.0, 0.0, 1.0, 1.0, 0.9);
        let b = bbox(5.0, 5.0, 6.0, 6.0, 0.9);
        assert_eq!(iou(&a, &b), 0.0);
    }

    #[test]
    fn iou_of_half_overlapping_unit_boxes_is_one_third() {
        // Two 1x1 boxes offset by 0.5 in x: intersection 0.5x1=0.5,
        // union = 1 + 1 - 0.5 = 1.5, iou = 0.5/1.5 = 1/3.
        let a = bbox(0.0, 0.0, 1.0, 1.0, 0.9);
        let b = bbox(0.5, 0.0, 1.5, 1.0, 0.9);
        assert!((iou(&a, &b) - (1.0 / 3.0)).abs() < 1e-6);
    }

    #[test]
    fn duplicate_boxes_collapse_to_the_highest_score() {
        let boxes = vec![
            bbox(0.0, 0.0, 10.0, 10.0, 0.6),
            bbox(0.5, 0.5, 10.5, 10.5, 0.95),
            bbox(0.2, 0.2, 10.2, 10.2, 0.7),
        ];
        let kept = nms(boxes, 0.5, 10);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].score, 0.95);
    }

    #[test]
    fn disjoint_boxes_all_survive() {
        let boxes = vec![
            bbox(0.0, 0.0, 1.0, 1.0, 0.9),
            bbox(10.0, 10.0, 11.0, 11.0, 0.8),
            bbox(20.0, 20.0, 21.0, 21.0, 0.7),
        ];
        assert_eq!(nms(boxes, 0.5, 10).len(), 3);
    }

    #[test]
    fn overlap_below_the_threshold_is_kept() {
        // iou here is 1/3 (see the test above). A 0.5 threshold tolerates
        // more overlap than that, so both boxes survive.
        let boxes = vec![
            bbox(0.0, 0.0, 1.0, 1.0, 0.9),
            bbox(0.5, 0.0, 1.5, 1.0, 0.8),
        ];
        assert_eq!(nms(boxes, 0.5, 10).len(), 2);
    }

    #[test]
    fn overlap_above_the_threshold_is_suppressed() {
        // Same pair, same 1/3 iou - a 0.1 threshold is stricter than the
        // overlap, so only the higher-scoring box should survive.
        let boxes = vec![
            bbox(0.0, 0.0, 1.0, 1.0, 0.9),
            bbox(0.5, 0.0, 1.5, 1.0, 0.8),
        ];
        let kept = nms(boxes, 0.1, 10);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].score, 0.9);
    }

    #[test]
    fn respects_max_out() {
        let boxes = vec![
            bbox(0.0, 0.0, 1.0, 1.0, 0.9),
            bbox(10.0, 10.0, 11.0, 11.0, 0.8),
            bbox(20.0, 20.0, 21.0, 21.0, 0.7),
        ];
        assert_eq!(nms(boxes, 0.5, 2).len(), 2);
    }
}
