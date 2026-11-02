//! Thread-local pumpable GL viewer for **native compile** and the tree-walking runner.
//!
//! `openPumpable` + `while (pump()) { }` returns control to Tish between frames. **`onKeyDown`**
//! only works when the handler is a compiled `Value::Function` (Rust closure); the interpreter
//! cannot invoke AST `fn` bodies from winit yet.

use std::cell::RefCell;
use std::time::Duration;

use glutin::config::ConfigTemplateBuilder;
use glutin_winit::DisplayBuilder;
use tish_winint::{GlWindowApp, GlWindowHost, PumpStatus};
use tishlang_core::{tish_module, Value};

use crate::ShadertoyHost;

/// Max time to block inside winit waiting for the next event when the script does not pass an explicit timeout.
///
/// **`while (pump())`** with **`Duration::ZERO`** spins the CPU and often looks **choppy** despite a high FPS readout.
/// Default **1 ms** yields the thread and lets the compositor/GPU pace work. Set **`TISH_SHADER_PUMP_TIMEOUT_MS=0`**
/// to restore zero-wait polling (busy loop).
pub fn pump_wait_duration() -> Duration {
    match std::env::var("TISH_SHADER_PUMP_TIMEOUT_MS") {
        Ok(s) if s.trim() == "0" => Duration::ZERO,
        Ok(s) => s
            .trim()
            .parse::<u64>()
            .map(Duration::from_millis)
            .unwrap_or(Duration::from_millis(1)),
        Err(_) => Duration::from_millis(1),
    }
}

fn pump_timeout_from_args(args: &[Value]) -> Duration {
    match args.first() {
        Some(Value::Number(n)) if n.is_finite() && *n >= 0.0 => {
            if *n <= 0.0 {
                Duration::ZERO
            } else {
                Duration::from_secs_f64(*n / 1000.0)
            }
        }
        _ => pump_wait_duration(),
    }
}

thread_local! {
    static PUMP: RefCell<Option<PumpState>> = RefCell::new(None);
}

struct PumpState {
    event_loop: winit::event_loop::EventLoop<()>,
    app: GlWindowApp<ShadertoyHost>,
}

fn take_on_key_from_options(arg: Option<&Value>) -> Option<tishlang_core::NativeFn> {
    let opt = arg?;
    let Value::Object(o) = opt else {
        return None;
    };
    let o = o.borrow();
    let v = o.strings.get(&std::sync::Arc::from("onKeyDown"))?;
    match v {
        Value::Function(f) => Some(f.clone()),
        Value::Null => None,
        _ => None,
    }
}

/// `openPumpable(shader, options?)` — `options.onKeyDown` = compiled function `(e) => …` with `e.key`, `e.repeat`.
pub fn open_pumpable(args: &[Value]) -> Value {
    let code = args.first().and_then(|v| match v {
        Value::String(s) if !s.is_empty() => Some(s.to_string()),
        _ => None,
    });
    let Some(code) = code else {
        eprintln!("tish:shadertoy.openPumpable: expected non-empty shader source string");
        return Value::Bool(false);
    };
    let on_key = take_on_key_from_options(args.get(1));
    open_pumpable_inner(code, on_key)
}

pub(crate) fn open_pumpable_inner(code: String, on_key: Option<tishlang_core::NativeFn>) -> Value {
    let already = PUMP.with(|c| c.borrow().is_some());
    if already {
        eprintln!("tish:shadertoy.openPumpable: a pumpable session is already open");
        return Value::Bool(false);
    }

    let event_loop = match winit::event_loop::EventLoop::new() {
        Ok(el) => el,
        Err(e) => {
            eprintln!("tish:shadertoy.openPumpable: {e}");
            return Value::Bool(false);
        }
    };
    let template = ConfigTemplateBuilder::new().with_alpha_size(8);
    let host = ShadertoyHost::new(code).with_on_key_down(on_key);
    let attrs = host.window_attributes();
    let display_builder = DisplayBuilder::new().with_window_attributes(Some(attrs));
    let vsync = !matches!(
        std::env::var("TISH_SHADER_VSYNC").as_deref(),
        Ok("0" | "false" | "off")
    );
    let app = GlWindowApp::new(template, display_builder, host, vsync);
    PUMP.with(|c| {
        *c.borrow_mut() = Some(PumpState { event_loop, app });
    });
    Value::Bool(true)
}

