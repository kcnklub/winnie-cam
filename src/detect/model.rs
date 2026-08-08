//! Loads a YOLOv8/YOLO11 ONNX model via `tract` and parses its raw output
//! into boxes.
//!
//! The model is expected to be exported without built-in NMS - see the
//! `yolo export` invocation in the README - so what tract hands back is one
//! flat `[1, CHANNELS, N]` plane per frame: `N` candidate boxes, each with 4
//! geometry values and 80 per-class scores, and no separate "objectness"
//! channel (that's a YOLOv5 thing, not v8/v11). This module's job stops at
//! "boxes in model-input-pixel space, filtered to one class" - letterboxing
//! them back to source-frame coordinates is [`crate::detect::letterbox`]'s
//! job, and suppressing overlapping duplicates is [`crate::detect::nms`]'s.

use std::path::Path;

use anyhow::Context;
use tract_onnx::prelude::*;

use crate::detect::nms::BBox;

/// COCO class index for "person". The only class this app cares about.
pub const PERSON_CLASS: usize = 0;

const NUM_CLASSES: usize = 80;
/// 4 box geometry values (cx, cy, w, h) + one score per COCO class.
const CHANNELS: usize = 4 + NUM_CLASSES;

pub type Plan = TypedRunnableModel<TypedModel>;

/// Loads the ONNX model at `path`, fixes its input to a `1x3xside*side`
/// tensor, and optimizes it into a runnable plan.
///
/// Fixing the input shape (rather than leaving it dynamic) is what lets
/// tract's optimizer specialize convolution kernels for the exact size
/// instead of emitting shape-generic code - it's a meaningful chunk of the
/// CPU-only performance budget on a Pi.
pub fn load(path: &Path, side: u32) -> anyhow::Result<Plan> {
    let side = side as usize;
    let model = tract_onnx::onnx()
        .model_for_path(path)
        .with_context(|| format!("reading ONNX model from {}", path.display()))?
        .with_input_fact(0, f32::fact([1, 3, side, side]).into())
        .context("fixing model input shape - does --detect-size match the export?")?
        .into_optimized()
        .context(
            "optimizing model graph - tract may not support an operator this model uses; \
             see the README's model export section for known-good export settings",
        )?
        .into_runnable()
        .context("compiling optimized model into a runnable plan")?;

    validate_output_shape(&model)?;
    Ok(model)
}

/// Fails loudly at load time rather than silently producing garbage boxes
/// later - the most likely way to hit this is `--detect-size` not matching
/// the size the model was exported with, or a `dynamic=True` export.
fn validate_output_shape(model: &Plan) -> anyhow::Result<()> {
    let fact = model
        .model()
        .output_fact(0)
        .context("reading model output shape")?;
    let shape = &fact.shape;
    anyhow::ensure!(
        shape.len() == 3 && shape[1] == CHANNELS.to_dim(),
        "unexpected model output shape {shape:?}; expected [1, {CHANNELS}, N] \
         (a YOLOv8/YOLO11 detection head exported without built-in NMS)"
    );
    Ok(())
}

/// Runs one inference pass on a filled input tensor, filtered to `class`
/// candidates above `threshold`. Boxes are in model-input pixel space
/// (0..side), corner form - callers map them back to source-frame fractions
/// with [`crate::detect::letterbox::Letterbox`].
pub fn infer(
    plan: &Plan,
    input: Tensor,
    class: usize,
    threshold: f32,
) -> anyhow::Result<Vec<BBox>> {
    let outputs = plan.run(tvec!(input.into())).context("running inference")?;
    let out = outputs[0]
        .as_slice::<f32>()
        .context("reading model output as f32")?;
    anyhow::ensure!(
        out.len() % CHANNELS == 0,
        "model output length {} is not a multiple of {CHANNELS} channels",
        out.len()
    );
    let n = out.len() / CHANNELS;
    Ok(parse_yolo_output(out, n, class, threshold))
}

