//! Native `tish:shadertoy` and `tish:winint` modules: GL fragment viewer + reusable window shell ([`tish_winint`]).

mod channels;
mod compat;
mod pump_session;
mod renderer;

/// Local workspace crate: window + GL shell ([`tish:winint`](https://github.com/spacedevin/tish-shadertoy/tree/main/crates/tish_winint)).
pub use tish_winint;

pub use pump_session::shadertoy_object;
pub use renderer::{ShadertoyInput, ShadertoyRenderer};

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use glutin::config::ConfigTemplateBuilder;
use glutin_winit::DisplayBuilder;
use tish_winint::{load_hotkeys, GlWindowApp, GlWindowHost, HotkeysLoadOptions};
use tishlang_core::NativeFn as CoreNativeFn;
use tishlang_eval::value_convert::{core_to_eval, eval_to_core};
use tishlang_eval::{PropMap, TishNativeModule, Value as EvalValue};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowAttributes};

/// Used by [`build_fragment_source`] logging and [`renderer::ShadertoyRenderer`].
pub(crate) fn framebuffer_srgb_write_enabled() -> bool {
    matches!(
        std::env::var("TISH_SHADER_FRAMEBUFFER_SRGB").as_deref(),
        Ok("1" | "true" | "on")
    )
}

/// Built-in `smoothstep` is **undefined** when `edge0 >= edge1`; native GL may NaN. Many stacks
/// (incl. WebGL) effectively use **`1 - smoothstep(e1, e0, x)`** when `edge0 > edge1`; swapping
/// edges to `(lo,hi)` and running the Hermite there does **not** match that and can push values
/// (e.g. heavy subtracts) far **negative** → `tanh` + UNORM clamp → all black.
const SMOOTHSTEP_DESKTOP_SHIM: &str = r#"
float tish_smoothstep(float e0, float e1, float x) {
    if (e0 < e1) {
        float d = e1 - e0;
        float t = clamp((x - e0) / d, 0.0, 1.0);
        return t * t * (3.0 - 2.0 * t);
    } else if (e1 < e0) {
        float d = e0 - e1;
        float t = clamp((x - e1) / d, 0.0, 1.0);
        float s = t * t * (3.0 - 2.0 * t);
        return 1.0 - s;
    }
    return step(e0, x);
}
vec2 tish_smoothstep(vec2 e0, vec2 e1, vec2 x) {
    return vec2(
        tish_smoothstep(e0.x, e1.x, x.x),
        tish_smoothstep(e0.y, e1.y, x.y));
}
vec3 tish_smoothstep(vec3 e0, vec3 e1, vec3 x) {
    return vec3(
        tish_smoothstep(e0.x, e1.x, x.x),
        tish_smoothstep(e0.y, e1.y, x.y),
        tish_smoothstep(e0.z, e1.z, x.z));
}
vec4 tish_smoothstep(vec4 e0, vec4 e1, vec4 x) {
    return vec4(
        tish_smoothstep(e0.x, e1.x, x.x),
        tish_smoothstep(e0.y, e1.y, x.y),
        tish_smoothstep(e0.z, e1.z, x.z),
        tish_smoothstep(e0.w, e1.w, x.w));
}
#define smoothstep tish_smoothstep
"#;

