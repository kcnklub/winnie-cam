//! Maps between a captured frame's pixels and a detection model's square,
//! aspect-ratio-preserving "letterboxed" input, and does the resize +
//! normalize + HWC->CHW conversion the model expects.
//!
//! Ultralytics letterboxes with grey (114,114,114) padding when training,
//! so this pads with the same constant - a model trained on grey bars sees
//! black bars as data, not padding.

/// Padding value (0-255) used for the letterbox bars, matching Ultralytics'
/// training-time preprocessing.
pub const PAD_VALUE: u8 = 114;

/// The forward (source -> model) and inverse (model -> source) mapping for
/// one source resolution letterboxed into one square model input size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Letterbox {
    pub src_w: u32,
    pub src_h: u32,
    pub side: u32,
    /// Source pixels -> model pixels.
    pub scale: f32,
    pub new_w: u32,
    pub new_h: u32,
    /// Left/top padding, in model pixels.
    pub pad_x: f32,
    pub pad_y: f32,
}

impl Letterbox {
    pub fn new(src_w: u32, src_h: u32, side: u32) -> Self {
        let scale = (side as f32 / src_w as f32).min(side as f32 / src_h as f32);
        // floor, not round: rounding up can push new_w/new_h past `side`.
        let new_w = (src_w as f32 * scale).floor().max(1.0) as u32;
        let new_h = (src_h as f32 * scale).floor().max(1.0) as u32;
        Self {
            src_w,
            src_h,
            side,
            scale,
            new_w,
            new_h,
            pad_x: (side.saturating_sub(new_w)) as f32 / 2.0,
            pad_y: (side.saturating_sub(new_h)) as f32 / 2.0,
        }
    }

