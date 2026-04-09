//! Native `tish:shadertoy` module: winit + glutin + glow fullscreen fragment viewer.

mod channels;
mod compat;

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CString;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use chrono::{Datelike, Timelike};
use glow::{HasContext, PixelUnpackData};
use glutin::config::{Config, ConfigTemplateBuilder, GetGlConfig};
use glutin::context::{ContextApi, ContextAttributesBuilder, PossiblyCurrentContext, Version};
use glutin::display::GetGlDisplay;
use glutin::prelude::*;
use glutin::surface::{Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use glutin_winit::{DisplayBuilder, GlWindow};
use raw_window_handle::HasWindowHandle;
use tishlang_eval::{PropMap, TishNativeModule, Value as EvalValue};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowAttributes};

const VS: &str = r#"#version 330 core
void main() {
    vec2 p;
    if (gl_VertexID == 0) {
        p = vec2(-1.0, -1.0);
    } else if (gl_VertexID == 1) {
        p = vec2(3.0, -1.0);
    } else {
        p = vec2(-1.0, 3.0);
    }
    gl_Position = vec4(p, 0.0, 1.0);
}
"#;

fn preprocess_user_code(src: &str) -> String {
    let stripped: String = src
        .lines()
        .filter(|line| {
            let t = line.trim_start();
            !(t.starts_with("#version") || t.starts_with("precision"))
        })
        .collect::<Vec<_>>()
        .join("\n");
    compat::preprocess_webgl_compat(&stripped)
}

fn build_fragment_source(user: &str) -> String {
    let user = preprocess_user_code(user);
    format!(
        r#"#version 330 core
layout(location = 0) out vec4 fragColor;
uniform vec3  iResolution;
uniform float iTime;
uniform float iTimeDelta;
uniform int   iFrame;
uniform vec4  iMouse;
uniform vec4  iDate;
uniform float iSampleRate;
uniform vec3  iChannelResolution[4];
uniform float iChannelTime[4];
uniform float iFrameRate;
uniform sampler2D iChannel0;
uniform sampler2D iChannel1;
uniform sampler2D iChannel2;
uniform sampler2D iChannel3;
// Maps FBO gl_FragCoord to virtual full-res coords: fragCoord.xy * uFragCoordScale
uniform vec2  uFragCoordScale;
#define iGlobalTime iTime
// Shadertoy injects this; required for many copied shaders (e.g. #if HW_PERFORMANCE==0).
#ifndef HW_PERFORMANCE
#define HW_PERFORMANCE 1
#endif

{user}

void main() {{
    mainImage(fragColor, gl_FragCoord.xy * uFragCoordScale);
}}
"#
    )
}

const BLIT_FS: &str = r#"#version 330 core
layout(location = 0) out vec4 fragColor;
uniform sampler2D uBlitTex;
uniform vec2 uBlitDestPx;
void main() {
    vec2 uv = gl_FragCoord.xy / uBlitDestPx;
    fragColor = texture(uBlitTex, uv);
}
"#;

fn render_scale_from_env() -> f32 {
    std::env::var("TISH_SHADER_RENDER_SCALE")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
        .map(|s| s.clamp(0.05, 1.0))
        .unwrap_or(1.0)
}

fn gl_config_picker(configs: Box<dyn Iterator<Item = Config> + '_>) -> Config {
    // Prefer **no MSAA**: we run a full-screen fragment shader; high sample counts can
    // multiply cost dramatically with little visual benefit for a single triangle.
    configs
        .min_by_key(|c| c.num_samples())
        .expect("no GL config")
}

fn create_gl_context(window: &Window, gl_config: &Config) -> glutin::context::NotCurrentContext {
    let raw_window_handle = window.window_handle().ok().map(|h| h.as_raw());
    let context_attributes = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
        .build(raw_window_handle);
    let fallback_gles = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::Gles(Some(Version::new(3, 0))))
        .build(raw_window_handle);
    let fallback_legacy = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::OpenGl(Some(Version::new(2, 1))))
        .build(raw_window_handle);
    let gl_display = gl_config.display();
    unsafe {
        gl_display
            .create_context(gl_config, &context_attributes)
            .or_else(|_| gl_display.create_context(gl_config, &fallback_gles))
            .or_else(|_| gl_display.create_context(gl_config, &fallback_legacy))
            .expect("failed to create GL context")
    }
}

