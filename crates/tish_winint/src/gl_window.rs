//! OpenGL window shell: glutin display, surface, context, and [`winit::application::ApplicationHandler`].

use std::ffi::CString;
use std::num::NonZeroU32;
use std::rc::Rc;

use glow::HasContext;
use glutin::config::{Config, ConfigTemplateBuilder, GetGlConfig};
use glutin::context::{ContextApi, ContextAttributesBuilder, PossiblyCurrentContext, Version};
use glutin::display::GetGlDisplay;
use glutin::prelude::*;
use glutin::surface::{Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use glutin_winit::{DisplayBuilder, GlWindow};
use raw_window_handle::HasWindowHandle;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes};

/// Prefer **no MSAA**: full-screen fragment work; high sample counts multiply cost.
pub fn gl_config_picker(configs: Box<dyn Iterator<Item = Config> + '_>) -> Config {
    configs
        .min_by_key(|c| c.num_samples())
        .expect("no GL config")
}

pub fn create_gl_context(
    window: &Window,
    gl_config: &Config,
) -> glutin::context::NotCurrentContext {
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

/// Backing size in **physical pixels**.
pub fn window_physical_size(
    width_var: &str,
    height_var: &str,
    default_w: u32,
    default_h: u32,
) -> PhysicalSize<u32> {
    let w = std::env::var(width_var)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default_w)
        .max(1);
    let h = std::env::var(height_var)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default_h)
        .max(1);
    PhysicalSize::new(w, h)
}

pub fn window_attributes(title: &str, size: PhysicalSize<u32>) -> WindowAttributes {
    Window::default_attributes()
        .with_title(title.to_string())
        .with_inner_size(size)
        .with_min_inner_size(PhysicalSize::new(1, 1))
}

#[allow(clippy::large_enum_variant)] // one-shot init state; boxing adds noise for little gain
pub enum GlDisplayInit {
    Builder(DisplayBuilder),
    Done,
}

pub struct GlWindowSurfaceState {
    pub gl_surface: Surface<WindowSurface>,
    pub window: Window,
}

/// User logic for an OpenGL window (shaders, input, chrome).
pub trait GlWindowHost: 'static {
    fn window_attributes(&self) -> WindowAttributes;

    fn on_gl_ready(&mut self, gl: &Rc<glow::Context>) -> Result<(), String>;

    fn on_redraw(
        &mut self,
        gl: &Rc<glow::Context>,
        window: &Window,
        width: u32,
        height: u32,
    ) -> Result<(), String>;

    fn on_viewport_resized(&mut self, gl: &glow::Context, width: u32, height: u32);

    /// Keyboard, mouse, and any events not handled by [`GlWindowApp`].
    fn window_event(&mut self, event_loop: &ActiveEventLoop, window: &Window, event: WindowEvent);

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop, window: &Window);

    /// Called when the window app is dropped so hosts can delete GL objects before the context is destroyed.
    fn tear_down_gl(&mut self, _gl: &Rc<glow::Context>) {}
}

pub struct GlWindowApp<H: GlWindowHost> {
    pub display: GlDisplayInit,
    pub template: ConfigTemplateBuilder,
    pub host: H,
    gl_context: Option<PossiblyCurrentContext>,
    state: Option<GlWindowSurfaceState>,
    gl: Option<Rc<glow::Context>>,
    exit_result: Result<(), String>,
    vsync: bool,
}

impl<H: GlWindowHost> GlWindowApp<H> {
    pub fn new(
        template: ConfigTemplateBuilder,
        display_builder: DisplayBuilder,
        host: H,
        vsync: bool,
    ) -> Self {
        Self {
            display: GlDisplayInit::Builder(display_builder),
            template,
            host,
            gl_context: None,
            state: None,
            gl: None,
            exit_result: Ok(()),
            vsync,
        }
    }

    /// Take the last error (or `Ok(())`) after [`EventLoop::run_app`](winit::event_loop::EventLoop::run_app) returns.
    pub fn take_exit_result(&mut self) -> Result<(), String> {
        std::mem::replace(&mut self.exit_result, Ok(()))
    }

    pub fn exit_result_ref(&self) -> &Result<(), String> {
        &self.exit_result
    }

