//! `CXL_FRAMESTATS=1`: measure the PLAN §7 frame budget ("one frame with glass
//! under 4 ms on the iGPU"). Brackets the whole egui paint with a GL
//! TIME_ELAPSED query (first callback on the background layer, last on the
//! Debug layer), reads the result one frame late so the GPU never stalls, and
//! prints GPU + CPU ms every 120 frames while forcing continuous repaint.

use egui::{Id, LayerId, Order, PaintCallback, Rect};
use egui_glow::glow::{self, HasContext};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Inner {
    queries: Vec<glow::Query>,
    frame: usize,
    gpu_ns: Vec<u64>,
    cpu_ms: Vec<f32>,
}

#[derive(Clone, Default)]
pub struct FrameStats(Arc<Mutex<Inner>>);

impl FrameStats {
    pub fn enabled() -> bool {
        std::env::var_os("CXL_FRAMESTATS").is_some()
    }

    pub fn log_renderer(gl: &glow::Context) {
        // SAFETY: string queries on the current context.
        let (r, v) = unsafe { (gl.get_parameter_string(glow::RENDERER), gl.get_parameter_string(glow::VERSION)) };
        eprintln!("framestats: GL renderer {r} · {v}");
    }

    pub fn frame(&self, ctx: &egui::Context, cpu_ms: Option<f32>, screen: Rect) {
        ctx.request_repaint();
        if let Some(ms) = cpu_ms {
            self.0.lock().unwrap().cpu_ms.push(ms);
        }
        let begin = self.clone();
        let cb = egui_glow::CallbackFn::new(move |_info, painter| {
            let gl = painter.gl();
            let mut s = begin.0.lock().unwrap();
            // SAFETY: timer queries on the painter's context, never nested.
            unsafe {
                if s.queries.is_empty() {
                    for _ in 0..2 {
                        if let Ok(q) = gl.create_query() {
                            s.queries.push(q);
                        }
                    }
                }
                if s.queries.len() < 2 {
                    return;
                }
                let prev = s.queries[(s.frame + 1) % 2];
                if s.frame > 0 {
                    let ns = gl.get_query_parameter_u32(prev, glow::QUERY_RESULT) as u64;
                    s.gpu_ns.push(ns);
                }
                gl.begin_query(glow::TIME_ELAPSED, s.queries[s.frame % 2]);
            }
        });
        ctx.layer_painter(LayerId::background()).add(PaintCallback { rect: screen, callback: Arc::new(cb) });

        let end = self.clone();
        let cb = egui_glow::CallbackFn::new(move |_info, painter| {
            let gl = painter.gl();
            let mut s = end.0.lock().unwrap();
            if s.queries.len() < 2 {
                return;
            }
            // SAFETY: closes the query opened by the background callback this frame.
            unsafe { gl.end_query(glow::TIME_ELAPSED) };
            s.frame += 1;
            if s.gpu_ns.len() >= 120 {
                let gpu: Vec<f64> = s.gpu_ns.iter().map(|n| *n as f64 / 1e6).collect();
                let avg = gpu.iter().sum::<f64>() / gpu.len() as f64;
                let max = gpu.iter().cloned().fold(0.0, f64::max);
                let cpu = if s.cpu_ms.is_empty() { 0.0 } else { s.cpu_ms.iter().sum::<f32>() / s.cpu_ms.len() as f32 };
                eprintln!("framestats: GPU avg {avg:.2} ms · max {max:.2} ms · CPU avg {cpu:.2} ms over {} frames", gpu.len());
                s.gpu_ns.clear();
                s.cpu_ms.clear();
            }
        });
        ctx.layer_painter(LayerId::new(Order::Debug, Id::new("framestats"))).add(PaintCallback { rect: screen, callback: Arc::new(cb) });
    }
}
