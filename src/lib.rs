//! Native `tish:shadertoy` module: winit + glutin + glow fullscreen fragment viewer.

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
use winit::dpi::LogicalSize;
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
    src.lines()
        .filter(|line| {
            let t = line.trim_start();
            !(t.starts_with("#version") || t.starts_with("precision"))
        })
        .collect::<Vec<_>>()
        .join("\n")
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
uniform sampler2D iChannel0;
uniform sampler2D iChannel1;
uniform sampler2D iChannel2;
uniform sampler2D iChannel3;
#define iGlobalTime iTime

{user}

void main() {{
    mainImage(fragColor, gl_FragCoord.xy);
}}
"#
    )
}

fn gl_config_picker(configs: Box<dyn Iterator<Item = Config> + '_>) -> Config {
    configs
        .reduce(|accum, config| {
            let transparency_check = config.supports_transparency().unwrap_or(false)
                & !accum.supports_transparency().unwrap_or(false);
            if transparency_check || config.num_samples() > accum.num_samples() {
                config
            } else {
                accum
            }
        })
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

fn window_attributes() -> WindowAttributes {
    Window::default_attributes()
        .with_title("Tish Shadertoy")
        .with_inner_size(LogicalSize::new(1280.0, 720.0))
        .with_min_inner_size(LogicalSize::new(1.0, 1.0))
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
    dummy_textures: Vec<glow::Texture>,
    start: Option<Instant>,
    last_frame: Option<Instant>,
    frame_idx: i32,
    mouse_xy: (f32, f32),
    mouse_click_xy: (f32, f32),
    mouse_buttons: (f32, f32),
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
            dummy_textures: Vec::new(),
            start: None,
            last_frame: None,
            frame_idx: -1,
            mouse_xy: (-1.0, -1.0),
            mouse_click_xy: (-1.0, -1.0),
            mouse_buttons: (0.0, 0.0),
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
            gl.delete_shader(vs);
            gl.delete_shader(fs);
            if !gl.get_program_link_status(program) {
                let log = gl.get_program_info_log(program);
                gl.delete_program(program);
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

            let vao = gl
                .create_vertex_array()
                .map_err(|e| format!("create_vertex_array: {e}"))?;
            gl.bind_vertex_array(Some(vao));
            self.vao = Some(vao);

            let mut tex = Vec::new();
            for _ in 0..4 {
                let t = gl
                    .create_texture()
                    .map_err(|e| format!("texture: {e}"))?;
                gl.bind_texture(glow::TEXTURE_2D, Some(t));
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
            self.dummy_textures = tex;
        }
        Ok(())
    }

    fn draw_frame(&mut self, gl: &Rc<glow::Context>, w: u32, h: u32) {
        let now = Instant::now();
        let start = *self.start.get_or_insert(now);
        let i_time = now.duration_since(start).as_secs_f32();
        let i_time_delta = self
            .last_frame
            .map(|p| now.duration_since(p).as_secs_f32())
            .unwrap_or(0.0);
        self.last_frame = Some(now);
        self.frame_idx = self.frame_idx.saturating_add(1);

        let (mx, my) = self.mouse_xy;
        let (cx, cy) = self.mouse_click_xy;

        let local = chrono::Local::now();
        let year = local.year() as f32;
        let month = local.month() as f32;
        let day = local.day() as f32;
        let sec_in_day = (local.hour() as f32) * 3600.0
            + (local.minute() as f32) * 60.0
            + local.second() as f32;

        let Some(program) = self.program else {
            return;
        };
        let loc_resolution = self.loc_resolution.as_ref();
        let loc_time = self.loc_time.as_ref();
        let loc_time_delta = self.loc_time_delta.as_ref();
        let loc_frame = self.loc_frame.as_ref();
        let loc_mouse = self.loc_mouse.as_ref();
        let loc_date = self.loc_date.as_ref();
        let loc_sample_rate = self.loc_sample_rate.as_ref();
        let loc_chan_res = self.loc_chan_res.as_ref();

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

        unsafe {
            gl.viewport(0, 0, w as i32, h as i32);
            gl.use_program(Some(program));

            if let Some(loc) = loc_resolution {
                gl.uniform_3_f32(Some(loc), w as f32, h as f32, 1.0);
            }
            if let Some(loc) = loc_time {
                gl.uniform_1_f32(Some(loc), i_time);
            }
            if let Some(loc) = loc_time_delta {
                gl.uniform_1_f32(Some(loc), i_time_delta);
            }
            if let Some(loc) = loc_frame {
                gl.uniform_1_i32(Some(loc), self.frame_idx);
            }
            if let Some(loc) = loc_mouse {
                gl.uniform_4_f32(Some(loc), mx, my, mz, mw);
            }
            if let Some(loc) = loc_date {
                gl.uniform_4_f32(Some(loc), year, month, day, sec_in_day);
            }
            if let Some(loc) = loc_sample_rate {
                gl.uniform_1_f32(Some(loc), 44100.0);
            }
            if let Some(loc) = loc_chan_res {
                let v = [1.0f32; 12];
                gl.uniform_3_f32_slice(Some(loc), &v);
            }

            for (unit, tex) in self.dummy_textures.iter().enumerate() {
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

        if let Err(e) = gl_surface.set_swap_interval(
            self.gl_context.as_ref().unwrap(),
            SwapInterval::Wait(NonZeroU32::new(1).unwrap()),
        ) {
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
                unsafe {
                    gl.clear_color(0.0, 0.0, 0.0, 1.0);
                    gl.clear(glow::COLOR_BUFFER_BIT);
                }
                self.draw_frame(&gl, ww, hh);
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
                    if let Some(v) = self.vao.take() {
                        gl.delete_vertex_array(v);
                    }
                    if let Some(p) = self.program.take() {
                        gl.delete_program(p);
                    }
                    for t in self.dummy_textures.drain(..) {
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
