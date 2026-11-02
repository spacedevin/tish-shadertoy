//! OpenGL Shadertoy-style fullscreen fragment pass (no windowing).

use std::rc::Rc;
use std::time::Instant;

use chrono::{Datelike, Timelike};
use glow::{HasContext, PixelPackData, PixelUnpackData};

use crate::channels;

/// Mouse uniforms for one frame (window pixel space, Y up).
#[derive(Clone, Copy, Default)]
pub struct ShadertoyInput {
    pub mouse_xy: (f32, f32),
    pub mouse_click_xy: (f32, f32),
    pub mouse_buttons: (f32, f32),
}

pub struct ShadertoyRenderer {
    shader_source: String,
    program: Option<glow::Program>,
    loc_resolution: Option<glow::UniformLocation>,
    loc_time: Option<glow::UniformLocation>,
    loc_time_delta: Option<glow::UniformLocation>,
    loc_frame: Option<glow::UniformLocation>,
    loc_mouse: Option<glow::UniformLocation>,
    loc_date: Option<glow::UniformLocation>,
    loc_sample_rate: Option<glow::UniformLocation>,
    loc_chan_res: Option<glow::UniformLocation>,
    vao: Option<glow::VertexArray>,
    channel_textures: Vec<glow::Texture>,
    channel_file_loaded: [bool; 4],
    channel_resolution_xy: [(f32, f32); 4],
    loc_i_channel_time: Option<glow::UniformLocation>,
    loc_i_frame_rate: Option<glow::UniformLocation>,
    fps_ema: f32,
    start: Option<Instant>,
    last_frame: Option<Instant>,
    frame_idx: i32,
    render_scale: f32,
    fbo: Option<glow::Framebuffer>,
    fbo_texture: Option<glow::Texture>,
    fbo_alloc_w: u32,
    fbo_alloc_h: u32,
    blit_program: Option<glow::Program>,
    loc_blit_tex: Option<glow::UniformLocation>,
    loc_blit_dest: Option<glow::UniformLocation>,
    debug_pixel_done: bool,
}

impl ShadertoyRenderer {
    pub fn new(shader_source: String) -> Self {
        Self {
            shader_source,
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
            render_scale: render_scale_from_env(),
            fbo: None,
            fbo_texture: None,
            fbo_alloc_w: 0,
            fbo_alloc_h: 0,
            blit_program: None,
            loc_blit_tex: None,
            loc_blit_dest: None,
            debug_pixel_done: false,
        }
    }

    pub fn fps_ema(&self) -> f32 {
        self.fps_ema
    }

