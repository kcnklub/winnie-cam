//! Canvas overlay hook — ports `drawOverlay` / `syncCanvas` / `imageRect`
//! from the vanilla JS. Handles the `object-fit:contain` coordinate mapping,
//! `devicePixelRatio` scaling, resize/fullscreen observer redraws, and the
//! imperative Canvas 2D bounding-box + label drawing.

use leptos::html::{Canvas, Div, Img};
use leptos::prelude::*;
use shared_types::DetectionPayload;
use wasm_bindgen::prelude::Closure;
use wasm_bindgen::JsCast;

/// Painted-image rectangle in local coordinates of the stage element.
struct ImageRect {
    left: f64,
    top: f64,
    width: f64,
    height: f64,
}

/// CSS custom property values read from `:root` for drawing colors.
struct PaintTokens {
    ok: String,
    text: String,
}

/// Reactive canvas overlay. Runs an effect that redraws whenever detection
/// data changes, the window resizes, or fullscreen state changes.
pub fn use_overlay(
    canvas_ref: NodeRef<Canvas>,
    img_ref: NodeRef<Img>,
    stage_ref: NodeRef<Div>,
    detection: ReadSignal<Option<DetectionPayload>>,
) {
    // A counter signal that increments on resize/fullscreen, so the single
    // drawing effect re-runs without needing to call draw imperatively from
    // every listener.
    let (redraw_version, set_redraw_version) = signal(0u64);

    // ── Coordinate mapping (object-fit:contain) ──────────────────────

    fn image_rect(
        img: &web_sys::HtmlImageElement,
        stage: &web_sys::HtmlDivElement,
    ) -> Option<ImageRect> {
        let nw = img.natural_width() as f64;
        let nh = img.natural_height() as f64;
        if nw == 0.0 || nh == 0.0 {
            return None;
        }

        let r = img.get_bounding_client_rect();
        let sr = stage.get_bounding_client_rect();
        if r.width() == 0.0 || r.height() == 0.0 {
            return None;
        }

        let scale = f64::min(r.width() / nw, r.height() / nh);
        let dw = nw * scale;
        let dh = nh * scale;

        Some(ImageRect {
            left: (r.left() - sr.left() - stage.client_left() as f64)
                + (r.width() - dw) / 2.0,
            top: (r.top() - sr.top() - stage.client_top() as f64)
                + (r.height() - dh) / 2.0,
            width: dw,
            height: dh,
        })
    }

    // ── Canvas positioning (devicePixelRatio) ────────────────────────

    fn sync_canvas(
        canvas: &web_sys::HtmlCanvasElement,
        rect: &ImageRect,
    ) -> Option<()> {
        let dpr = window().device_pixel_ratio();

        // Access the native `style` property through `HtmlElement` to avoid
        // tachys's `style()` setter shadowing it.
        let html_el: &web_sys::HtmlElement = canvas.unchecked_ref();
        let style = html_el.style();
        let _ = style.set_property("left", &format!("{}px", rect.left));
        let _ = style.set_property("top", &format!("{}px", rect.top));
        let _ =
            style.set_property("width", &format!("{}px", rect.width));
        let _ =
            style.set_property("height", &format!("{}px", rect.height));

        let bw = (rect.width * dpr).round() as u32;
        let bh = (rect.height * dpr).round() as u32;
        if canvas.width() != bw || canvas.height() != bh {
            canvas.set_width(bw);
            canvas.set_height(bh);
        }

        let ctx = canvas
            .get_context("2d")
            .ok()
            .flatten()
            .and_then(|c| {
                c.dyn_into::<web_sys::CanvasRenderingContext2d>().ok()
            })?;

        let _ = ctx.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0);
        canvas.set_hidden(false);
        Some(())
    }

    // ── Paint tokens ─────────────────────────────────────────────────

    fn read_paint_tokens() -> PaintTokens {
        let doc_el = window()
            .document()
            .and_then(|d| d.document_element());

        let cs = doc_el
            .as_ref()
            .and_then(|el| {
                let element: &web_sys::Element = el.unchecked_ref();
                window()
                    .get_computed_style(element)
                    .ok()
                    .flatten()
            });

        let read = |name: &str| -> String {
            cs.as_ref()
                .and_then(|s| s.get_property_value(name).ok())
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| match name {
                    "--ok" => "#8fbf9f".into(),
                    _ => "#efe6dd".into(),
                })
        };

        PaintTokens {
            ok: read("--ok"),
            text: read("--text"),
        }
    }

    // ── Drawing ──────────────────────────────────────────────────────

    fn draw_detections(
        canvas: &web_sys::HtmlCanvasElement,
        payload: &DetectionPayload,
        rect: &ImageRect,
    ) {
        let ctx = canvas
            .get_context("2d")
            .ok()
            .flatten()
            .and_then(|c| {
                c.dyn_into::<web_sys::CanvasRenderingContext2d>().ok()
            });

        let Some(ctx) = ctx else { return };

        ctx.clear_rect(0.0, 0.0, rect.width, rect.height);

        if payload.dets.is_empty() {
            return;
        }

        let tokens = read_paint_tokens();
        let has_round = js_sys::Reflect::has(
            &ctx,
            &wasm_bindgen::JsValue::from_str("roundRect"),
        )
        .unwrap_or(false);

        ctx.set_line_width(2.0);
        ctx.set_stroke_style_str(&tokens.ok);
        ctx.set_font("12px system-ui, sans-serif");
        ctx.set_text_baseline("top");

        for d in &payload.dets {
            let x = d.x as f64 * rect.width;
            let y = d.y as f64 * rect.height;
            let w = d.w as f64 * rect.width;
            let h = d.h as f64 * rect.height;

            // Bounding box stroke.
            ctx.begin_path();
            if has_round {
                let _ = call_round_rect(&ctx, x, y, w, h, 6.0);
            } else {
                ctx.rect(x, y, w, h);
            }
            ctx.stroke();

            // Label pill.
            let text = format!(
                "{} {}%",
                d.label,
                (d.score * 100.0).round() as u32
            );
            let text_width = measure_text_width(&ctx, &text);
            let ty = f64::max(0.0, y - 18.0);

            // Dark pill background.
            ctx.set_fill_style_str("rgba(0,0,0,0.55)");
            ctx.begin_path();
            if has_round {
                let _ = call_round_rect(
                    &ctx, x, ty, text_width + 8.0, 16.0, 4.0,
                );
            } else {
                ctx.rect(x, ty, text_width + 8.0, 16.0);
            }
            ctx.fill();

            // Label text.
            ctx.set_fill_style_str(&tokens.text);
            let _ = ctx.fill_text(&text, x + 4.0, ty + 2.0);
        }
    }

    fn call_round_rect(
        ctx: &web_sys::CanvasRenderingContext2d,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        r: f64,
    ) -> Result<(), wasm_bindgen::JsValue> {
        let args = js_sys::Array::of5(
            &wasm_bindgen::JsValue::from_f64(x),
            &wasm_bindgen::JsValue::from_f64(y),
            &wasm_bindgen::JsValue::from_f64(w),
            &wasm_bindgen::JsValue::from_f64(h),
            &wasm_bindgen::JsValue::from_f64(r),
        );
        js_sys::Reflect::apply(
            &js_sys::Reflect::get(
                ctx,
                &wasm_bindgen::JsValue::from_str("roundRect"),
            )?
            .dyn_into::<js_sys::Function>()?,
            ctx,
            &args,
        )
        .map(|_| ())
    }

    fn measure_text_width(
        ctx: &web_sys::CanvasRenderingContext2d,
        text: &str,
    ) -> f64 {
        let result = js_sys::Reflect::apply(
            &js_sys::Reflect::get(
                ctx,
                &wasm_bindgen::JsValue::from_str("measureText"),
            )
            .and_then(|v| v.dyn_into::<js_sys::Function>())
            .unwrap_or_default(),
            ctx,
            &js_sys::Array::of1(&wasm_bindgen::JsValue::from_str(text)),
        );

        result
            .ok()
            .and_then(|metrics| {
                js_sys::Reflect::get(
                    &metrics,
                    &wasm_bindgen::JsValue::from_str("width"),
                )
                .ok()
            })
            .and_then(|w| w.as_f64())
            .unwrap_or(0.0)
    }

    // ── Main drawing effect ──────────────────────────────────────────

    Effect::new(move |_| {
        // Read dependencies so Leptos tracks them.
        let _version = redraw_version.get();
        let payload_opt = detection.get();

        let Some(canvas) = canvas_ref.get() else { return };
        let Some(img) = img_ref.get() else { return };
        let Some(stage) = stage_ref.get() else { return };

        let Some(rect) = image_rect(&img, &stage) else {
            canvas.set_hidden(true);
            return;
        };

        if sync_canvas(&canvas, &rect).is_none() {
            return;
        }

        if let Some(ref payload) = payload_opt {
            draw_detections(&canvas, payload, &rect);
        } else {
            if let Some(ctx) = canvas
                .get_context("2d")
                .ok()
                .flatten()
                .and_then(|c| {
                    c.dyn_into::<web_sys::CanvasRenderingContext2d>().ok()
                })
            {
                ctx.clear_rect(0.0, 0.0, rect.width, rect.height);
            }
        }
    });

    // ── ResizeObserver on the stage ──────────────────────────────────

    let _resize_observer = {
        let set_redraw_version = set_redraw_version;
        let stage_ref = stage_ref;

        let cb = Closure::wrap(
            Box::new(
                move |_entries: js_sys::Array,
                      _observer: web_sys::ResizeObserver| {
                    set_redraw_version
                        .update(|v| *v = v.wrapping_add(1));
                },
            ) as Box<dyn FnMut(js_sys::Array, web_sys::ResizeObserver)>,
        );

        let observer =
            web_sys::ResizeObserver::new(cb.as_ref().unchecked_ref())
                .expect("ResizeObserver constructor should work");

        // Observe once the stage element is available.
        Effect::new({
            let stage_ref = stage_ref;
            let observer = observer.clone();
            move |prev: Option<()>| {
                if prev.is_none() {
                    if let Some(stage) = stage_ref.get() {
                        let el: &web_sys::Element = stage.unchecked_ref();
                        observer.observe(el);
                    }
                }
            }
        });

        cb.forget();

        on_cleanup(move || {
            observer.disconnect();
        });
    };

    // ── Window resize ────────────────────────────────────────────────

    let _resize_listener = {
        let cb = Closure::wrap(
            Box::new(move || {
                set_redraw_version.update(|v| *v = v.wrapping_add(1));
            }) as Box<dyn FnMut()>,
        );

        window()
            .add_event_listener_with_callback(
                "resize",
                cb.as_ref().unchecked_ref(),
            )
            .expect("resize listener should register");

        cb.forget();
    };

    // ── Fullscreen change ────────────────────────────────────────────

    let _fullscreen_listener = {
        let cb = Closure::wrap(
            Box::new(move || {
                set_redraw_version.update(|v| *v = v.wrapping_add(1));
            }) as Box<dyn FnMut()>,
        );

        let doc = document();
        doc.add_event_listener_with_callback(
            "fullscreenchange",
            cb.as_ref().unchecked_ref(),
        )
        .expect("fullscreenchange listener should register");
        doc.add_event_listener_with_callback(
            "webkitfullscreenchange",
            cb.as_ref().unchecked_ref(),
        )
        .expect("webkitfullscreenchange listener should register");

        cb.forget();
    };
}

// ── Helpers ────────────────────────────────────────────────────────────

fn window() -> web_sys::Window {
    web_sys::window().expect("no window in browser")
}

fn document() -> web_sys::Document {
    window().document().expect("no document")
}