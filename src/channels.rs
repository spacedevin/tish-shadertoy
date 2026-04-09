//! Optional **iChannel0–3** image inputs (Shadertoy-style texture slots).

use std::path::{Path, PathBuf};

/// Magic path: procedural **grayscale noise** (256², tiling), similar role to Shadertoy’s texture slots.
pub const BUILTIN_NOISE: &str = "builtin:noise";

/// When **iChannel0** has no env/comment path and the source **samples** `iChannel0`, use [`BUILTIN_NOISE`].
/// Disable with **`TISH_SHADER_AUTO_CHANNEL0_NOISE=0`**.
pub fn auto_noise_ichannel0(source: &str) -> bool {
    if matches!(
        std::env::var("TISH_SHADER_AUTO_CHANNEL0_NOISE").as_deref(),
        Ok("0" | "false" | "off")
    ) {
        return false;
    }
    let compact: String = source.chars().filter(|c| !c.is_whitespace()).collect();
    compact.contains("texture(iChannel0,")
        || compact.contains("textureLod(iChannel0,")
        || compact.contains("texelFetch(iChannel0,")
}

/// Deterministic tiling noise (RGBA, gray in RGB) for SDF / biplanar shaders that expect a real texture.
pub fn procedural_noise_rgba(width: u32, height: u32) -> (Vec<u8>, u32, u32) {
    let mut v = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let n = fbm_byte(x, y);
            v.extend_from_slice(&[n, n, n, 255]);
        }
    }
    (v, width, height)
}

fn hash01(x: u32, y: u32) -> f32 {
    let mut n = x
        .wrapping_mul(374761393)
        .wrapping_add(y.wrapping_mul(668265263));
    n ^= n >> 13;
    n = n.wrapping_mul(1274126177);
    (n ^ (n >> 16)) as f32 * (1.0 / u32::MAX as f32)
}

fn fbm_byte(x: u32, y: u32) -> u8 {
    let mut a = 0.0f32;
    let mut amp = 0.5f32;
    let mut xf = x as f32;
    let mut yf = y as f32;
    for _ in 0..5 {
        let ix = xf as u32;
        let iy = yf as u32;
        a += amp * hash01(ix, iy);
        amp *= 0.5;
        xf *= 2.0;
        yf *= 2.0;
    }
    (a.clamp(0.0, 1.0) * 255.0) as u8
}

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

/// Same as [`merged_paths`], but if **iChannel0** is unset and [`auto_noise_ichannel0`] matches, uses [`BUILTIN_NOISE`].
pub fn resolved_channel_paths(source: &str) -> [Option<PathBuf>; 4] {
    let mut p = merged_paths(source);
    if p[0].is_none() && auto_noise_ichannel0(source) {
        p[0] = Some(PathBuf::from(BUILTIN_NOISE));
    }
    p
}

pub fn is_builtin_noise_path(p: &Path) -> bool {
    p == Path::new(BUILTIN_NOISE)
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
