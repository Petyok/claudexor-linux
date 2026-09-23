//! L2 Liquid Glass: a glow paint callback that REFRACTS the framebuffer behind
//! a chrome surface instead of only blurring it (PLAN §5 L2).
//!
//! Per fragment: a squircle SDF gives the shape and (via its gradient) the
//! edge normal; the sample point is pushed inward along the normal by a bevel
//! profile that is strong at the rim and zero in the centre; R/G/B sample at
//! slightly different offsets (≈0.02× dispersion); an 8-tap ring blur softens
//! it; then a tint film, a Fresnel rim and a thin top specular. The bend fades
//! out over the outermost ~10 px to avoid "cracked glass" hairlines.
//!
//! Composite is exact in two passes over the same quad: pass 0 scales the
//! destination by (1 − mask), pass 1 adds mask·glass — so transparent-window
//! alpha stays correct and nothing outside the squircle is touched.
//! Requires GLSL 1.40 / ES 3.00; `Refractor::new` returns None otherwise and
//! the caller falls back to L1 frost.

use egui::{PaintCallback, Rect, Ui};
use egui_glow::glow::{self, HasContext};
use std::sync::{Arc, Mutex};

const VS: &str = r#"
in vec2 a_pos;
out vec2 v_px;
uniform vec2 u_size;
void main() {
    v_px = a_pos * u_size;
    gl_Position = vec4(a_pos * 2.0 - 1.0, 0.0, 1.0);
}
"#;

const FS: &str = r#"
#ifdef GL_ES
precision highp float;
#endif
in vec2 v_px;
out vec4 o_color;
uniform sampler2D u_tex;
uniform vec4 u_rect;   // surface: left, bottom, width, height (GL px)
uniform vec4 u_grab;   // grab origin (GL px) + grab texture size (px)
uniform vec2 u_lim;    // uv extent actually filled by the copy
uniform vec2 u_size;
uniform float u_radius;
uniform float u_bevel;
uniform float u_strength;
uniform float u_fade;
uniform float u_blur;
uniform vec4 u_tint;   // straight-alpha gamma rgb + film mix
uniform float u_dark;
uniform int u_pass;