/// Parses a flat `[1, CHANNELS, n]` YOLOv8/11 output plane (channel-major:
/// all `n` cx values, then all `n` cy values, etc.) into boxes in
/// model-input pixel coordinates (corner form), filtered to one class.
///
/// A plain slice in, plain `Vec` out, so this is testable without a model.
pub fn parse_yolo_output(out: &[f32], n: usize, class: usize, threshold: f32) -> Vec<BBox> {
    debug_assert_eq!(out.len(), CHANNELS * n);
    let mut boxes = Vec::new();
    for i in 0..n {
        let score = out[(4 + class) * n + i];
        if score < threshold {
            continue;
        }
        let (cx, cy) = (out[i], out[n + i]);
        let (w, h) = (out[2 * n + i], out[3 * n + i]);
        boxes.push(BBox {
            x1: cx - w / 2.0,
            y1: cy - h / 2.0,
            x2: cx + w / 2.0,
            y2: cy + h / 2.0,
            score,
        });
    }
    boxes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a synthetic `[1, CHANNELS, n]` plane with every value zeroed
    /// except what the caller sets, mirroring the `minimal_frame()` fixture
    /// idiom in `src/jpeg.rs`.
    fn plane(n: usize) -> Vec<f32> {
        vec![0.0; CHANNELS * n]
    }

    fn set_box(out: &mut [f32], n: usize, i: usize, cx: f32, cy: f32, w: f32, h: f32) {
        out[i] = cx;
        out[n + i] = cy;
        out[2 * n + i] = w;
        out[3 * n + i] = h;
    }

    fn set_class_score(out: &mut [f32], n: usize, i: usize, class: usize, score: f32) {
        out[(4 + class) * n + i] = score;
    }

    #[test]
    fn parses_a_single_high_confidence_person_box() {
        let n = 4;
        let mut out = plane(n);
        set_box(&mut out, n, 2, 100.0, 50.0, 40.0, 20.0);
        set_class_score(&mut out, n, 2, PERSON_CLASS, 0.9);

        let boxes = parse_yolo_output(&out, n, PERSON_CLASS, 0.35);

        assert_eq!(boxes.len(), 1);
        let b = boxes[0];
        assert_eq!((b.x1, b.y1, b.x2, b.y2), (80.0, 40.0, 120.0, 60.0));
        assert_eq!(b.score, 0.9);
    }

    #[test]
    fn ignores_boxes_below_the_threshold() {
        let n = 1;
        let mut out = plane(n);
        set_box(&mut out, n, 0, 10.0, 10.0, 5.0, 5.0);
        set_class_score(&mut out, n, 0, PERSON_CLASS, 0.2);

        assert!(parse_yolo_output(&out, n, PERSON_CLASS, 0.35).is_empty());
    }

    #[test]
    fn ignores_a_high_score_on_a_different_class() {
        let n = 1;
        let mut out = plane(n);
        set_box(&mut out, n, 0, 10.0, 10.0, 5.0, 5.0);
        // Class 16 = "dog" in COCO; person channel stays at 0.0.
        set_class_score(&mut out, n, 0, 16, 0.95);

        assert!(parse_yolo_output(&out, n, PERSON_CLASS, 0.35).is_empty());
    }

    #[test]
    fn converts_centre_form_to_corner_form() {
        let n = 1;
        let mut out = plane(n);
        set_box(&mut out, n, 0, 50.0, 60.0, 20.0, 10.0);
        set_class_score(&mut out, n, 0, PERSON_CLASS, 0.5);

        let b = parse_yolo_output(&out, n, PERSON_CLASS, 0.35)[0];
        assert_eq!((b.x1, b.y1, b.x2, b.y2), (40.0, 55.0, 60.0, 65.0));
    }

    /// The gate from the implementation plan's Step 0: confirms tract can
    /// actually load, optimize, and run the exported model before any of
    /// the rest of this feature is trusted. Point `WINNIE_CAM_TEST_MODEL`
    /// at a `yolo export ... format=onnx imgsz=320 opset=12 simplify=True
    /// dynamic=False` output and run with `cargo test --release -- --ignored`
    /// (release matters - tract is 30-100x slower in debug).
    #[test]
    #[ignore = "needs WINNIE_CAM_TEST_MODEL pointing at an exported ONNX model"]
    fn the_model_loads_optimizes_and_runs() {
        let path = std::env::var("WINNIE_CAM_TEST_MODEL")
            .expect("set WINNIE_CAM_TEST_MODEL to an exported .onnx path");
        let plan = load(Path::new(&path), 320).expect("model should load and optimize");

        let input = Tensor::zero::<f32>(&[1, 3, 320, 320]).unwrap();
        let boxes = infer(&plan, input, PERSON_CLASS, 0.99).expect("inference should run");
        // An all-zero (mid-grey after normalization) input isn't expected to
        // contain a confident person - this mainly proves the plumbing works
        // end to end without panicking or erroring.
        assert!(boxes.len() <= 2100, "sanity bound on candidate count");
    }
}