    pub fn tear_down_gl(&mut self, gl: &glow::Context) {
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

/// Full-screen quad (two triangles). Fragment `mainImage` uses **`gl_FragCoord.xy`** with `iResolution.xy`.
const VS: &str = r#"#version 330 core
void main() {
    vec2 p = vec2[](
        vec2( 1.0,  1.0),
        vec2(-1.0,  1.0),
        vec2(-1.0, -1.0),
        vec2( 1.0,  1.0),
        vec2(-1.0, -1.0),
        vec2( 1.0, -1.0)
    )[gl_VertexID];
    gl_Position = vec4(p, 0.0, 1.0);
}
"#;

/// `iResolution` **xy** plus **z = x/y**, matching [shadertoy-rs `runner.rs`](https://github.com/fmenozzi/shadertoy-rs/blob/master/src/runner.rs) (`[width, height, width / height]`).
fn i_resolution_uniform(w: f32, h: f32) -> [f32; 3] {
    let z = if h > 1e-20 { w / h } else { 1.0 };
    [w, h, z]
}

/// Map window-space `iMouse` into coordinates for a smaller offscreen buffer (`TISH_SHADER_RENDER_SCALE` path).
fn mouse_scaled_to_buffer(
    mouse: (f32, f32, f32, f32),
    win_w: u32,
    win_h: u32,
    buf_w: u32,
    buf_h: u32,
) -> (f32, f32, f32, f32) {
    if win_w == 0 || win_h == 0 {
        return mouse;
    }
    let sx = buf_w as f32 / win_w as f32;
    let sy = buf_h as f32 / win_h as f32;
    (
        mouse.0 * sx,
        mouse.1 * sy,
        mouse.2 * sx,
        mouse.3 * sy,
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

impl ShadertoyRenderer {
    /// Replace fragment source and rebuild GL programs (same context). Call from the GL thread.
    pub fn reload(&mut self, gl: &glow::Context, new_source: String) -> Result<(), String> {
        self.tear_down_gl(gl);
        self.shader_source = new_source;
        self.start = None;
        self.last_frame = None;
        self.frame_idx = -1;
        self.debug_pixel_done = false;
        self.init(gl)
    }

    pub fn init(&mut self, gl: &glow::Context) -> Result<(), String> {
        let fs_src = crate::build_fragment_source(&self.shader_source);
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

            self.loc_resolution = gl.get_uniform_location(program, "iResolution");
            if self.loc_resolution.is_none() {
                gl.delete_program(program);
                gl.delete_shader(vs);
                return Err(
                    "linked program has no active iResolution uniform (required for Shadertoy)".into(),
                );
            }
            self.program = Some(program);
            self.loc_time = gl.get_uniform_location(program, "iTime");
            self.loc_time_delta = gl.get_uniform_location(program, "iTimeDelta");
            self.loc_frame = gl.get_uniform_location(program, "iFrame");
            self.loc_mouse = gl.get_uniform_location(program, "iMouse");
            self.loc_date = gl.get_uniform_location(program, "iDate");
            self.loc_sample_rate = gl.get_uniform_location(program, "iSampleRate");
            self.loc_chan_res = gl.get_uniform_location(program, "iChannelResolution");
            self.loc_i_channel_time = gl.get_uniform_location(program, "iChannelTime");
            self.loc_i_frame_rate = gl.get_uniform_location(program, "iFrameRate");
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

            let paths = channels::resolved_channel_paths(&self.shader_source);
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
                    let (bytes, w, h) = if channels::is_builtin_noise_path(rel.as_path()) {
                        channels::procedural_noise_rgba(256, 256)
                    } else {
                        let full = base.join(rel);
                        channels::load_rgba8(&full).map_err(|e| {
                            format!("iChannel{i} {}: {e}", full.display())
                        })?
                    };
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

            eprintln!(
                "tish-shadertoy: GL {:?} (shader OK; if black, check terminal with TISH_SHADER_DEBUG=1)",
                gl.version()
            );
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

    /// **`i_resolution.xy` must match the draw viewport** so `gl_FragCoord.xy` and `iResolution` agree.
    /// **`z` is `x/y`** (shadertoy-rs style).
    #[allow(clippy::too_many_arguments)]
    fn run_shadertoy_pass(
        &self,
        gl: &glow::Context,
        viewport_w: u32,
        viewport_h: u32,
        i_resolution: (f32, f32),
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
            gl.disable(glow::CULL_FACE);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::STENCIL_TEST);
            gl.disable(glow::BLEND);
            gl.disable(glow::SCISSOR_TEST);
            if crate::framebuffer_srgb_write_enabled() {
                gl.enable(glow::FRAMEBUFFER_SRGB);
            } else {
                gl.disable(glow::FRAMEBUFFER_SRGB);
            }
            gl.use_program(Some(program));

            let loc = self
                .loc_resolution
                .as_ref()
                .expect("iResolution location checked at init");
            let r = i_resolution_uniform(i_resolution.0, i_resolution.1);
            gl.uniform_3_f32(Some(loc), r[0], r[1], r[2]);
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
            gl.draw_arrays(glow::TRIANGLES, 0, 6);
        }
        Ok(())
    }

    pub fn draw_frame(&mut self, gl: &Rc<glow::Context>, w: u32, h: u32, input: &ShadertoyInput) -> Result<(), String> {
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

        // Shadertoy: iMouse.xy are only valid while LMB is down; otherwise negative (see site / IQ notes).
        let (mx, my) = if input.mouse_buttons.0 > 0.5 {
            input.mouse_xy
        } else {
            (-1.0, -1.0)
        };
        let (cx, cy) = input.mouse_click_xy;

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
        let mz = if input.mouse_buttons.0 > 0.5 {
            cx
        } else {
            -cx.abs() - 1.0
        };
        let mw = if input.mouse_buttons.0 > 0.5 {
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
        let res_buf = (rw as f32, rh as f32);

        if use_low_res_pass {
            self.ensure_fbo(gl, rw, rh)?;
            unsafe {
                let fbo = self.fbo.ok_or_else(|| "internal: missing FBO".to_string())?;
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                gl.clear_color(0.0, 0.0, 0.0, 1.0);
                gl.clear(glow::COLOR_BUFFER_BIT);
            }
            let mouse_buf = mouse_scaled_to_buffer(mouse_xyzw, w, h, rw, rh);
            self.run_shadertoy_pass(
                gl,
                rw,
                rh,
                res_buf,
                i_time,
                i_time_delta,
                mouse_buf,
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
                gl.disable(glow::CULL_FACE);
                gl.disable(glow::DEPTH_TEST);
                gl.disable(glow::STENCIL_TEST);
                gl.disable(glow::BLEND);
                gl.disable(glow::SCISSOR_TEST);
                if crate::framebuffer_srgb_write_enabled() {
                    gl.enable(glow::FRAMEBUFFER_SRGB);
                } else {
                    gl.disable(glow::FRAMEBUFFER_SRGB);
                }
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
                gl.draw_arrays(glow::TRIANGLES, 0, 6);
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
                i_time,
                i_time_delta,
                mouse_xyzw,
                date_xyzw,
            )?;
        }

        if !self.debug_pixel_done
            && matches!(std::env::var("TISH_SHADER_DEBUG").as_deref(), Ok("1"))
            && w > 0
            && h > 0
        {
            self.debug_pixel_done = true;
            unsafe {
                gl.finish();
                gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
                let mut px = [0u8; 4];
                let cx = (w / 2) as i32;
                let cy = (h / 2) as i32;
                gl.read_pixels(
                    cx,
                    cy,
                    1,
                    1,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    PixelPackData::Slice(Some(&mut px)),
                );
                let mut c1 = [0u8; 4];
                let mut c2 = [0u8; 4];
                gl.read_pixels(8, 8, 1, 1, glow::RGBA, glow::UNSIGNED_BYTE, PixelPackData::Slice(Some(&mut c1)));
                gl.read_pixels(
                    (w.saturating_sub(9)) as i32,
                    (h.saturating_sub(9)) as i32,
                    1,
                    1,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    PixelPackData::Slice(Some(&mut c2)),
                );
                eprintln!("tish-shadertoy: read_pixels center ({cx},{cy}) RGBA={px:?}  near-bl-lr {c1:?} {c2:?}");
            }
        }

        Ok(())
    }
}