float len4(vec2 v) { v = v * v; return pow(dot(v, v), 0.25); }
// Rounded box whose corner uses the L4 norm: a squircle-like continuous corner.
float sd(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + r;
    return len4(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}
vec4 tap(vec2 px) {
    vec2 uv = clamp((px - u_grab.xy) / u_grab.zw, vec2(0.0), u_lim);
    return texture(u_tex, uv);
}
vec4 ring(vec2 px) {
    vec4 c = tap(px) * 2.0;
    for (int i = 0; i < 8; i++) {
        float a = float(i) * 0.7853982;
        c += tap(px + vec2(cos(a), sin(a)) * u_blur);
    }
    return c / 10.0;
}
void main() {
    vec2 h = u_size * 0.5;
    vec2 p = v_px - h;
    float d = -sd(p, h, u_radius);                 // px inside the edge
    float mask = clamp(d + 0.5, 0.0, 1.0);          // 1 px anti-aliased edge
    if (mask <= 0.0) discard;
    if (u_pass == 0) { o_color = vec4(0.0, 0.0, 0.0, mask); return; }

    vec2 e = vec2(1.0, 0.0);
    vec2 n = vec2(sd(p + e.xy, h, u_radius) - sd(p - e.xy, h, u_radius),
                  sd(p + e.yx, h, u_radius) - sd(p - e.yx, h, u_radius));
    n = n / max(length(n), 1e-4);                   // outward normal

    float x = clamp(d / u_bevel, 0.0, 1.0);          // 0 at rim → 1 past the bevel
    float bend = 1.0 - x;
    bend = bend * bend * (3.0 - 2.0 * bend);         // strong at the rim, 0 in the centre
    float off = u_strength * bend * smoothstep(0.0, u_fade, d);

    vec2 frag = u_rect.xy + v_px;
    vec2 dir = -n;                                   // pull the sample inward
    float cr = ring(frag + dir * off * 1.02).r;
    vec4 cg = ring(frag + dir * off);
    float cb = ring(frag + dir * off * 0.98).b;
    vec4 col = vec4(cr, cg.g, cb, cg.a);             // premultiplied texels

    col = mix(col, vec4(u_tint.rgb, 1.0), u_tint.a); // glass film
    float fres = pow(bend, 3.0) * (u_dark > 0.5 ? 0.10 : 0.16);
    float spec = pow(max(n.y, 0.0), 6.0) * (1.0 - smoothstep(0.0, 2.5, d)) * 0.55;
    col.rgb = min(col.rgb + vec3(fres + spec) * col.a, vec3(col.a));
    o_color = col * mask;
}
"#;

struct Gl {
    program: glow::Program,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    /// Grab texture, grown on demand: (texture, width, height).
    tex: Mutex<Option<(glow::Texture, i32, i32)>>,
}

#[derive(Clone)]
pub struct Refractor {
    gl: Arc<Gl>,
}

unsafe fn compile(gl: &glow::Context, kind: u32, src: &str, header: &str) -> Result<glow::Shader, String> {
    unsafe {
        let s = gl.create_shader(kind)?;
        gl.shader_source(s, &format!("{header}{src}"));
        gl.compile_shader(s);
        if !gl.get_shader_compile_status(s) {
            let log = gl.get_shader_info_log(s);
            gl.delete_shader(s);
            return Err(log);
        }
        Ok(s)
    }
}

impl Refractor {
    pub fn new(gl: &glow::Context) -> Result<Self, String> {
        let v = egui_glow::ShaderVersion::get(gl);
        if !v.is_new_shader_interface() {
            return Err(format!("{v:?} lacks in/out shader interface"));
        }
        let header = v.version_declaration();
        // SAFETY: standard GL object creation on the current context.
        unsafe {
            let vs = compile(gl, glow::VERTEX_SHADER, VS, header)?;
            let fs = compile(gl, glow::FRAGMENT_SHADER, FS, header)?;
            let program = gl.create_program()?;
            gl.attach_shader(program, vs);
            gl.attach_shader(program, fs);
            gl.bind_attrib_location(program, 0, "a_pos");
            gl.link_program(program);
            gl.detach_shader(program, vs);
            gl.detach_shader(program, fs);
            gl.delete_shader(vs);
            gl.delete_shader(fs);
            if !gl.get_program_link_status(program) {
                return Err(gl.get_program_info_log(program));
            }
            let vao = gl.create_vertex_array()?;
            let vbo = gl.create_buffer()?;
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let quad: [f32; 8] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];
            let bytes: Vec<u8> = quad.iter().flat_map(|f| f.to_ne_bytes()).collect();
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &bytes, glow::STATIC_DRAW);
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            Ok(Refractor { gl: Arc::new(Gl { program, vao, vbo, tex: Mutex::new(None) }) })
        }
    }

    pub fn destroy(&self, gl: &glow::Context) {
        // SAFETY: deleting objects this struct created, on the same context.
        unsafe {
            gl.delete_program(self.gl.program);
            gl.delete_vertex_array(self.gl.vao);
            gl.delete_buffer(self.gl.vbo);
            if let Some((t, ..)) = self.gl.tex.lock().ok().and_then(|mut g| g.take()) {
                gl.delete_texture(t);
            }
        }
    }

    /// Enqueue the refraction for `rect` (points). Paint content AFTER this.
    pub fn paint(&self, ui: &Ui, rect: Rect, radius: f32, dark: bool) {
        let me = self.gl.clone();
        let tint: [f32; 4] = if dark { [0.16, 0.18, 0.22, 0.52] } else { [0.97, 0.98, 1.0, 0.48] };
        let cb = egui_glow::CallbackFn::new(move |info, painter| {
            let gl = painter.gl();
            let ppp = info.pixels_per_point;
            let vp = info.viewport_in_pixels();
            let [sw, sh] = info.screen_size_px;
            let (strength, blur, bevel, fade) = (14.0 * ppp, 2.2 * ppp, 22.0 * ppp, 10.0 * ppp);
            let margin = (strength + blur * 2.0 + 2.0).ceil() as i32;
            let gx = (vp.left_px - margin).max(0);
            let gy = (vp.from_bottom_px - margin).max(0);
            let gw = (vp.left_px + vp.width_px + margin).min(sw as i32) - gx;
            let gh = (vp.from_bottom_px + vp.height_px + margin).min(sh as i32) - gy;
            if gw <= 0 || gh <= 0 {
                return;
            }
            // SAFETY: GL calls on the painter's current context; egui_glow restores state after.
            unsafe {
                let mut slot = me.tex.lock().unwrap_or_else(|p| p.into_inner());
                let need_new = !matches!(*slot, Some((_, w, h)) if w >= gw && h >= gh);
                if need_new {
                    if let Some((old, ..)) = slot.take() {
                        gl.delete_texture(old);
                    }
                    let Ok(t) = gl.create_texture() else { return };
                    let (w, h) = ((gw as u32).next_power_of_two().max(256) as i32, (gh as u32).next_power_of_two().max(256) as i32);
                    gl.bind_texture(glow::TEXTURE_2D, Some(t));
                    gl.tex_image_2d(
                        glow::TEXTURE_2D,
                        0,
                        glow::RGBA8 as i32,
                        w,
                        h,
                        0,
                        glow::RGBA,
                        glow::UNSIGNED_BYTE,
                        glow::PixelUnpackData::Slice(None),
                    );
                    for (k, v) in [
                        (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                        (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                        (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                        (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
                    ] {
                        gl.tex_parameter_i32(glow::TEXTURE_2D, k, v as i32);
                    }
                    *slot = Some((t, w, h));
                }
                let (tex, tw, th) = slot.expect("texture allocated above");
                gl.active_texture(glow::TEXTURE0);
                gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                gl.copy_tex_sub_image_2d(glow::TEXTURE_2D, 0, 0, 0, gx, gy, gw, gh);

                let prog = me.program;
                gl.use_program(Some(prog));
                let u = |n: &str| gl.get_uniform_location(prog, n);
                gl.uniform_1_i32(u("u_tex").as_ref(), 0);
                gl.uniform_4_f32(
                    u("u_rect").as_ref(),
                    vp.left_px as f32,
                    vp.from_bottom_px as f32,
                    vp.width_px as f32,
                    vp.height_px as f32,
                );
                // uv maps grab px → [0,1] of the (larger) texture's used corner
                gl.uniform_4_f32(u("u_grab").as_ref(), gx as f32, gy as f32, tw as f32, th as f32);
                gl.uniform_2_f32(u("u_lim").as_ref(), (gw as f32 - 0.5) / tw as f32, (gh as f32 - 0.5) / th as f32);
                gl.uniform_2_f32(u("u_size").as_ref(), vp.width_px as f32, vp.height_px as f32);
                gl.uniform_1_f32(u("u_radius").as_ref(), (radius * ppp).min(vp.height_px as f32 * 0.5));
                gl.uniform_1_f32(u("u_bevel").as_ref(), bevel);
                gl.uniform_1_f32(u("u_strength").as_ref(), strength);
                gl.uniform_1_f32(u("u_fade").as_ref(), fade);
                gl.uniform_1_f32(u("u_blur").as_ref(), blur);
                gl.uniform_4_f32(u("u_tint").as_ref(), tint[0], tint[1], tint[2], tint[3]);
                gl.uniform_1_f32(u("u_dark").as_ref(), if dark { 1.0 } else { 0.0 });
                gl.bind_vertex_array(Some(me.vao));
                gl.enable(glow::BLEND);
                // pass 0: dst *= (1 - mask)
                gl.uniform_1_i32(u("u_pass").as_ref(), 0);
                gl.blend_func(glow::ZERO, glow::ONE_MINUS_SRC_ALPHA);
                gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
                // pass 1: dst += mask * glass
                gl.uniform_1_i32(u("u_pass").as_ref(), 1);
                gl.blend_func(glow::ONE, glow::ONE);
                gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
                gl.bind_vertex_array(None);
            }
        });
        ui.painter().add(PaintCallback { rect, callback: Arc::new(cb) });
    }
}