/// Process winit/GL for a short slice. Returns **`true`** while the window is open, **`false`** after close or if no session.
///
/// Optional first argument: **wait milliseconds** (`Number`, ≥ 0). **`0`** = non-blocking poll (busy `while` will peg CPU).
/// Omitted → [`pump_wait_duration`] (default **1 ms** unless `TISH_SHADER_PUMP_TIMEOUT_MS` overrides).
pub fn pump(args: &[Value]) -> Value {
    let timeout = pump_timeout_from_args(args);
    PUMP.with(|cell| {
        let mut b = cell.borrow_mut();
        let Some(sess) = b.as_mut() else {
            return Value::Bool(false);
        };
        match sess.app.pump_events(&mut sess.event_loop, timeout) {
            PumpStatus::Exit(_) => {
                let _ = sess.app.take_exit_result();
                *b = None;
                Value::Bool(false)
            }
            PumpStatus::Continue => Value::Bool(true),
        }
    })
}

/// `setWindowTitle(title)` — no-op if the window is not created yet.
pub fn set_window_title(args: &[Value]) -> Value {
    let title = args
        .first()
        .map(|v| v.to_display_string())
        .unwrap_or_default();
    if title.is_empty() {
        return Value::Null;
    }
    PUMP.with(|cell| {
        let b = cell.borrow();
        let Some(sess) = b.as_ref() else {
            return Value::Bool(false);
        };
        if let Some(w) = sess.app.window() {
            w.set_title(&title);
            Value::Bool(true)
        } else {
            Value::Bool(false)
        }
    })
}

/// `reloadShader(source)` — rebuild programs on the GL thread; returns `false` on compile/link error.
pub fn reload_shader(args: &[Value]) -> Value {
    let src = args.first().and_then(|v| match v {
        Value::String(s) if !s.is_empty() => Some(s.to_string()),
        _ => None,
    });
    let Some(src) = src else {
        eprintln!("tish:shadertoy.reloadShader: expected non-empty source string");
        return Value::Bool(false);
    };
    PUMP.with(|cell| {
        let mut b = cell.borrow_mut();
        let Some(sess) = b.as_mut() else {
            return Value::Bool(false);
        };
        let Some((host, gl)) = sess.app.host_and_gl_mut() else {
            return Value::Bool(false);
        };
        match host.reload_shader_source(gl.as_ref(), src) {
            Ok(()) => Value::Bool(true),
            Err(e) => {
                eprintln!("tish:shadertoy.reloadShader: {e}");
                Value::Bool(false)
            }
        }
    })
}

/// Namespace for **`tish build`** (`package.json` → `tish.module`).
pub fn shadertoy_object() -> Value {
    tish_module! {
        "run" => shadertoy_run_blocking,
        "openPumpable" => open_pumpable,
        "pump" => pump,
        "setWindowTitle" => set_window_title,
        "reloadShader" => reload_shader,
    }
}

fn shadertoy_run_blocking(args: &[Value]) -> Value {
    let code = args.first().and_then(|v| match v {
        Value::String(s) if !s.is_empty() => Some(s.to_string()),
        _ => None,
    });
    let Some(code) = code else {
        eprintln!("tish:shadertoy.run: expected non-empty shader source string");
        return Value::Null;
    };
    if let Err(e) = crate::run_viewer(&code) {
        eprintln!("tish:shadertoy.run: {e}");
    }
    Value::Null
}
