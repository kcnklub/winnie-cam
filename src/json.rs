//! Tiny helpers shared by every hand-rolled JSON serializer in this crate
//! (`detect::hub`, `motion::hub`, `web::healthz`). This project deliberately
//! doesn't pull in `serde` for payloads this small - see the doc comment on
//! `detect::hub::detections_json` for the rationale.

/// Formats a 0..1 fraction safely. JSON has no NaN/Infinity literal, and
/// `format!("{:.4}", f32::NAN)` prints the bare word `NaN` - one degenerate
/// value would break `JSON.parse` for every viewer, so every fraction that
/// reaches the wire goes through this first.
pub fn frac(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}

/// Clamps a single pixel coordinate into `0..=extent` (a frame's width or
/// height), non-finite values pinned to 0.
pub fn clamp_coord(v: f32, extent: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, extent) } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frac_clamps_into_zero_to_one() {
        assert_eq!(frac(0.5), 0.5);
        assert_eq!(frac(-1.0), 0.0);
        assert_eq!(frac(2.0), 1.0);
    }

    #[test]
    fn frac_maps_non_finite_to_zero() {
        assert_eq!(frac(f32::NAN), 0.0);
        assert_eq!(frac(f32::INFINITY), 0.0);
        assert_eq!(frac(f32::NEG_INFINITY), 0.0);
    }

    #[test]
    fn clamp_coord_clamps_into_the_extent() {
        assert_eq!(clamp_coord(-5.0, 100.0), 0.0);
        assert_eq!(clamp_coord(150.0, 100.0), 100.0);
        assert_eq!(clamp_coord(50.0, 100.0), 50.0);
    }

    #[test]
    fn clamp_coord_maps_non_finite_to_zero() {
        assert_eq!(clamp_coord(f32::NAN, 100.0), 0.0);
        assert_eq!(clamp_coord(f32::INFINITY, 100.0), 0.0);
    }
}