/// Optional rational `tanh` (**`TISH_SHADER_COMPAT_TANH=1`**), **`#define tanh tish_tanh`**.
///
/// **Default (no env):** the **driver’s** built-in `tanh` — not a separate host implementation.
/// “Oversaturation” vs using only `o/(k*d)` is usually the **tanh curve** (large inputs → ~±1), not a Metal bug.
/// For **huge** inputs, built-in `tanh` can return NaN on some drivers; the rational form avoids that
/// (see `shaders/scXSRf.glsl`). **`vec4`** passes **`.w` through** so tone-mapped finals don’t also warp alpha
/// before the wrapper sets `a = 1`.
const TANH_DESKTOP_SHIM: &str = r#"
float tish_tanh(float x) {
    x = clamp(x, -80.0, 80.0);
    float x2 = x * x;
    return clamp(x * (27.0 + x2) / (27.0 + 9.0 * x2), -1.0, 1.0);
}
vec2 tish_tanh(vec2 x) {
    return vec2(tish_tanh(x.x), tish_tanh(x.y));
}
vec3 tish_tanh(vec3 x) {
    return vec3(tish_tanh(x.x), tish_tanh(x.y), tish_tanh(x.z));
}
vec4 tish_tanh(vec4 x) {
    return vec4(tish_tanh(x.x), tish_tanh(x.y), tish_tanh(x.z), x.w);
}
#define tanh tish_tanh
"#;

