//! The actual motion math: downsample a decoded RGB frame to a small gray
//! grid, mask that grid to the region a person box covers, and score how
//! much of the masked region changed since the previous sample.
//!
//! Everything here is a pure function over plain slices - no `Instant`, no
//! channels - so it's straightforward to unit test and cheap to run at
//! `--motion-fps` on a Pi.

use crate::detect::nms::BBox;

/// Downsamples `rgb` (a `w`x`h` interleaved RGB buffer) into a `side`x`side`
/// grayscale grid, one byte per cell. Each cell is the box-filtered average
/// luma of every source pixel that falls inside it - averaging rather than
/// point-sampling means sensor noise mostly cancels out instead of flipping
/// individual cells from frame to frame.
pub fn downscale(rgb: &[u8], w: u32, h: u32, side: u32, out: &mut Vec<u8>) {
    let side = side.max(1);
    let w = w.max(1);
    let h = h.max(1);
    out.clear();
    out.resize((side * side) as usize, 0);

    for gy in 0..side {
        let y0 = gy * h / side;
        let y1 = ((gy + 1) * h / side).max(y0 + 1).min(h);
        for gx in 0..side {
            let x0 = gx * w / side;
            let x1 = ((gx + 1) * w / side).max(x0 + 1).min(w);

            let mut weighted_sum: u64 = 0;
            let mut weight: u64 = 0;
            for y in y0..y1 {
                let row = (y as usize) * (w as usize) * 3;
                for x in x0..x1 {
                    let idx = row + (x as usize) * 3;
                    if idx + 2 >= rgb.len() {
                        continue;
                    }
                    let (r, g, b) = (rgb[idx] as u64, rgb[idx + 1] as u64, rgb[idx + 2] as u64);
                    // Rec. 601 luma weights, scaled by 1000 to stay in
                    // integer math.
                    weighted_sum += r * 299 + g * 587 + b * 114;
                    weight += 1000;
                }
            }
            let gray = weighted_sum.checked_div(weight).unwrap_or(0) as u8;
            out[(gy * side + gx) as usize] = gray;
        }
    }
}

/// Builds a `side`x`side` boolean mask marking every cell touched by any of
/// `boxes` (source-frame pixel coordinates), each dilated by `margin` - a
/// fraction of that box's own width/height - before rasterizing. Dilating
/// lets a limb moving just outside the last known box still count, since the
/// box is only as fresh as the last detection pass (see
/// `motion::MotionInput`).
pub fn mask(boxes: &[BBox], src_w: u32, src_h: u32, side: u32, margin: f32) -> Vec<bool> {
    let side = side.max(1) as usize;
    let mut out = vec![false; side * side];
    if src_w == 0 || src_h == 0 {
        return out;
    }
    let cell_w = src_w as f32 / side as f32;
    let cell_h = src_h as f32 / side as f32;

    for b in boxes {
        let dx = (b.x2 - b.x1).max(0.0) * margin.max(0.0);
        let dy = (b.y2 - b.y1).max(0.0) * margin.max(0.0);
        let x1 = (b.x1 - dx).max(0.0);
        let y1 = (b.y1 - dy).max(0.0);
        let x2 = (b.x2 + dx).min(src_w as f32);
        let y2 = (b.y2 + dy).min(src_h as f32);
        if x2 <= x1 || y2 <= y1 {
            continue;
        }

        for gy in 0..side {
            let cy0 = gy as f32 * cell_h;
            let cy1 = cy0 + cell_h;
            if cy1 <= y1 || cy0 >= y2 {
                continue;
            }
            for gx in 0..side {
                let cx0 = gx as f32 * cell_w;
                let cx1 = cx0 + cell_w;
                if cx1 <= x1 || cx0 >= x2 {
                    continue;
                }
                out[gy * side + gx] = true;
            }
        }
    }
    out
}

/// Fraction of changed cells, split by whether the cell is inside the
/// person mask or not. The `outside` half is what lets the caller tell
/// "the baby moved" apart from "the whole room got brighter" - see
/// `motion::mod`'s global-lighting guard.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Score {
    pub inside: f32,
    pub outside: f32,
}

