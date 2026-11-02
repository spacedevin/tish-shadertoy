//! winit + glutin + glow window shell for Tish embedders (`tish:winint`).

pub mod gl_window;
pub mod hotkeys;

pub use gl_window::{
    create_gl_context, gl_config_picker, window_attributes, window_physical_size, GlDisplayInit,
    GlWindowApp, GlWindowHost, GlWindowSurfaceState,
};
pub use hotkeys::{key_matches, load as load_hotkeys, HotkeysConfig, HotkeysLoadOptions};
pub use winit::platform::pump_events::{EventLoopExtPumpEvents, PumpStatus};