    /// Window after [`ApplicationHandler::resumed`] has created the surface (else `None`).
    pub fn window(&self) -> Option<&Window> {
        self.state.as_ref().map(|s| &s.window)
    }

    /// Mutable host + GL context after GL init (for reload, title tweaks from the host thread).
    pub fn host_and_gl_mut(&mut self) -> Option<(&mut H, &Rc<glow::Context>)> {
        let gl = self.gl.as_ref()?;
        Some((&mut self.host, gl))
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, msg: String) {
        self.exit_result = Err(msg);
        event_loop.exit();
    }

    /// Pump winit events (non-blocking when `timeout == Some(Duration::ZERO)`). See [`winit::platform::pump_events`].
    pub fn pump_events(
        &mut self,
        event_loop: &mut EventLoop<()>,
        timeout: std::time::Duration,
    ) -> winit::platform::pump_events::PumpStatus {
        use winit::platform::pump_events::EventLoopExtPumpEvents;
        event_loop.pump_app_events(Some(timeout), self)
    }
}

impl<H: GlWindowHost> Drop for GlWindowApp<H> {
    fn drop(&mut self) {
        if let Some(gl) = self.gl.take() {
            self.host.tear_down_gl(&gl);
        }
    }
}

impl<H: GlWindowHost> ApplicationHandler for GlWindowApp<H> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let (window, gl_config) = match std::mem::replace(&mut self.display, GlDisplayInit::Done) {
            GlDisplayInit::Builder(display_builder) => {
                let (window, gl_config) = match display_builder.build(
                    event_loop,
                    self.template.clone(),
                    gl_config_picker,
                ) {
                    Ok((Some(w), c)) => (w, c),
                    Ok((None, _)) => {
                        self.fail(event_loop, "GL display build returned no window".into());
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
            GlDisplayInit::Done => {
                let gl_config = match self.gl_context.as_ref() {
                    Some(c) => c.config().clone(),
                    None => {
                        self.fail(event_loop, "missing GL context".into());
                        return;
                    }
                };
                match glutin_winit::finalize_window(
                    event_loop,
                    self.host.window_attributes(),
                    &gl_config,
                ) {
                    Ok(w) => (w, gl_config),
                    Err(e) => {
                        self.fail(event_loop, format!("window: {e}"));
                        return;
                    }
                }
            }
        };

        let attrs = match window
            .build_surface_attributes(SurfaceAttributesBuilder::<WindowSurface>::new())
        {
            Ok(a) => a,
            Err(e) => {
                self.fail(event_loop, format!("surface attrs: {e}"));
                return;
            }
        };
        let gl_surface = match unsafe {
            gl_config
                .display()
                .create_window_surface(&gl_config, &attrs)
        } {
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

        let gl_rc = Rc::new(gl);
        if let Err(msg) = self.host.on_gl_ready(&gl_rc) {
            self.fail(event_loop, msg);
            return;
        }
        self.gl = Some(gl_rc);

        let interval = if self.vsync {
            SwapInterval::Wait(NonZeroU32::new(1).unwrap())
        } else {
            SwapInterval::DontWait
        };
        if let Err(e) = gl_surface.set_swap_interval(self.gl_context.as_ref().unwrap(), interval) {
            eprintln!("swap interval: {e:?}");
        }

        assert!(self
            .state
            .replace(GlWindowSurfaceState { gl_surface, window })
            .is_none());
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => {
                let Some(st) = self.state.as_ref() else {
                    return;
                };
                let sz = st.window.inner_size();
                let (ww, hh) = (sz.width, sz.height);
                if ww == 0 || hh == 0 {
                    return;
                }
                let Some(gl) = self.gl.clone() else {
                    return;
                };
                if let Err(e) = self.host.on_redraw(&gl, &st.window, ww, hh) {
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
                            self.host.on_viewport_resized(gl, size.width, size.height);
                        }
                    }
                }
            }
            other => {
                if let Some(s) = self.state.as_ref() {
                    self.host.window_event(event_loop, &s.window, other);
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(s) = self.state.as_ref() {
            self.host.about_to_wait(event_loop, &s.window);
        }
        event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
    }
}