/// Backing size in **physical pixels** (actual fragment workload). `LogicalSize` would map
/// to very large framebuffers on HiDPI (e.g. Retina), which tanks heavy shaders.
fn window_physical_size() -> PhysicalSize<u32> {
    let w = std::env::var("TISH_SHADER_WIDTH")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1280)
        .max(1);
    let h = std::env::var("TISH_SHADER_HEIGHT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(720)
        .max(1);
    PhysicalSize::new(w, h)
}

fn window_attributes() -> WindowAttributes {
    let size = window_physical_size();
    Window::default_attributes()
        .with_title("Tish Shadertoy")
        .with_inner_size(size)
        .with_min_inner_size(PhysicalSize::new(1, 1))
}

enum GlInit {
    Builder(DisplayBuilder),
    Done,
}

struct ViewerState {
    gl_surface: Surface<WindowSurface>,
    window: Window,
}

pub struct ViewerApp {
    template: ConfigTemplateBuilder,
    display: GlInit,
    shader_source: String,
    gl_context: Option<PossiblyCurrentContext>,
    state: Option<ViewerState>,
    gl: Option<Rc<glow::Context>>,
    program: Option<glow::Program>,
    loc_resolution: Option<glow::UniformLocation>,
    loc_time: Option<glow::UniformLocation>,
    loc_time_delta: Option<glow::UniformLocation>,
    loc_frame: Option<glow::UniformLocation>,
    loc_mouse: Option<glow::UniformLocation>,
    loc_date: Option<glow::UniformLocation>,
    loc_sample_rate: Option<glow::UniformLocation>,
    loc_chan_res: Option<glow::UniformLocation>,
    /// OpenGL 3.3 core requires a bound VAO even when using `gl_VertexID` only.
    vao: Option<glow::VertexArray>,
    /// iChannel0–3 (1×1 black or loaded image). `channel_file_loaded[i]` true when from disk.
    channel_textures: Vec<glow::Texture>,
    channel_file_loaded: [bool; 4],
    channel_resolution_xy: [(f32, f32); 4],
    loc_i_channel_time: Option<glow::UniformLocation>,
    loc_i_frame_rate: Option<glow::UniformLocation>,
    fps_ema: f32,
    start: Option<Instant>,
    last_frame: Option<Instant>,
    frame_idx: i32,
    mouse_xy: (f32, f32),
    mouse_click_xy: (f32, f32),
    mouse_buttons: (f32, f32),
    /// `TISH_SHADER_RENDER_SCALE` at startup (GPU cost ∝ scale²).
    render_scale: f32,
    loc_frag_coord_scale: Option<glow::UniformLocation>,
    fbo: Option<glow::Framebuffer>,
    fbo_texture: Option<glow::Texture>,
    fbo_alloc_w: u32,
    fbo_alloc_h: u32,
    blit_program: Option<glow::Program>,
    loc_blit_tex: Option<glow::UniformLocation>,
    loc_blit_dest: Option<glow::UniformLocation>,
    exit_result: Result<(), String>,
}