fn preprocess_user_code(src: &str) -> String {
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    let stripped: String = src
        .lines()
        .filter(|line| {
            let t = line.trim_start();
            !(t.starts_with("#version") || t.starts_with("precision"))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let stripped = compat::strip_float_f_suffix(&stripped);
    let stripped = compat::rewrite_mat2_cos_define_lines(&stripped);
    let stripped = compat::rewrite_main_image_out_vec4_to_inout(&stripped);
    let stripped = compat::preprocess_webgl_compat(&stripped);
    let stripped = compat::rewrite_for_loop_fragcolor_mul_init(&stripped);
    if compat::float_postincrement_enabled() {
        let names = compat::collect_float_declared_names(&stripped);
        compat::rewrite_float_post_increment(&stripped, &names)
    } else {
        stripped
    }
}

fn build_fragment_source(user: &str) -> String {
    let (user, host_pragmas) = compat::extract_host_pragmas(user);
    let user = preprocess_user_code(&user);
    let mat2_shim = if user.contains("tish_mat2_from_vec4") {
        eprintln!(
            "tish-shadertoy: R-macro mat2(cos(vec4)) expanded for Apple GL (tish_mat2_from_vec4)"
        );
        "mat2 tish_mat2_from_vec4(vec4 cs) { return mat2(cs.x, cs.y, cs.z, cs.w); }\n"
    } else {
        ""
    };
    let smooth = if compat::smoothstep_desktop_shim_enabled() {
        SMOOTHSTEP_DESKTOP_SHIM
    } else {
        ""
    };
    let tanh_shim = if compat::tanh_desktop_shim_enabled() {
        TANH_DESKTOP_SHIM
    } else {
        ""
    };
    let auto_tanh_map = compat::auto_tanh_fragcolor_display_map_enabled()
        && (user.contains("fragColor = tanh(") || user.contains("fragColor=tanh("));
    // Same GLSL line as `post_main_rgb_linear_clamp`; pragma path expects pre-tanh acc — do not double up.
    let post_main_tanh_map = if auto_tanh_map && !host_pragmas.post_main_rgb_linear_clamp {
        eprintln!("tish-shadertoy: TISH_SHADER_AUTO_TANH_MAP: rgb = rgb*0.5+0.5 (debug only; washes real output)");
        r#"
    _tish_acc.rgb = clamp(_tish_acc.rgb * 0.5 + vec3(0.5), 0.0, 1.0);"#
    } else {
        ""
    };
    let post_main_rgb_linear_clamp = if host_pragmas.post_main_rgb_linear_clamp
        && !matches!(
            std::env::var("TISH_SHADER_DISABLE_POST_MAIN_RGB_LINEAR_CLAMP").as_deref(),
            Ok("1" | "true" | "on")
        ) {
        eprintln!(
            "tish-shadertoy: host post-main rgb linear clamp (// tish:post-main-rgb-linear-clamp)"
        );
        r#"
    _tish_acc.rgb = clamp(_tish_acc.rgb * 0.5 + vec3(0.5), 0.0, 1.0);"#
    } else {
        ""
    };
    let mainimage_coord = if matches!(
        std::env::var("TISH_SHADER_FRAGCOORD_Y_FLIP").as_deref(),
        Ok("1" | "true" | "on")
    ) {
        eprintln!(
            "tish-shadertoy: mainImage uses flipped Y fragCoord (TISH_SHADER_FRAGCOORD_Y_FLIP=1)"
        );
        "vec2(gl_FragCoord.x, iResolution.y - gl_FragCoord.y)"
    } else if matches!(
        std::env::var("TISH_SHADER_FRAGCOORD_SUB_HALF").as_deref(),
        Ok("1" | "true" | "on")
    ) {
        eprintln!("tish-shadertoy: mainImage fragCoord uses pixel centers minus 0.5 (TISH_SHADER_FRAGCOORD_SUB_HALF=1)");
        "(gl_FragCoord.xy - vec2(0.5))"
    } else {
        "gl_FragCoord.xy"
    };
    let force_opaque_alpha = !matches!(
        std::env::var("TISH_SHADER_NO_FORCE_OPAQUE_ALPHA").as_deref(),
        Ok("1" | "true" | "on")
    );
    let alpha_fix = if force_opaque_alpha {
        r#"
    _tish_acc.a = 1.0;"#
    } else {
        ""
    };
    if matches!(std::env::var("TISH_SHADER_DEBUG").as_deref(), Ok("1")) {
        eprintln!(
            "tish-shadertoy: FS inject: smoothstep_preamble={} tanh_shim={} tanh_remap={} host_linear_clamp={} force_a={} \
             framebuffer_srgb_write={} (smoothstep only applies if the shader calls smoothstep(); \
             set TISH_SHADER_FRAMEBUFFER_SRGB=1 to enable sRGB encode on write)",
            !smooth.is_empty(),
            !tanh_shim.is_empty(),
            !post_main_tanh_map.is_empty(),
            !post_main_rgb_linear_clamp.is_empty(),
            force_opaque_alpha,
            framebuffer_srgb_write_enabled(),
        );
    }
    format!(
        r#"#version 330 core
layout(location = 0) out vec4 _tish_ColorOut;
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
#define iGlobalTime iTime
// Shadertoy injects this; required for many copied shaders (e.g. #if HW_PERFORMANCE==0).
#ifndef HW_PERFORMANCE
#define HW_PERFORMANCE 1
#endif
{smooth}
{tanh_shim}
{mat2_shim}
{user}

void main() {{
    // Do **not** pass the `layout(location=0) out` variable into `mainImage(out vec4 fragColor, …)`:
    // aliasing the same `out` with a function `out` parameter miscompiles on some GL drivers (e.g. Apple),
    // yielding all-black. Accumulate in a local, then assign once.
    vec4 _tish_acc = vec4(0.0, 0.0, 0.0, 1.0);
    mainImage(_tish_acc, {mainimage_coord});{post_main_tanh_map}{post_main_rgb_linear_clamp}{alpha_fix}
    _tish_ColorOut = _tish_acc;
}}
"#,
        smooth = smooth,
        tanh_shim = tanh_shim,
        mat2_shim = mat2_shim,
        user = user,
        post_main_tanh_map = post_main_tanh_map,
        post_main_rgb_linear_clamp = post_main_rgb_linear_clamp,
        alpha_fix = alpha_fix,
        mainimage_coord = mainimage_coord,
    )
}

#[cfg(test)]
mod built_fs_tests {
    static TEST_ENV_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();

    #[test]
    fn auto_tanh_display_map_only_when_env_opt_in() {
        let _g = TEST_ENV_LOCK
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap();
        let snippet = "void mainImage(out vec4 c, in vec2 u) { fragColor=tanh(c+c); }";
        std::env::remove_var("TISH_SHADER_AUTO_TANH_MAP");
        let fs = super::build_fragment_source(snippet);
        assert!(
            !fs.contains("clamp(_tish_acc.rgb * 0.5 + vec3(0.5)"),
            "default: no tanh→UNORM hack"
        );
        std::env::set_var("TISH_SHADER_AUTO_TANH_MAP", "1");
        let fs_on = super::build_fragment_source(snippet);
        assert!(
            fs_on.contains("clamp(_tish_acc.rgb * 0.5 + vec3(0.5)"),
            "opt-in: inject remap"
        );
        std::env::remove_var("TISH_SHADER_AUTO_TANH_MAP");
    }

    #[test]
    fn tanh_rational_shim_only_when_compat_tanh_opt_in() {
        let _g = TEST_ENV_LOCK
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap();
        let snippet =
            "void mainImage(out vec4 fragColor, in vec2 u) { fragColor = tanh(fragColor); }";
        std::env::remove_var("TISH_SHADER_COMPAT_TANH");
        let fs = super::build_fragment_source(snippet);
        assert!(
            !fs.contains("#define tanh tish_tanh"),
            "default: native GLSL tanh"
        );
        std::env::set_var("TISH_SHADER_COMPAT_TANH", "1");
        let fs_on = super::build_fragment_source(snippet);
        assert!(fs_on.contains("#define tanh tish_tanh"));
        assert!(
            fs_on.contains("x = clamp(x,"),
            "rational shim must clamp x before x*x (Metal inf/NaN)"
        );
        std::env::remove_var("TISH_SHADER_COMPAT_TANH");
    }

    #[test]
    fn post_main_rgb_linear_clamp_from_pragma() {
        let _g = TEST_ENV_LOCK
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap();
        std::env::remove_var("TISH_SHADER_DISABLE_POST_MAIN_RGB_LINEAR_CLAMP");
        let fs = super::build_fragment_source(
            "// tish:post-main-rgb-linear-clamp\nvoid mainImage(out vec4 c, in vec2 u) { c = vec4(1.0); }",
        );
        assert!(
            fs.contains("clamp(_tish_acc.rgb * 0.5 + vec3(0.5)"),
            "expected host linear clamp after mainImage; got excerpt:\n{}",
            fs.lines()
                .find(|l| l.contains("_tish_acc.rgb"))
                .unwrap_or("")
        );
    }

    #[test]
    fn fragment_never_injects_post_main_rgb_max() {
        let _g = TEST_ENV_LOCK
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap();
        let snippet = "void mainImage(out vec4 c, in vec2 u) { c = vec4(1.0); }";
        let fs = super::build_fragment_source(snippet);
        assert!(
            !fs.contains("max(_tish_acc.rgb"),
            "post-main max(rgb,0) was removed; it axes-splat signed HDR (e.g. flat red)"
        );
    }

    #[test]
    fn seven_cf_gzn_rewrites_float_loop_and_for_line_sane() {
        let code = include_str!("../shaders/7cfGzn.glsl");
        let fs = super::build_fragment_source(code);
        assert!(
            fs.contains("i = 0.0;") && fs.contains("for (;") && fs.contains("fragColor *= i"),
            "expected for-init compat rewrite (split lines); excerpt:\n{}",
            fs.lines()
                .enumerate()
                .skip(105)
                .take(40)
                .fold(String::new(), |mut a, (i, l)| {
                    use std::fmt::Write;
                    let _ = writeln!(a, "{}: {}", i + 1, l);
                    a
                })
        );
        assert!(
            fs.contains("i += 1.0"),
            "expected float loop rewrite; excerpt:\n{}",
            fs.lines()
                .enumerate()
                .skip(105)
                .take(30)
                .fold(String::new(), |mut a, (i, l)| {
                    use std::fmt::Write;
                    let _ = writeln!(a, "{}: {}", i + 1, l);
                    a
                })
        );
        assert!(
            !fs.lines()
                .any(|l| l.contains("i++") && !l.trim_start().starts_with("//")),
            "raw i++ should not appear outside comments"
        );
        assert!(
            !fs.contains("#define tanh tish_tanh"),
            "default: use native tanh for Shadertoy parity (rational shim is TISH_SHADER_COMPAT_TANH=1)"
        );
        assert!(
            fs.contains("tish_mat2_from_vec4") && fs.contains("#define R(a) tish_mat2_from_vec4"),
            "7cfGzn R macro must expand mat2(cos(vec4)) for Apple GL (Metal)"
        );
        assert!(
            fs.contains("void mainImage( inout vec4 fragColor"),
            "out vec4 does not receive caller seed on desktop GLSL; use inout for Metal parity"
        );
        assert!(
            !fs.contains("clamp(_tish_acc.rgb * 0.5 + vec3(0.5)"),
            "TISH_SHADER_AUTO_TANH_MAP is opt-in; do not remap tanh output by default"
        );
        assert!(
            !fs.contains("max(_tish_acc.rgb"),
            "wrapper must not inject per-channel rgb clamp after mainImage"
        );
    }

    #[test]
    fn fragment_main_seeds_opaque_black_before_main_image() {
        let fs = super::build_fragment_source(
            "void mainImage(out vec4 c, in vec2 u) { c = vec4(1.0); }",
        );
        assert!(
            fs.contains("vec4 _tish_acc = vec4(0.0, 0.0, 0.0, 1.0);"),
            "main() should seed accumulator (opaque black a=1) before mainImage"
        );
        assert!(
            fs.contains("mainImage(_tish_acc, gl_FragCoord.xy)"),
            "main() must not alias global out with mainImage out param (driver bug)"
        );
        assert!(
            fs.contains("_tish_ColorOut = _tish_acc"),
            "main() should write layout(location=0) once from local accumulator"
        );
    }
}

const HOTKEY_LOAD: HotkeysLoadOptions = HotkeysLoadOptions {
    path_env_var: "TISH_SHADER_HOTKEYS_PATH",
    default_path: "hotkeys.toml",
    log_prefix: "tish-shadertoy",
};

/// Shadertoy viewer logic + input; paired with [`GlWindowApp`](tish_winint::GlWindowApp).
pub struct ShadertoyHost {
    renderer: ShadertoyRenderer,
    hotkeys: tish_winint::HotkeysConfig,
    mouse_xy: (f32, f32),
    mouse_click_xy: (f32, f32),
    mouse_buttons: (f32, f32),
    show_fps_in_title: bool,
    /// Native compile: `Value::Function` from `openPumpable(..., { onKeyDown })`. Unset in tree-walk interpreter if `onKeyDown` is a Tish `fn` (AST).
    pub on_key_down: Option<CoreNativeFn>,
}

impl ShadertoyHost {
    pub(crate) fn new(shader_source: String) -> Self {
        Self {
            renderer: ShadertoyRenderer::new(shader_source),
            hotkeys: load_hotkeys(&HOTKEY_LOAD),
            mouse_xy: (-1.0, -1.0),
            mouse_click_xy: (-1.0, -1.0),
            mouse_buttons: (0.0, 0.0),
            show_fps_in_title: false,
            on_key_down: None,
        }
    }

    pub(crate) fn with_on_key_down(mut self, cb: Option<CoreNativeFn>) -> Self {
        self.on_key_down = cb;
        self
    }

    pub(crate) fn keyboard_event_value(logical: &Key, repeat: bool) -> tishlang_core::Value {
        use std::sync::Arc;
        use tishlang_core::{ArcStr, ObjectMap, Value as Cv};
        let key_str: ArcStr = match logical {
            Key::Character(s) => s.as_str().into(),
            Key::Named(_) => format!("{:?}", logical).into(),
            Key::Dead(_) => "Dead".into(),
            Key::Unidentified(_) => "".into(),
        };
        let mut m = ObjectMap::default();
        m.insert(Arc::from("key"), Cv::String(key_str));
        m.insert(Arc::from("repeat"), Cv::Bool(repeat));
        Cv::object(m)
    }

    fn input_snapshot(&self) -> ShadertoyInput {
        ShadertoyInput {
            mouse_xy: self.mouse_xy,
            mouse_click_xy: self.mouse_click_xy,
            mouse_buttons: self.mouse_buttons,
        }
    }

    /// Hot-reload fragment shader on the GL thread (`pump` / redraw path).
    pub fn reload_shader_source(&mut self, gl: &glow::Context, src: String) -> Result<(), String> {
        self.renderer.reload(gl, src)
    }

    fn sync_window_title(&self, window: &Window) {
        if self.show_fps_in_title {
            let t = format!("Tish Shadertoy — {:.1} FPS", self.renderer.fps_ema());
            window.set_title(&t);
        } else {
            window.set_title("Tish Shadertoy");
        }
    }
}

impl GlWindowHost for ShadertoyHost {
    fn window_attributes(&self) -> WindowAttributes {
        tish_winint::window_attributes(
            "Tish Shadertoy",
            tish_winint::window_physical_size("TISH_SHADER_WIDTH", "TISH_SHADER_HEIGHT", 1280, 720),
        )
    }

    fn on_gl_ready(&mut self, gl: &Rc<glow::Context>) -> Result<(), String> {
        self.renderer.init(gl)
    }

    fn on_redraw(
        &mut self,
        gl: &Rc<glow::Context>,
        window: &Window,
        w: u32,
        h: u32,
    ) -> Result<(), String> {
        let input = self.input_snapshot();
        self.renderer.draw_frame(gl, w, h, &input)?;
        if self.show_fps_in_title {
            self.sync_window_title(window);
        }
        Ok(())
    }

    fn on_viewport_resized(&mut self, _gl: &glow::Context, _width: u32, _height: u32) {}

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window: &Window, event: WindowEvent) {
        match event {
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed {
                    if let Some(f) = &self.on_key_down {
                        let ev = Self::keyboard_event_value(&event.logical_key, event.repeat);
                        let _ = f.call(std::slice::from_ref(&ev));
                    }
                    if event.logical_key == Key::Named(NamedKey::Escape) {
                        event_loop.exit();
                        return;
                    }
                    if !event.repeat
                        && tish_winint::key_matches(&event.logical_key, &self.hotkeys.fps_toggle)
                    {
                        self.show_fps_in_title = !self.show_fps_in_title;
                        self.sync_window_title(window);
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let ps = window.inner_size();
                self.mouse_xy = (position.x as f32, (ps.height as f32) - position.y as f32);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let v = if state == ElementState::Pressed {
                    1.0
                } else {
                    0.0
                };
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
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop, window: &Window) {
        window.request_redraw();
    }

    fn tear_down_gl(&mut self, gl: &Rc<glow::Context>) {
        self.renderer.tear_down_gl(gl.as_ref());
    }
}

/// Run the viewer until the window closes. Blocks the Tish interpreter thread.
pub fn run_viewer(shader_code: &str) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    let template = ConfigTemplateBuilder::new().with_alpha_size(8);
    let host = ShadertoyHost::new(shader_code.to_string());
    let attrs = host.window_attributes();
    let display_builder = DisplayBuilder::new().with_window_attributes(Some(attrs));
    let vsync = !matches!(
        std::env::var("TISH_SHADER_VSYNC").as_deref(),
        Ok("0" | "false" | "off")
    );
    let mut app = GlWindowApp::new(template, display_builder, host, vsync);
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    app.take_exit_result()
}

/// Pump winit with **zero timeout** between frames and call `on_idle` each time the loop continues.
/// Same window lifecycle as [`run_viewer`], but allows embedding other work between pumps (Rust-side).
pub fn run_viewer_with_idle(shader_code: &str, mut on_idle: impl FnMut()) -> Result<(), String> {
    use tish_winint::PumpStatus;

    let mut event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    let template = ConfigTemplateBuilder::new().with_alpha_size(8);
    let host = ShadertoyHost::new(shader_code.to_string());
    let attrs = host.window_attributes();
    let display_builder = DisplayBuilder::new().with_window_attributes(Some(attrs));
    let vsync = !matches!(
        std::env::var("TISH_SHADER_VSYNC").as_deref(),
        Ok("0" | "false" | "off")
    );
    let mut app = GlWindowApp::new(template, display_builder, host, vsync);
    loop {
        match app.pump_events(&mut event_loop, crate::pump_session::pump_wait_duration()) {
            PumpStatus::Exit(_) => break,
            PumpStatus::Continue => on_idle(),
        }
    }
    app.take_exit_result()
}

fn winint_version(_args: &[EvalValue]) -> Result<EvalValue, String> {
    Ok(EvalValue::String("0.1.0".into()))
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

fn shadertoy_open_pumpable_eval(args: &[EvalValue]) -> Result<EvalValue, String> {
    let code = args
        .first()
        .map(|v| v.to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "openPumpable: expected non-empty shader string".to_string())?;
    let on_key = match args.get(1) {
        None | Some(EvalValue::Null) => None,
        Some(EvalValue::Object(o)) => {
            let v = o
                .borrow()
                .strings
                .get(&Arc::from("onKeyDown"))
                .cloned()
                .unwrap_or(EvalValue::Null);
            match eval_to_core(&v) {
                Ok(tishlang_core::Value::Function(f)) => Some(f),
                Ok(tishlang_core::Value::Null) => None,
                Ok(_) => {
                    return Err("openPumpable: onKeyDown must be a function or null".to_string());
                }
                Err(_) => {
                    eprintln!(
                        "tish:shadertoy: onKeyDown (tree-walk fn) unsupported in tish-shadertoy-run; omit or use `tish build`."
                    );
                    None
                }
            }
        }
        Some(_) => {
            return Err(
                "openPumpable: second argument must be null or an options object".to_string(),
            );
        }
    };
    Ok(core_to_eval(crate::pump_session::open_pumpable_inner(
        code, on_key,
    )))
}

fn shadertoy_pump_eval(args: &[EvalValue]) -> Result<EvalValue, String> {
    let ca: Vec<tishlang_core::Value> = args.iter().map(eval_to_core).collect::<Result<_, _>>()?;
    Ok(core_to_eval(crate::pump_session::pump(&ca)))
}

fn shadertoy_set_window_title_eval(args: &[EvalValue]) -> Result<EvalValue, String> {
    let ca: Vec<tishlang_core::Value> = args.iter().map(eval_to_core).collect::<Result<_, _>>()?;
    Ok(core_to_eval(crate::pump_session::set_window_title(&ca)))
}

fn shadertoy_reload_shader_eval(args: &[EvalValue]) -> Result<EvalValue, String> {
    let ca: Vec<tishlang_core::Value> = args.iter().map(eval_to_core).collect::<Result<_, _>>()?;
    Ok(core_to_eval(crate::pump_session::reload_shader(&ca)))
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
        exports.insert(
            "openPumpable".into(),
            EvalValue::Native(shadertoy_open_pumpable_eval),
        );
        exports.insert("pump".into(), EvalValue::Native(shadertoy_pump_eval));
        exports.insert(
            "setWindowTitle".into(),
            EvalValue::Native(shadertoy_set_window_title_eval),
        );
        exports.insert(
            "reloadShader".into(),
            EvalValue::Native(shadertoy_reload_shader_eval),
        );
        vec![("tish:shadertoy", EvalValue::object(exports))]
    }
}

/// `import { version } from 'tish:winint'` — window/GL shell crate ([`tish_winint`]).
pub struct WinintModule;

impl TishNativeModule for WinintModule {
    fn name(&self) -> &'static str {
        "Winint"
    }

    fn register(&self) -> HashMap<Arc<str>, EvalValue> {
        HashMap::new()
    }

    fn virtual_builtin_modules(&self) -> Vec<(&'static str, EvalValue)> {
        let mut exports = PropMap::default();
        exports.insert("version".into(), EvalValue::Native(winint_version));
        vec![("tish:winint", EvalValue::object(exports))]
    }
}