/// Compares two same-sized grids cell-by-cell against `mask`. A cell counts
/// as changed when its gray value moved by at least `pixel_delta`. Returns
/// `None` if `mask` has no cells set - there's no subject to score motion
/// against, so a caller shouldn't average zero cells into a number that
/// looks meaningful.
pub fn score(prev: &[u8], cur: &[u8], mask: &[bool], pixel_delta: u8) -> Option<Score> {
    let n = prev.len().min(cur.len()).min(mask.len());
    let (mut inside_total, mut inside_changed) = (0usize, 0usize);
    let (mut outside_total, mut outside_changed) = (0usize, 0usize);

    for i in 0..n {
        let delta = (prev[i] as i16 - cur[i] as i16).unsigned_abs();
        let changed = delta >= pixel_delta as u16;
        if mask[i] {
            inside_total += 1;
            if changed {
                inside_changed += 1;
            }
        } else {
            outside_total += 1;
            if changed {
                outside_changed += 1;
            }
        }
    }

    if inside_total == 0 {
        return None;
    }
    Some(Score {
        inside: inside_changed as f32 / inside_total as f32,
        outside: if outside_total == 0 {
            0.0
        } else {
            outside_changed as f32 / outside_total as f32
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_rgb(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
        let mut out = Vec::with_capacity((w * h * 3) as usize);
        for _ in 0..(w * h) {
            out.extend_from_slice(&rgb);
        }
        out
    }

    fn bbox(x1: f32, y1: f32, x2: f32, y2: f32) -> BBox {
        BBox { x1, y1, x2, y2, score: 1.0 }
    }

    #[test]
    fn downscale_of_a_solid_frame_is_uniform() {
        let rgb = solid_rgb(16, 16, [100, 150, 200]);
        let mut out = Vec::new();
        downscale(&rgb, 16, 16, 4, &mut out);
        assert_eq!(out.len(), 16);
        let expected = (100u64 * 299 + 150 * 587 + 200 * 114) / 1000;
        assert!(out.iter().all(|&v| v as u64 == expected));
    }

    #[test]
    fn identical_frames_score_zero_everywhere() {
        let grid = vec![128u8; 16];
        let mask = vec![true; 16];
        let s = score(&grid, &grid, &mask, 12).unwrap();
        assert_eq!(s, Score { inside: 0.0, outside: 0.0 });
    }

    #[test]
    fn a_change_inside_the_mask_scores_its_exact_fraction() {
        // 4x4 grid, mask on cells 0..4 only (the top row). Change 2 of
        // those 4 cells beyond the pixel_delta threshold.
        let side = 4;
        let mut mask = vec![false; side * side];
        for m in mask.iter_mut().take(side) {
            *m = true;
        }
        let prev = vec![100u8; side * side];
        let mut cur = prev.clone();
        cur[0] = 200;
        cur[1] = 200;
        let s = score(&prev, &cur, &mask, 12).unwrap();
        assert_eq!(s.inside, 0.5); // 2 of 4 masked cells changed
        assert_eq!(s.outside, 0.0);
    }

    #[test]
    fn a_change_entirely_outside_the_mask_scores_zero_inside() {
        let side = 4;
        let mut mask = vec![false; side * side];
        mask[0] = true; // only cell 0 is "inside"
        let prev = vec![100u8; side * side];
        let mut cur = prev.clone();
        cur[side * side - 1] = 250; // change the last cell, which is outside
        let s = score(&prev, &cur, &mask, 12).unwrap();
        assert_eq!(s.inside, 0.0);
        assert!(s.outside > 0.0);
    }

    #[test]
    fn an_empty_mask_returns_none() {
        let grid = vec![100u8; 16];
        let mask = vec![false; 16];
        assert_eq!(score(&grid, &grid, &mask, 12), None);
    }

    #[test]
    fn a_uniform_brightness_shift_raises_outside_as_much_as_inside() {
        // A global exposure/IR-cut change lights up the whole frame, not
        // just the masked region - `outside` should track `inside` closely,
        // which is the signal the caller's lighting guard checks for.
        let side = 4;
        let mut mask = vec![false; side * side];
        for m in mask.iter_mut().take(side * side / 2) {
            *m = true;
        }
        let prev = vec![100u8; side * side];
        let cur = vec![140u8; side * side]; // every cell brightened
        let s = score(&prev, &cur, &mask, 12).unwrap();
        assert_eq!(s.inside, 1.0);
        assert_eq!(s.outside, 1.0);
    }

    #[test]
    fn mask_dilates_by_margin_before_rasterizing() {
        // A tight box in the exact center of a 100x100 frame, 10x10 grid
        // (10px per cell). With 0 margin it should cover roughly the cells
        // under x=[40,60), y=[40,60) -> cells 4 and 5. A large margin should
        // cover strictly more cells.
        let boxes = [bbox(45.0, 45.0, 55.0, 55.0)];
        let tight = mask(&boxes, 100, 100, 10, 0.0);
        let dilated = mask(&boxes, 100, 100, 10, 1.0);
        let tight_count = tight.iter().filter(|&&v| v).count();
        let dilated_count = dilated.iter().filter(|&&v| v).count();
        assert!(
            dilated_count > tight_count,
            "dilated ({dilated_count}) should cover more cells than tight ({tight_count})"
        );
        // Every tight cell must still be covered once dilated.
        for i in 0..tight.len() {
            if tight[i] {
                assert!(dilated[i], "cell {i} was covered tight but not dilated");
            }
        }
    }

    #[test]
    fn mask_with_no_boxes_is_empty() {
        let m = mask(&[], 100, 100, 10, 0.1);
        assert!(m.iter().all(|&v| !v));
    }
}