impl ViewerApp {
    fn new(
        template: ConfigTemplateBuilder,
        display_builder: DisplayBuilder,
        shader_source: String,
    ) -> Self {
        Self {
            template,
            display: GlInit::Builder(display_builder),
            shader_source,
            gl_context: None,
            state: None,
            gl: None,
            program: None,
            loc_resolution: None,
            loc_time: None,
            loc_time_delta: None,
            loc_frame: None,
            loc_mouse: None,
            loc_date: None,
            loc_sample_rate: None,
            loc_chan_res: None,
            vao: None,
            channel_textures: Vec::new(),
            channel_file_loaded: [false; 4],
            channel_resolution_xy: [(1.0, 1.0); 4],
            loc_i_channel_time: None,
            loc_i_frame_rate: None,
            fps_ema: 0.0,
            start: None,
            last_frame: None,
            frame_idx: -1,
            mouse_xy: (-1.0, -1.0),
            mouse_click_xy: (-1.0, -1.0),
            mouse_buttons: (0.0, 0.0),
            render_scale: render_scale_from_env(),
            loc_frag_coord_scale: None,
            fbo: None,
            fbo_texture: None,
            fbo_alloc_w: 0,
            fbo_alloc_h: 0,
            blit_program: None,
            loc_blit_tex: None,
            loc_blit_dest: None,
            exit_result: Ok(()),
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, msg: String) {
        self.exit_result = Err(msg);
        event_loop.exit();
    }

    fn init_gl_resources(&mut self, gl: &glow::Context) -> Result<(), String> {
        let fs_src = build_fragment_source(&self.shader_source);
        unsafe {
            let vs = gl
                .create_shader(glow::VERTEX_SHADER)
                .map_err(|e| format!("create_shader vs: {e}"))?;
            gl.shader_source(vs, VS);
            gl.compile_shader(vs);
            if !gl.get_shader_compile_status(vs) {
                let log = gl.get_shader_info_log(vs);
                gl.delete_shader(vs);
                return Err(format!("vertex shader: {log}"));
            }

            let fs = gl
                .create_shader(glow::FRAGMENT_SHADER)
                .map_err(|e| format!("create_shader fs: {e}"))?;
            gl.shader_source(fs, &fs_src);
            gl.compile_shader(fs);
            if !gl.get_shader_compile_status(fs) {
                let log = gl.get_shader_info_log(fs);
                gl.delete_shader(vs);
                gl.delete_shader(fs);
                return Err(format!("fragment shader: {log}"));
            }

            let program = gl
                .create_program()
                .map_err(|e| format!("create_program: {e}"))?;
            gl.attach_shader(program, vs);
            gl.attach_shader(program, fs);
            gl.link_program(program);
            gl.delete_shader(fs);
            if !gl.get_program_link_status(program) {
                let log = gl.get_program_info_log(program);
                gl.delete_program(program);
                gl.delete_shader(vs);
                return Err(format!("link program: {log}"));
            }

            self.program = Some(program);
            self.loc_resolution = gl.get_uniform_location(program, "iResolution");
            self.loc_time = gl.get_uniform_location(program, "iTime");
            self.loc_time_delta = gl.get_uniform_location(program, "iTimeDelta");
            self.loc_frame = gl.get_uniform_location(program, "iFrame");
            self.loc_mouse = gl.get_uniform_location(program, "iMouse");
            self.loc_date = gl.get_uniform_location(program, "iDate");
            self.loc_sample_rate = gl.get_uniform_location(program, "iSampleRate");
            self.loc_chan_res = gl.get_uniform_location(program, "iChannelResolution");
            self.loc_i_channel_time = gl.get_uniform_location(program, "iChannelTime");
            self.loc_i_frame_rate = gl.get_uniform_location(program, "iFrameRate");
            self.loc_frag_coord_scale = gl.get_uniform_location(program, "uFragCoordScale");

            let bfs = gl
                .create_shader(glow::FRAGMENT_SHADER)
                .map_err(|e| format!("create_shader blit fs: {e}"))?;
            gl.shader_source(bfs, BLIT_FS);
            gl.compile_shader(bfs);
            if !gl.get_shader_compile_status(bfs) {
                let log = gl.get_shader_info_log(bfs);
                gl.delete_shader(bfs);
                return Err(format!("blit fragment shader: {log}"));
            }
            let blit_prog = gl
                .create_program()
                .map_err(|e| format!("create_program blit: {e}"))?;
            gl.attach_shader(blit_prog, vs);
            gl.attach_shader(blit_prog, bfs);
            gl.link_program(blit_prog);
            gl.delete_shader(bfs);
            if !gl.get_program_link_status(blit_prog) {
                let log = gl.get_program_info_log(blit_prog);
                gl.delete_program(blit_prog);
                gl.delete_shader(vs);
                return Err(format!("link blit program: {log}"));
            }
            gl.delete_shader(vs);
            self.blit_program = Some(blit_prog);
            self.loc_blit_tex = gl.get_uniform_location(blit_prog, "uBlitTex");
            self.loc_blit_dest = gl.get_uniform_location(blit_prog, "uBlitDestPx");

            let vao = gl
                .create_vertex_array()
                .map_err(|e| format!("create_vertex_array: {e}"))?;
            gl.bind_vertex_array(Some(vao));
            self.vao = Some(vao);

            let paths = channels::merged_paths(&self.shader_source);
            let base = channels::channel_base_dir();
            let mut tex = Vec::new();
            let mut file_loaded = [false; 4];
            let mut ch_xy = [(1.0f32, 1.0f32); 4];

            for i in 0..4 {
                let t = gl
                    .create_texture()
                    .map_err(|e| format!("texture: {e}"))?;
                gl.bind_texture(glow::TEXTURE_2D, Some(t));
                if let Some(ref rel) = paths[i] {
                    let full = base.join(rel);
                    let (bytes, w, h) = channels::load_rgba8(&full).map_err(|e| {
                        format!("iChannel{i} {}: {e}", full.display())
                    })?;
                    gl.tex_image_2d(
                        glow::TEXTURE_2D,
                        0,
                        glow::RGBA8 as i32,
                        w as i32,
                        h as i32,
                        0,
                        glow::RGBA,
                        glow::UNSIGNED_BYTE,
                        PixelUnpackData::Slice(Some(&bytes)),
                    );
                    file_loaded[i] = true;
                    ch_xy[i] = (w as f32, h as f32);
                } else {
                    gl.tex_image_2d(
                        glow::TEXTURE_2D,
                        0,
                        glow::RGBA8 as i32,
                        1,
                        1,
                        0,
                        glow::RGBA,
                        glow::UNSIGNED_BYTE,
                        PixelUnpackData::Slice(Some(&[0u8, 0, 0, 255])),
                    );
                }
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MIN_FILTER,
                    glow::LINEAR as i32,
                );
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MAG_FILTER,
                    glow::LINEAR as i32,
                );
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_WRAP_S,
                    glow::REPEAT as i32,
                );
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_WRAP_T,
                    glow::REPEAT as i32,
                );
                tex.push(t);
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
            self.channel_textures = tex;
            self.channel_file_loaded = file_loaded;
            self.channel_resolution_xy = ch_xy;
        }
        Ok(())
    }

    fn release_fbo(&mut self, gl: &glow::Context) {
        unsafe {
            if let Some(f) = self.fbo.take() {
                gl.delete_framebuffer(f);
            }
            if let Some(t) = self.fbo_texture.take() {
                gl.delete_texture(t);
            }
            self.fbo_alloc_w = 0;
            self.fbo_alloc_h = 0;
        }
    }

    fn ensure_fbo(&mut self, gl: &glow::Context, rw: u32, rh: u32) -> Result<(), String> {
        if self.fbo.is_some() && self.fbo_alloc_w == rw && self.fbo_alloc_h == rh {
            return Ok(());
        }
        unsafe {
            self.release_fbo(gl);
            let tex = gl
                .create_texture()
                .map_err(|e| format!("fbo texture: {e}"))?;
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                rw as i32,
                rh as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                PixelUnpackData::Slice(None),
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.bind_texture(glow::TEXTURE_2D, None);

            let fbo = gl
                .create_framebuffer()
                .map_err(|e| format!("create_framebuffer: {e}"))?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(tex),
                0,
            );
            gl.draw_buffers(&[glow::COLOR_ATTACHMENT0]);
            let st = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            if st != glow::FRAMEBUFFER_COMPLETE {
                gl.delete_framebuffer(fbo);
                gl.delete_texture(tex);
                return Err(format!(
                    "framebuffer incomplete (status {:#x}, size {rw}x{rh})",
                    st
                ));
            }
            self.fbo = Some(fbo);
            self.fbo_texture = Some(tex);
            self.fbo_alloc_w = rw;
            self.fbo_alloc_h = rh;
        }
        Ok(())
    }

    /// `i_resolution` is the logical size passed to `iResolution` (usually the window).
    /// `viewport` is the framebuffer we draw into. `frag_scale` maps `gl_FragCoord` to
    /// virtual coords: multiply by `(full_w/vp_w, full_h/vp_h)` when downsampling.
    fn run_shadertoy_pass(
        &self,
        gl: &glow::Context,
        viewport_w: u32,
        viewport_h: u32,
        i_resolution: (f32, f32),
        frag_scale: (f32, f32),
        i_time: f32,
        i_time_delta: f32,
        mouse_xyzw: (f32, f32, f32, f32),
        date_xyzw: (f32, f32, f32, f32),
    ) -> Result<(), String> {
        let program = self
            .program
            .ok_or_else(|| "internal: missing shader program".to_string())?;

        unsafe {
            gl.viewport(0, 0, viewport_w as i32, viewport_h as i32);
            gl.use_program(Some(program));

            if let Some(loc) = self.loc_resolution.as_ref() {
                gl.uniform_3_f32(Some(loc), i_resolution.0, i_resolution.1, 1.0);
            }
            if let Some(loc) = self.loc_frag_coord_scale.as_ref() {
                gl.uniform_2_f32(Some(loc), frag_scale.0, frag_scale.1);
            }
            if let Some(loc) = self.loc_time.as_ref() {
                gl.uniform_1_f32(Some(loc), i_time);
            }
            if let Some(loc) = self.loc_time_delta.as_ref() {
                gl.uniform_1_f32(Some(loc), i_time_delta);
            }
            if let Some(loc) = self.loc_frame.as_ref() {
                gl.uniform_1_i32(Some(loc), self.frame_idx);
            }
            if let Some(loc) = self.loc_mouse.as_ref() {
                gl.uniform_4_f32(
                    Some(loc),
                    mouse_xyzw.0,
                    mouse_xyzw.1,
                    mouse_xyzw.2,
                    mouse_xyzw.3,
                );
            }
            if let Some(loc) = self.loc_date.as_ref() {
                gl.uniform_4_f32(
                    Some(loc),
                    date_xyzw.0,
                    date_xyzw.1,
                    date_xyzw.2,
                    date_xyzw.3,
                );
            }
            if let Some(loc) = self.loc_sample_rate.as_ref() {
                gl.uniform_1_f32(Some(loc), 44100.0);
            }
            if let Some(loc) = self.loc_chan_res.as_ref() {
                let mut v = [0.0f32; 12];
                for i in 0..4 {
                    v[i * 3] = self.channel_resolution_xy[i].0;
                    v[i * 3 + 1] = self.channel_resolution_xy[i].1;
                    v[i * 3 + 2] = 1.0;
                }
                gl.uniform_3_f32_slice(Some(loc), &v);
            }
            if let Some(loc) = self.loc_i_channel_time.as_ref() {
                let ct: [f32; 4] = std::array::from_fn(|i| {
                    if self.channel_file_loaded[i] {
                        0.0
                    } else {
                        i_time
                    }
                });
                gl.uniform_1_f32_slice(Some(loc), &ct);
            }
            if let Some(loc) = self.loc_i_frame_rate.as_ref() {
                gl.uniform_1_f32(Some(loc), self.fps_ema);
            }

            for (unit, tex) in self.channel_textures.iter().enumerate() {
                gl.active_texture(glow::TEXTURE0 + unit as u32);
                gl.bind_texture(glow::TEXTURE_2D, Some(*tex));
                let name = match unit {
                    0 => "iChannel0",
                    1 => "iChannel1",
                    2 => "iChannel2",
                    _ => "iChannel3",
                };
                if let Some(u) = gl.get_uniform_location(program, name) {
                    gl.uniform_1_i32(Some(&u), unit as i32);
                }
            }

            if let Some(vao) = self.vao {
                gl.bind_vertex_array(Some(vao));
            }
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
        }
        Ok(())
    }

    fn draw_frame(&mut self, gl: &Rc<glow::Context>, w: u32, h: u32) -> Result<(), String> {
        let now = Instant::now();
        let start = *self.start.get_or_insert(now);
        let i_time = now.duration_since(start).as_secs_f32();
        let i_time_delta = self
            .last_frame
            .map(|p| now.duration_since(p).as_secs_f32())
            .unwrap_or(0.0);
        self.last_frame = Some(now);
        self.frame_idx = self.frame_idx.saturating_add(1);

        let inst_fps = if i_time_delta > 1e-6 {
            1.0 / i_time_delta
        } else {
            0.0
        };
        if self.fps_ema < 1e-3 {
            self.fps_ema = inst_fps;
        } else {
            self.fps_ema = self.fps_ema * 0.92 + inst_fps * 0.08;
        }

        let (mx, my) = self.mouse_xy;
        let (cx, cy) = self.mouse_click_xy;

        let local = chrono::Local::now();
        let year = local.year() as f32;
        let month = local.month() as f32;
        let day = local.day() as f32;
        let sec_in_day = (local.hour() as f32) * 3600.0
            + (local.minute() as f32) * 60.0
            + local.second() as f32;

        if self.program.is_none() {
            return Ok(());
        }
        let mz = if self.mouse_buttons.0 > 0.5 {
            cx
        } else {
            -cx.abs() - 1.0
        };
        let mw = if self.mouse_buttons.0 > 0.5 {
            cy
        } else {
            -cy.abs() - 1.0
        };

        let scale = self.render_scale;
        let rw = ((w as f32 * scale).round() as u32).max(1);
        let rh = ((h as f32 * scale).round() as u32).max(1);
        let use_low_res_pass = scale < 0.999 && (rw < w || rh < h);

        let mouse_xyzw = (mx, my, mz, mw);
        let date_xyzw = (year, month, day, sec_in_day);
        let res_wh = (w as f32, h as f32);

        if use_low_res_pass {
            self.ensure_fbo(gl, rw, rh)?;
            unsafe {
                let fbo = self.fbo.ok_or_else(|| "internal: missing FBO".to_string())?;
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                gl.clear_color(0.0, 0.0, 0.0, 1.0);
                gl.clear(glow::COLOR_BUFFER_BIT);
            }
            let sx = w as f32 / rw as f32;
            let sy = h as f32 / rh as f32;
            self.run_shadertoy_pass(
                gl,
                rw,
                rh,
                res_wh,
                (sx, sy),
                i_time,
                i_time_delta,
                mouse_xyzw,
                date_xyzw,
            )?;
            unsafe {
                gl.bind_framebuffer(glow::FRAMEBUFFER, None);

                let blit = self
                    .blit_program
                    .ok_or_else(|| "internal: missing blit program".to_string())?;
                let fbo_tex = self
                    .fbo_texture
                    .ok_or_else(|| "internal: missing FBO texture".to_string())?;

                gl.viewport(0, 0, w as i32, h as i32);
                gl.clear_color(0.0, 0.0, 0.0, 1.0);
                gl.clear(glow::COLOR_BUFFER_BIT);
                gl.use_program(Some(blit));
                gl.active_texture(glow::TEXTURE0);
                gl.bind_texture(glow::TEXTURE_2D, Some(fbo_tex));
                if let Some(loc) = self.loc_blit_tex.as_ref() {
                    gl.uniform_1_i32(Some(loc), 0);
                }
                if let Some(loc) = self.loc_blit_dest.as_ref() {
                    gl.uniform_2_f32(Some(loc), w as f32, h as f32);
                }
                if let Some(vao) = self.vao {
                    gl.bind_vertex_array(Some(vao));
                }
                gl.draw_arrays(glow::TRIANGLES, 0, 3);
            }
        } else {
            if self.fbo.is_some() {
                self.release_fbo(gl);
            }
            unsafe {
                gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            }
            self.run_shadertoy_pass(
                gl,
                w,
                h,
                res_wh,
                (1.0, 1.0),
                i_time,
                i_time_delta,
                mouse_xyzw,
                date_xyzw,
            )?;
        }

        Ok(())
    }
}

