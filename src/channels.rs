//! Optional **iChannel0–3** image inputs (Shadertoy-style texture slots).

use std::path::{Path, PathBuf};

/// `TISH_SHADER_CHANNEL0` … `TISH_SHADER_CHANNEL3` (paths override comment hints for that slot).
pub fn paths_from_env() -> [Option<PathBuf>; 4] {
    std::array::from_fn(|i| {
        std::env::var(format!("TISH_SHADER_CHANNEL{i}"))
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
    })
}

/// First non-empty token after `// iChannelN:` on a line (scans first 160 lines).
pub fn paths_from_source_comments(source: &str) -> [Option<PathBuf>; 4] {
    let mut out: [Option<PathBuf>; 4] = std::array::from_fn(|_| None);
    for line in source.lines().take(160) {
        let s = line.trim_start();
        let Some(rest) = s.strip_prefix("//") else {
            continue;
        };
        let rest = rest.trim_start();
        for idx in 0..4 {
            let prefix = format!("iChannel{idx}:");
            if let Some(p) = rest.strip_prefix(prefix.as_str()) {
                let p = p.split_whitespace().next().unwrap_or("").trim();
                if !p.is_empty() {
                    out[idx] = Some(PathBuf::from(p));
                }
            }
        }
    }
    out
}

pub fn merged_paths(source: &str) -> [Option<PathBuf>; 4] {
    let env = paths_from_env();
    let com = paths_from_source_comments(source);
    std::array::from_fn(|i| env[i].clone().or_else(|| com[i].clone()))
}

pub fn channel_base_dir() -> PathBuf {
    std::env::var("TISH_SHADER_CHANNEL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

pub fn load_rgba8(path: &Path) -> Result<(Vec<u8>, u32, u32), String> {
    let img = image::open(path).map_err(|e| format!("image {}: {e}", path.display()))?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Ok((rgba.into_raw(), w, h))
}
