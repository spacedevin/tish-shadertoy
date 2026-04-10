//! Keyboard bindings from a TOML file (Milkdrop-style).

use std::path::Path;

use serde::Deserialize;
use winit::keyboard::{Key, NamedKey};

/// Where to load hotkeys from and how to log parse errors.
#[derive(Debug, Clone)]
pub struct HotkeysLoadOptions {
    /// Env var for override path (e.g. `TISH_SHADER_HOTKEYS_PATH`).
    pub path_env_var: &'static str,
    /// Used when the env var is unset (e.g. `hotkeys.toml`).
    pub default_path: &'static str,
    /// Prefix for `eprintln!` messages (e.g. `tish-shadertoy`).
    pub log_prefix: &'static str,
}

#[derive(Debug, Deserialize, Default)]
struct FileRoot {
    #[serde(default)]
    fps: FpsSection,
}

#[derive(Debug, Deserialize)]
struct FpsSection {
    #[serde(default = "default_fps_toggle")]
    toggle: String,
}

impl Default for FpsSection {
    fn default() -> Self {
        Self {
            toggle: default_fps_toggle(),
        }
    }
}

fn default_fps_toggle() -> String {
    "F3".to_string()
}

#[derive(Debug, Clone)]
pub struct HotkeysConfig {
    pub fps_toggle: String,
}

impl Default for HotkeysConfig {
    fn default() -> Self {
        Self {
            fps_toggle: default_fps_toggle(),
        }
    }
}

impl From<FileRoot> for HotkeysConfig {
    fn from(f: FileRoot) -> Self {
        Self {
            fps_toggle: f.fps.toggle,
        }
    }
}

pub fn load(opts: &HotkeysLoadOptions) -> HotkeysConfig {
    let path_str =
        std::env::var(opts.path_env_var).unwrap_or_else(|_| opts.default_path.to_string());
    let path_trim = path_str.trim().to_string();
    let path = Path::new(&path_trim);
    if path.is_file() {
        match std::fs::read_to_string(path) {
            Ok(s) => match toml::from_str::<FileRoot>(&s) {
                Ok(root) => return root.into(),
                Err(e) => eprintln!(
                    "{}: {}: parse error: {e}",
                    opts.log_prefix,
                    path.display()
                ),
            },
            Err(e) => eprintln!("{}: {}: {e}", opts.log_prefix, path.display()),
        }
    }
    HotkeysConfig::default()
}

/// Match a logical key against a config string (e.g. `"F3"`, `"escape"`, `"f"`).
pub fn key_matches(key: &Key, spec: &str) -> bool {
    let s = spec.trim();
    if s.is_empty() {
        return false;
    }
    let lower = s.to_ascii_lowercase();

    if let Key::Named(nk) = key {
        return match lower.as_str() {
            "f1" => *nk == NamedKey::F1,
            "f2" => *nk == NamedKey::F2,
            "f3" => *nk == NamedKey::F3,
            "f4" => *nk == NamedKey::F4,
            "f5" => *nk == NamedKey::F5,
            "f6" => *nk == NamedKey::F6,
            "f7" => *nk == NamedKey::F7,
            "f8" => *nk == NamedKey::F8,
            "f9" => *nk == NamedKey::F9,
            "f10" => *nk == NamedKey::F10,
            "f11" => *nk == NamedKey::F11,
            "f12" => *nk == NamedKey::F12,
            "f13" => *nk == NamedKey::F13,
            "f14" => *nk == NamedKey::F14,
            "f15" => *nk == NamedKey::F15,
            "f16" => *nk == NamedKey::F16,
            "f17" => *nk == NamedKey::F17,
            "f18" => *nk == NamedKey::F18,
            "f19" => *nk == NamedKey::F19,
            "f20" => *nk == NamedKey::F20,
            "f21" => *nk == NamedKey::F21,
            "f22" => *nk == NamedKey::F22,
            "f23" => *nk == NamedKey::F23,
            "f24" => *nk == NamedKey::F24,
            "escape" | "esc" => *nk == NamedKey::Escape,
            "tab" => *nk == NamedKey::Tab,
            "space" => *nk == NamedKey::Space,
            "enter" | "return" => *nk == NamedKey::Enter,
            "backspace" => *nk == NamedKey::Backspace,
            "delete" | "del" => *nk == NamedKey::Delete,
            "insert" | "ins" => *nk == NamedKey::Insert,
            "home" => *nk == NamedKey::Home,
            "end" => *nk == NamedKey::End,
            "pageup" | "pgup" => *nk == NamedKey::PageUp,
            "pagedown" | "pgdn" => *nk == NamedKey::PageDown,
            "up" | "arrowup" => *nk == NamedKey::ArrowUp,
            "down" | "arrowdown" => *nk == NamedKey::ArrowDown,
            "left" | "arrowleft" => *nk == NamedKey::ArrowLeft,
            "right" | "arrowright" => *nk == NamedKey::ArrowRight,
            "scrolllock" | "scroll_lock" => *nk == NamedKey::ScrollLock,
            "pause" | "break" => *nk == NamedKey::Pause,
            _ => false,
        };
    }

    if let Key::Character(c) = key {
        if lower.chars().count() == 1 {
            let want = lower.chars().next().unwrap();
            return c
                .chars()
                .next()
                .is_some_and(|ch| ch.to_ascii_lowercase() == want);
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f3_matches() {
        assert!(key_matches(&Key::Named(NamedKey::F3), "F3"));
        assert!(key_matches(&Key::Named(NamedKey::F3), "f3"));
    }
}