    /// Source pixel -> model-input pixel. The runtime pipeline never needs
    /// this direction (detection only ever maps model output *back* to
    /// source coordinates), but it's `to_source`'s exact counterpart and
    /// having it lets the round-trip test below actually prove the inverse
    /// is correct instead of just asserting isolated points.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn to_model(&self, x: f32, y: f32) -> (f32, f32) {
        (x * self.scale + self.pad_x, y * self.scale + self.pad_y)
    }

    /// Model-input pixel -> source pixel. Exact inverse of [`Self::to_model`];
    /// may return values outside the source frame for points that fall in
    /// the pad band. Clamping is [`Self::to_fraction`]'s job, not this one's.
    pub fn to_source(&self, x: f32, y: f32) -> (f32, f32) {
        ((x - self.pad_x) / self.scale, (y - self.pad_y) / self.scale)
    }

    /// Model-input pixel -> 0..1 fraction of the source frame, clamped.
    /// Not currently called from the runtime pipeline - `detect::hub`
    /// normalizes from source-frame *pixels* instead, with its own
    /// NaN-guard tailored to the JSON it's writing - but it's the natural
    /// counterpart to `to_source` and is covered directly by tests below.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn to_fraction(&self, x: f32, y: f32) -> (f32, f32) {
        let (sx, sy) = self.to_source(x, y);
        (
            (sx / self.src_w as f32).clamp(0.0, 1.0),
            (sy / self.src_h as f32).clamp(0.0, 1.0),
        )
    }

    /// Bilinear-samples `rgb` (HWC, u8, `src_w * src_h * 3` bytes) into
    /// `dst` (NCHW f32, len `3 * side * side`), letterboxed and normalized
    /// to 0..1. `dst` is only ever written to, never reallocated, so a
    /// caller can reuse the same buffer across frames.
    pub fn fill_tensor(&self, rgb: &[u8], dst: &mut [f32]) {
        let side = self.side as usize;
        debug_assert_eq!(rgb.len(), self.src_w as usize * self.src_h as usize * 3);
        debug_assert_eq!(dst.len(), 3 * side * side);

        let pad_val = PAD_VALUE as f32 / 255.0;
        dst.fill(pad_val);

        if self.new_w == 0 || self.new_h == 0 {
            return;
        }

        // Inverse scale: for each destination pixel inside the resized
        // (non-pad) region, find the source pixel it samples from.
        let inv_scale = 1.0 / self.scale;
        let x0 = self.pad_x.round() as usize;
        let y0 = self.pad_y.round() as usize;

        for dy in 0..self.new_h as usize {
            let sy = ((dy as f32 + 0.5) * inv_scale - 0.5).clamp(0.0, self.src_h as f32 - 1.0);
            let y0f = sy.floor();
            let y1f = (y0f + 1.0).min(self.src_h as f32 - 1.0);
            let wy = sy - y0f;

            for dx in 0..self.new_w as usize {
                let sx =
                    ((dx as f32 + 0.5) * inv_scale - 0.5).clamp(0.0, self.src_w as f32 - 1.0);
                let x0f = sx.floor();
                let x1f = (x0f + 1.0).min(self.src_w as f32 - 1.0);
                let wx = sx - x0f;

                let (x0i, x1i, y0i, y1i) =
                    (x0f as usize, x1f as usize, y0f as usize, y1f as usize);

                let px_out = x0 + dx;
                let py_out = y0 + dy;
                if px_out >= side || py_out >= side {
                    continue;
                }

                for c in 0..3 {
                    let p = |xi: usize, yi: usize| -> f32 {
                        rgb[(yi * self.src_w as usize + xi) * 3 + c] as f32
                    };
                    let top = p(x0i, y0i) * (1.0 - wx) + p(x1i, y0i) * wx;
                    let bot = p(x0i, y1i) * (1.0 - wx) + p(x1i, y1i) * wx;
                    let v = (top * (1.0 - wy) + bot * wy) / 255.0;
                    dst[c * side * side + py_out * side + px_out] = v;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn landscape_source_pads_vertically_only() {
        let lb = Letterbox::new(1280, 720, 320);
        assert_eq!(lb.scale, 0.25);
        assert_eq!((lb.new_w, lb.new_h), (320, 180));
        assert_eq!((lb.pad_x, lb.pad_y), (0.0, 70.0));
    }

    #[test]
    fn portrait_source_pads_horizontally_only() {
        let lb = Letterbox::new(720, 1280, 320);
        assert_eq!(lb.scale, 0.25);
        assert_eq!((lb.new_w, lb.new_h), (180, 320));
        assert_eq!((lb.pad_x, lb.pad_y), (70.0, 0.0));
    }

    #[test]
    fn square_source_has_no_padding() {
        let lb = Letterbox::new(320, 320, 320);
        assert_eq!(lb.scale, 1.0);
        assert_eq!((lb.new_w, lb.new_h), (320, 320));
        assert_eq!((lb.pad_x, lb.pad_y), (0.0, 0.0));
    }

    #[test]
    fn model_to_source_round_trips() {
        let lb = Letterbox::new(1280, 720, 320);
        for &(x, y) in &[(0.0, 70.0), (320.0, 250.0), (160.0, 160.0), (10.0, 80.0)] {
            let (sx, sy) = lb.to_source(x, y);
            let (mx, my) = lb.to_model(sx, sy);
            assert!((mx - x).abs() < 1e-3, "x round trip: {mx} vs {x}");
            assert!((my - y).abs() < 1e-3, "y round trip: {my} vs {y}");
        }
    }

    #[test]
    fn frame_corners_map_to_zero_and_one() {
        let lb = Letterbox::new(1280, 720, 320);
        // Model-y 70 is the top of the resized image (source y=0); model-y
        // 250 (70 + 180) is the bottom (source y=720).
        assert_eq!(lb.to_fraction(0.0, 70.0), (0.0, 0.0));
        assert_eq!(lb.to_fraction(320.0, 250.0), (1.0, 1.0));
    }

    #[test]
    fn points_in_the_pad_band_map_outside_the_source() {
        let lb = Letterbox::new(1280, 720, 320);
        let (_, sy) = lb.to_source(0.0, 0.0); // above the pad band
        assert!(sy < 0.0);
        // to_fraction must still clamp it into range rather than propagate
        // the out-of-bounds value.
        assert_eq!(lb.to_fraction(0.0, 0.0), (0.0, 0.0));
    }

    #[test]
    fn fill_tensor_pads_with_the_training_time_grey() {
        let lb = Letterbox::new(1280, 720, 320);
        let rgb = vec![0u8; 1280 * 720 * 3];
        let mut dst = vec![-1.0f32; 3 * 320 * 320];
        lb.fill_tensor(&rgb, &mut dst);

        let expected_pad = PAD_VALUE as f32 / 255.0;
        // Top-left corner of the R channel plane is in the pad band
        // (pad_y = 70), so it must be the pad value, not the black source.
        assert_eq!(dst[0], expected_pad);
    }

    #[test]
    fn fill_tensor_places_a_solid_color_frame_in_the_resized_region() {
        let lb = Letterbox::new(1280, 720, 320);
        // Solid red source frame.
        let mut rgb = vec![0u8; 1280 * 720 * 3];
        for px in rgb.chunks_exact_mut(3) {
            px[0] = 255;
        }
        let mut dst = vec![0.0f32; 3 * 320 * 320];
        lb.fill_tensor(&rgb, &mut dst);

        let side = 320usize;
        // Centre of the resized region (pad_y=70, new_h=180 -> centre row 160).
        let (cx, cy) = (160usize, 160usize);
        let r = dst[0 * side * side + cy * side + cx];
        let g = dst[1 * side * side + cy * side + cx];
        assert!((r - 1.0).abs() < 1e-6, "expected full-intensity red, got {r}");
        assert_eq!(g, 0.0);
    }
}