impl ApplicationHandler for ViewerApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let (window, gl_config) = match std::mem::replace(&mut self.display, GlInit::Done) {
            GlInit::Builder(display_builder) => {
                let (window, gl_config) = match display_builder.build(
                    event_loop,
                    self.template.clone(),
                    gl_config_picker,
                ) {
                    Ok((Some(w), c)) => (w, c),
                    Ok((None, _)) => {
                        self.fail(
                            event_loop,
                            "GL display build returned no window".into(),
                        );
                        return;
                    }
                    Err(e) => {
                        self.fail(event_loop, format!("GL init: {e}"));
                        return;
                    }
                };
                let ctx = create_gl_context(&window, &gl_config);
                self.gl_context = Some(ctx.treat_as_possibly_current());
                (window, gl_config)
            }
            GlInit::Done => {
                let gl_config = match self.gl_context.as_ref() {
                    Some(c) => c.config().clone(),
                    None => {
                        self.fail(event_loop, "missing GL context".into());
                        return;
                    }
                };
                match glutin_winit::finalize_window(event_loop, window_attributes(), &gl_config) {
                    Ok(w) => (w, gl_config),
                    Err(e) => {
                        self.fail(event_loop, format!("window: {e}"));
                        return;
                    }
                }
            }
        };

        let attrs = match window.build_surface_attributes(SurfaceAttributesBuilder::<WindowSurface>::new())
        {
            Ok(a) => a,
            Err(e) => {
                self.fail(event_loop, format!("surface attrs: {e}"));
                return;
            }
        };
        let gl_surface = match unsafe { gl_config.display().create_window_surface(&gl_config, &attrs) }
        {
            Ok(s) => s,
            Err(e) => {
                self.fail(event_loop, format!("surface: {e}"));
                return;
            }
        };

        {
            let gl_context = match self.gl_context.as_ref() {
                Some(c) => c,
                None => {
                    self.fail(event_loop, "no context".into());
                    return;
                }
            };
            if let Err(e) = gl_context.make_current(&gl_surface) {
                self.fail(event_loop, format!("make_current: {e}"));
                return;
            }
        }

        let gl = unsafe {
            glow::Context::from_loader_function(|s| {
                let name = CString::new(s).unwrap();
                gl_config.display().get_proc_address(&name)
            })
        };

        if let Err(msg) = self.init_gl_resources(&gl) {
            self.fail(event_loop, msg);
            return;
        }
        self.gl = Some(Rc::new(gl));

        let vsync = !matches!(
            std::env::var("TISH_SHADER_VSYNC").as_deref(),
            Ok("0" | "false" | "off")
        );
        let interval = if vsync {
            SwapInterval::Wait(NonZeroU32::new(1).unwrap())
        } else {
            SwapInterval::DontWait
        };
        if let Err(e) = gl_surface.set_swap_interval(self.gl_context.as_ref().unwrap(), interval) {
            eprintln!("swap interval: {e:?}");
        }

        assert!(self.state.replace(ViewerState { gl_surface, window }).is_none());
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { event, .. } => {
                if event.logical_key == Key::Named(NamedKey::Escape) {
                    event_loop.exit();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(s) = self.state.as_ref() {
                    let ps = s.window.inner_size();
                    self.mouse_xy = (position.x as f32, (ps.height as f32) - position.y as f32);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let v = if state == ElementState::Pressed { 1.0 } else { 0.0 };
                match button {
                    MouseButton::Left => {
                        self.mouse_buttons.0 = v;
                        if state == ElementState::Pressed {
                            self.mouse_click_xy = self.mouse_xy;
                        }
                    }
                    MouseButton::Right => self.mouse_buttons.1 = v,
                    _ => {}
                }
            }
            WindowEvent::RedrawRequested => {
                let (ww, hh) = match self.state.as_ref() {
                    Some(s) => {
                        let sz = s.window.inner_size();
                        (sz.width, sz.height)
                    }
                    None => return,
                };
                if ww == 0 || hh == 0 {
                    return;
                }
                let Some(gl) = self.gl.clone() else {
                    return;
                };
                if let Err(e) = self.draw_frame(&gl, ww, hh) {
                    self.fail(event_loop, e);
                    return;
                }
                if let (Some(s), Some(gc)) = (self.state.as_ref(), self.gl_context.as_ref()) {
                    if let Err(e) = s.gl_surface.swap_buffers(gc) {
                        eprintln!("swap_buffers: {e:?}");
                    }
                }
            }
            WindowEvent::Resized(size) => {
                if let (Some(s), Some(gl_context)) = (self.state.as_ref(), self.gl_context.as_ref())
                {
                    if size.width > 0 && size.height > 0 {
                        s.window.resize_surface(&s.gl_surface, gl_context);
                        if let Some(gl) = self.gl.as_ref() {
                            unsafe {
                                gl.viewport(0, 0, size.width as i32, size.height as i32);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(s) = self.state.as_ref() {
            s.window.request_redraw();
        }
        event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
    }
}

impl Drop for ViewerApp {
    fn drop(&mut self) {
        if let Some(gl) = self.gl.take() {
            if let Ok(gl) = Rc::try_unwrap(gl) {
                unsafe {
                    if let Some(f) = self.fbo.take() {
                        gl.delete_framebuffer(f);
                    }
                    if let Some(t) = self.fbo_texture.take() {
                        gl.delete_texture(t);
                    }
                    if let Some(p) = self.blit_program.take() {
                        gl.delete_program(p);
                    }
                    if let Some(v) = self.vao.take() {
                        gl.delete_vertex_array(v);
                    }
                    if let Some(p) = self.program.take() {
                        gl.delete_program(p);
                    }
                    for t in self.channel_textures.drain(..) {
                        gl.delete_texture(t);
                    }
                }
            }
        }
    }
}

/// Run the viewer until the window closes. Blocks the Tish interpreter thread.
pub fn run_viewer(shader_code: &str) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    let template = ConfigTemplateBuilder::new().with_alpha_size(8);
    let display_builder =
        DisplayBuilder::new().with_window_attributes(Some(window_attributes()));
    let mut app = ViewerApp::new(template, display_builder, shader_code.to_string());
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    std::mem::replace(&mut app.exit_result, Ok(()))
}

fn shadertoy_run(args: &[EvalValue]) -> Result<EvalValue, String> {
    let code = args
        .first()
        .map(|v| v.to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "shadertoy.run: expected non-empty shader source string".to_string())?;
    run_viewer(&code)?;
    Ok(EvalValue::Null)
}

/// Registered with `Evaluator::with_modules` for `import { run } from 'tish:shadertoy'`.
pub struct ShadertoyModule;

impl TishNativeModule for ShadertoyModule {
    fn name(&self) -> &'static str {
        "Shadertoy"
    }

    fn register(&self) -> HashMap<Arc<str>, EvalValue> {
        HashMap::new()
    }

    fn virtual_builtin_modules(&self) -> Vec<(&'static str, EvalValue)> {
        let mut exports = PropMap::default();
        exports.insert("run".into(), EvalValue::Native(shadertoy_run));
        vec![(
            "tish:shadertoy",
            EvalValue::Object(Rc::new(RefCell::new(exports))),
        )]
    }
}
