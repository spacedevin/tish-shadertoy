//! Approximate common **WebGL / GLES** behavior for shaders copied from Shadertoy.
//!
//! Shadertoy's own docs say to initialize variables and not rely on zero
//! ([Natron Shadertoy notes](https://github.com/NatronGitHub/openfx-misc/blob/master/Shadertoy/Shadertoy.md)).
//! Many published shaders still leave globals or `float a,b,c;` locals uninitialized; desktop **GLSL 330**
//! treats those reads as **undefined**, while several WebGL stacks effectively zero them — leading to
//! black or garbage only on native OpenGL.

use std::collections::HashSet;

/// Host-only directives on their own line; stripped before lexer/preprocess (see [`extract_host_pragmas`]).
#[derive(Clone, Copy, Default, Debug, Eq, PartialEq)]
pub struct HostPragmas {
    /// After `mainImage`, inject `clamp(_tish_acc.rgb * 0.5 + 0.5, 0, 1)` so wide linear color maps to
    /// display like many WebGL stacks. Use when the shader ends with **`o = o / (k * d)`** (or equivalent)
    /// instead of **`tanh(o / k / d)`** — the latter cannot be recovered in the wrapper.
    pub post_main_rgb_linear_clamp: bool,
}

/// Remove `// tish:…` pragma lines and return flags. Call **before** [`strip_float_f_suffix`] / other rewrites.
pub fn extract_host_pragmas(source: &str) -> (String, HostPragmas) {
    let mut flags = HostPragmas::default();
    let mut out = Vec::new();
    for line in source.lines() {
        let t = line.trim();
        if t == "// tish:post-main-rgb-linear-clamp" {
            flags.post_main_rgb_linear_clamp = true;
            continue;
        }
        out.push(line);
    }
    (out.join("\n"), flags)
}

fn is_ident_continue_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Remove C/HLSL-style `f` suffix from decimal float literals (invalid in desktop GLSL; common in copied shaders).
pub fn strip_float_f_suffix(source: &str) -> String {
    let b = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    while i < b.len() {
        let starts_float = b[i].is_ascii_digit()
            || (b[i] == b'.' && i + 1 < b.len() && b[i + 1].is_ascii_digit());
        if starts_float {
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            if i < b.len() && b[i] == b'.' {
                i += 1;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
            }
            if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
                i += 1;
                if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
                    i += 1;
                }
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
            }
            if i < b.len() && b[i] == b'f' {
                let boundary_ok = i + 1 >= b.len() || !is_ident_continue_byte(b[i + 1]);
                if boundary_ok {
                    out.push_str(&source[start..i]);
                    i += 1;
                    continue;
                }
            }
            out.push_str(&source[start..i]);
            continue;
        }
        out.push(char::from(b[i]));
        i += 1;
    }
    out
}

/// `TISH_SHADER_COMPAT_ZEROINIT=0` disables the rewriter.
pub fn compat_zeroinit_enabled() -> bool {
    !matches!(
        std::env::var("TISH_SHADER_COMPAT_ZEROINIT").as_deref(),
        Ok("0" | "false" | "off")
    )
}

fn indent_of(line: &str) -> &str {
    let n = line
        .chars()
        .take_while(|c| c.is_whitespace())
        .map(|c| c.len_utf8())
        .sum();
    line.get(..n).unwrap_or("")
}

/// Strip end-of-line `//` comment for matching (crude; sufficient for typical Shadertoy sources).
fn code_before_line_comment(line: &str) -> &str {
    line.split("//").next().map(str::trim_end).unwrap_or(line)
}

fn is_glsl_ident(s: &str) -> bool {
    let mut it = s.chars();
    match it.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    it.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `vec4 id;` / `vec3 id;` / `vec2 id;` / `float id;` / `int id;` with no `=` on the line.
fn try_single_decl(line: &str, ty: &str, zero: &str) -> Option<String> {
    let code = code_before_line_comment(line).trim();
    let prefix = format!("{ty} ");
    let rest = code.strip_prefix(prefix.as_str())?;
    let rest = rest.strip_suffix(';')?.trim();
    if rest.contains('=') || rest.contains(',') || rest.contains(' ') || !is_glsl_ident(rest) {
        return None;
    }
    let ind = indent_of(line);
    Some(format!("{ind}{ty} {rest} = {zero};"))
}

/// `float a, b, c;` with no `=` — initialize all to 0.0.
fn try_multi_float_decl(line: &str) -> Option<String> {
    let code = code_before_line_comment(line).trim();
    let rest = code.strip_prefix("float ")?.strip_suffix(';')?.trim();
    if rest.contains('=') {
        return None;
    }
    let parts: Vec<&str> = rest.split(',').map(str::trim).collect();
    if parts.len() < 2 {
        return None;
    }
    if !parts.iter().copied().all(is_glsl_ident) {
        return None;
    }
    let ind = indent_of(line);
    let inits: Vec<String> = parts
        .iter()
        .map(|p| format!("{p} = 0.0"))
        .collect();
    Some(format!("{ind}float {};", inits.join(", ")))
}

fn transform_line(line: &str) -> String {
    let t = line.trim_start();
    if t.starts_with('#') {
        return line.to_string();
    }
    if let Some(s) = try_single_decl(line, "vec4", "vec4(0.0)") {
        return s;
    }
    if let Some(s) = try_single_decl(line, "vec3", "vec3(0.0)") {
        return s;
    }
    if let Some(s) = try_single_decl(line, "vec2", "vec2(0.0)") {
        return s;
    }
    if let Some(s) = try_single_decl(line, "float", "0.0") {
        return s;
    }
    if let Some(s) = try_single_decl(line, "int", "0") {
        return s;
    }
    if let Some(s) = try_multi_float_decl(line) {
        return s;
    }
    line.to_string()
}

pub fn preprocess_webgl_compat(source: &str) -> String {
    if !compat_zeroinit_enabled() {
        return source.to_string();
    }
    source.lines().map(transform_line).collect::<Vec<_>>().join("\n")
}

fn compact_for_define_match(line: &str) -> String {
    line.chars().filter(|c| !c.is_whitespace()).collect()
}

/// `mat2(cos(vec4))` / `mat2(cos(expr))` is **not** reliable on Apple GL 4.1 (Metal); it can yield
/// garbage / NaN → black. Expand to **explicit** `mat2(cs.x, cs.y, cs.z, cs.w)` via [`tish_mat2_from_vec4`]
/// injected in `lib.rs` when this rewrite fires.
///
/// Opt out: `TISH_SHADER_COMPAT_MAT2_COS=0`.
pub fn mat2_cos_define_rewrite_enabled() -> bool {
    !matches!(
        std::env::var("TISH_SHADER_COMPAT_MAT2_COS").as_deref(),
        Ok("0" | "false" | "off")
    )
}

/// `7cfGzn`-style tunnel rotation: `#define R(a) mat2(cos(a+vec4(0, 33, 11, 0)))` (spacing may vary).
fn is_r_tunnel_mat2_cos_vec4(compact_define: &str) -> bool {
    if !compact_define.starts_with("#defineR(") {
        return false;
    }
    if !compact_define.contains("mat2(cos(") || !compact_define.contains("vec4(") {
        return false;
    }
    // Magic offsets 33 / 11 (degrees-as-radians hack in the original macro).
    (compact_define.contains(",33,") || compact_define.contains(",33."))
        && (compact_define.contains(",11,") || compact_define.contains(",11."))
}

fn rewrite_mat2_cos_define_line(line: &str) -> String {
    if !mat2_cos_define_rewrite_enabled() {
        return line.to_string();
    }
    let t = line.trim_start();
    if !t.starts_with("#define") {
        return line.to_string();
    }
    let c = compact_for_define_match(t);
    if is_r_tunnel_mat2_cos_vec4(&c) {
        let ind = indent_of(line);
        return format!(
            "{ind}#define R(a) tish_mat2_from_vec4(cos((a)+vec4(0.0, 33.0, 11.0, 0.0)))"
        );
    }
    line.to_string()
}

pub fn rewrite_mat2_cos_define_lines(source: &str) -> String {
    if !mat2_cos_define_rewrite_enabled() {
        return source.to_string();
    }
    source
        .lines()
        .map(rewrite_mat2_cos_define_line)
        .collect::<Vec<_>>()
        .join("\n")
}

/// `TISH_SHADER_COMPAT_SMOOTHSTEP=0` skips injecting a safe `smoothstep` (see `lib.rs` preamble).
pub fn smoothstep_desktop_shim_enabled() -> bool {
    !matches!(
        std::env::var("TISH_SHADER_COMPAT_SMOOTHSTEP").as_deref(),
        Ok("0" | "false" | "off")
    )
}

/// Rational `tanh` replacement for **extreme** magnitudes where some drivers return NaN (see `lib.rs`).
///
/// **Default off:** native GLSL `tanh` matches Shadertoy/WebGL for typical tone-mapping (e.g. `7cfGzn`);
/// the shim can skew or zero results on Apple/Metal for `vec4` finals.
///
/// Enable with **`TISH_SHADER_COMPAT_TANH=1`** when you need the workaround (e.g. huge intermediate
/// values in shaders like `7cjGRR` / `scS3Wm`).
pub fn tanh_desktop_shim_enabled() -> bool {
    matches!(
        std::env::var("TISH_SHADER_COMPAT_TANH").as_deref(),
        Ok("1" | "true" | "on")
    )
}

/// `TISH_SHADER_COMPAT_FLOAT_INC=0` disables rewriting `floatVar++` / `++floatVar` → `+= 1.0`.
pub fn float_postincrement_enabled() -> bool {
    !matches!(
        std::env::var("TISH_SHADER_COMPAT_FLOAT_INC").as_deref(),
        Ok("0" | "false" | "off")
    )
}

/// `TISH_SHADER_COMPAT_FOR_FRAGMUL=0` skips rewriting `for (fragColor *= v;` into separate init lines + `for (; …`.
pub fn for_loop_fragcolor_mul_init_enabled() -> bool {
    !matches!(
        std::env::var("TISH_SHADER_COMPAT_FOR_FRAGMUL").as_deref(),
        Ok("0" | "false" | "off")
    )
}

/// Desktop GLSL **`out`** parameters **do not** receive the argument value from the caller — they are
/// uninitialized on entry. Shaders that **`mainImage(out vec4 fragColor, …)`** then read `fragColor`
/// (e.g. `fragColor *= i` before any assignment) invoke **undefined behavior**; WebGL stacks often
/// still “see” a zeroed buffer, while **Apple/Metal** may propagate **NaN** → all-black.
///
/// Rewrite the first parameter to **`inout vec4`** so `mainImage(_tish_acc, …)` passes the wrapper’s
/// seeded `vec4(0,0,0,1)` into the function. Opt out with **`TISH_SHADER_COMPAT_MAINIMAGE_INOUT=0`**.
pub fn main_image_inout_enabled() -> bool {
    !matches!(
        std::env::var("TISH_SHADER_COMPAT_MAINIMAGE_INOUT").as_deref(),
        Ok("0" | "false" | "off")
    )
}

/// `void mainImage( out vec4 foo,` → `void mainImage( inout vec4 foo,` (first match only).
pub fn rewrite_main_image_out_vec4_to_inout(source: &str) -> String {
    if !main_image_inout_enabled() {
        return source.to_string();
    }
    let bytes = source.as_bytes();
    let needle = b"mainImage";
    let mut i = 0usize;
    while i + needle.len() <= bytes.len() {
        let hit = bytes[i..i + needle.len()] == *needle;
        let before_ok = i == 0 || !is_ident_continue_byte(bytes[i - 1]);
        let after_ok = i + needle.len() >= bytes.len()
            || !is_ident_continue_byte(bytes[i + needle.len()]);
        if !hit || !before_ok || !after_ok {
            i += 1;
            continue;
        }
        let mut j = i + needle.len();
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= bytes.len() || bytes[j] != b'(' {
            i += 1;
            continue;
        }
        j += 1;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j + 3 > bytes.len() || &bytes[j..j + 3] != b"out" {
            i += 1;
            continue;
        }
        let out_end = j + 3;
        if out_end < bytes.len() && is_ident_continue_byte(bytes[out_end]) {
            i += 1;
            continue;
        }
        let mut k = out_end;
        while k < bytes.len() && bytes[k].is_ascii_whitespace() {
            k += 1;
        }
        const VEC4: &[u8] = b"vec4";
        if k + VEC4.len() > bytes.len() || bytes[k..k + VEC4.len()] != *VEC4 {
            i += 1;
            continue;
        }
        let mut out = String::with_capacity(source.len() + 3);
        out.push_str(&source[..j]);
        out.push_str("inout");
        out.push_str(&source[j + 3..]);
        return out;
    }
    source.to_string()
}

fn rewrite_for_loop_fragcolor_mul_init_line(line: &str) -> String {
    let t = line.trim_start();
    if t.starts_with('#') {
        return line.to_string();
    }
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i + 3 <= bytes.len() {
        let is_for = bytes[i..].starts_with(b"for")
            && (i == 0 || !is_ident_continue_byte(bytes[i - 1]))
            && (i + 3 >= bytes.len() || !is_ident_continue_byte(bytes[i + 3]));
        if !is_for {
            i += 1;
            continue;
        }
        let mut j = i + 3;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= bytes.len() || bytes[j] != b'(' {
            i += 1;
            continue;
        }
        j += 1;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        const FRAG: &[u8] = b"fragColor";
        if j + FRAG.len() > bytes.len() || bytes[j..j + FRAG.len()] != *FRAG {
            i += 1;
            continue;
        }
        if j + FRAG.len() < bytes.len() && is_ident_continue_byte(bytes[j + FRAG.len()]) {
            i += 1;
            continue;
        }
        j += FRAG.len();
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j + 2 > bytes.len() || &bytes[j..j + 2] != b"*=" {
            i += 1;
            continue;
        }
        j += 2;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        let id_start = j;
        while j < bytes.len() && is_ident_continue_byte(bytes[j]) {
            j += 1;
        }
        if j == id_start {
            i += 1;
            continue;
        }
        let ident = &line[id_start..j];
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= bytes.len() || bytes[j] != b';' {
            i += 1;
            continue;
        }
        let semi = j;
        let ind = indent_of(line);
        let rest = &line[semi + 1..];
        return format!("{ind}{ident} = 0.0;\n{ind}fragColor *= {ident};\n{ind}for (;{rest}");
    }
    line.to_string()
}

/// Some Shadertoy shaders use `for (fragColor *= i; …)` where `i` is read before assignment in the
/// init clause; desktop GLSL does not zero-init locals. Split into `ident = 0.0`, `fragColor *= ident`,
/// then `for (; …)` (comma-in-for is avoided for picky drivers).
pub fn rewrite_for_loop_fragcolor_mul_init(source: &str) -> String {
    if !for_loop_fragcolor_mul_init_enabled() {
        return source.to_string();
    }
    source
        .lines()
        .map(rewrite_for_loop_fragcolor_mul_init_line)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Optional **`fragColor = tanh(…)`** display hack: **`rgb * 0.5 + 0.5`**. Turns **near‑zero** `tanh` into
/// **flat ~50% gray** everywhere (misleading). **Off by default.** Enable only for experiments:
/// **`TISH_SHADER_AUTO_TANH_MAP=1`**.
pub fn auto_tanh_fragcolor_display_map_enabled() -> bool {
    matches!(
        std::env::var("TISH_SHADER_AUTO_TANH_MAP").as_deref(),
        Ok("1" | "true" | "on")
    )
}

fn first_ident_in_decl_segment(seg: &str) -> Option<&str> {
    let seg = seg.trim();
    if seg.is_empty() {
        return None;
    }
    let name = seg
        .split(|c: char| c == '=' || c.is_whitespace())
        .next()?
        .trim();
    if is_glsl_ident(name) {
        Some(name)
    } else {
        None
    }
}

/// First `for (...)` init clause on a line (between `(` after `for` and the first `;` at paren depth 1).
fn first_for_loop_init_clause(line: &str) -> Option<&str> {
    let idx = line.find("for")?;
    let bytes = line.as_bytes();
    let mut i = idx + 3;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= bytes.len() || bytes[i] != b'(' {
        return None;
    }
    i += 1;
    let start = i;
    let mut depth = 1i32;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b';' if depth == 1 => {
                return Some(line.get(start..i)?);
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Identifiers introduced in a `for` init clause (`int i=0`, `int i=0, j=0`, …).
/// Skips compound assignments (`fragColor *= i`) so we do not shadow [`rewrite_float_post_increment`].
fn int_idents_from_for_init_clause(init: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    for part in init.split(',') {
        let p = part.trim_start();
        if p.is_empty() {
            continue;
        }
        let b = p.as_bytes();
        if b.len() >= 3
            && &b[0..3] == b"int"
            && (b.len() == 3 || !is_ident_continue_byte(b[3]))
        {
            let rest = p.get(3..).unwrap_or("").trim_start();
            let name = rest
                .split(|c: char| c == '=' || c.is_whitespace())
                .next()
                .filter(|s| is_glsl_ident(s));
            if let Some(n) = name {
                out.insert(n.to_string());
            }
        } else if !p.contains("*=")
            && !p.contains("+=")
            && !p.contains("-=")
            && !p.contains("/=")
        {
            // `int i=0, j=0` → second segment is ` j=0` (no `int` keyword).
            let name = p.split('=').next().map(str::trim).filter(|s| is_glsl_ident(s));
            if let Some(n) = name {
                out.insert(n.to_string());
            }
        }
    }
    out
}

/// Names declared as `float` / `const float` on a line (skips function return types like `float foo(`).
pub fn collect_float_declared_names(source: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    for line in source.lines() {
        let t = line.trim_start();
        if t.starts_with('#') {
            continue;
        }
        let code = code_before_line_comment(line).trim();
        let rest = if let Some(r) = code.strip_prefix("const float ") {
            r
        } else if let Some(r) = code.strip_prefix("float ") {
            r
        } else {
            continue;
        };
        let rest = rest.split(';').next().unwrap_or(rest).trim();
        if rest.is_empty() {
            continue;
        }
        let first_tok = rest.split_whitespace().next().unwrap_or("");
        let after_first = rest[first_tok.len().min(rest.len())..].trim_start();
        if after_first.starts_with('(') {
            continue;
        }
        for part in rest.split(',') {
            if let Some(name) = first_ident_in_decl_segment(part) {
                out.insert(name.to_string());
            }
        }
    }
    out
}

/// After `++` (postfix) or after `++ident` (prefix), true if a comparison follows (`<`, `<=`, `!=`, …).
/// Excludes shift `<<` / `>>` so `i++ << 2` is not treated as comparison.
fn comparison_follows(bytes: &[u8], mut pos: usize) -> bool {
    while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }
    if pos >= bytes.len() {
        return false;
    }
    match bytes[pos] {
        b'<' => !(pos + 1 < bytes.len() && bytes[pos + 1] == b'<'),
        b'>' => !(pos + 1 < bytes.len() && bytes[pos + 1] == b'>'),
        b'=' => pos + 1 < bytes.len() && bytes[pos + 1] == b'=',
        b'!' => pos + 1 < bytes.len() && bytes[pos + 1] == b'=',
        _ => false,
    }
}

fn rewrite_postfix_float_pp(line: &str, float_names: &HashSet<String>) -> String {
    let bytes = line.as_bytes();
    let mut out = String::with_capacity(line.len() + 24);
    let mut start = 0usize;
    let mut pos = 0usize;
    while pos < bytes.len() {
        if pos + 1 < bytes.len() && bytes[pos] == b'+' && bytes[pos + 1] == b'+' {
            let end = pos;
            if end > 0 && is_ident_continue_byte(bytes[end - 1]) {
                let mut s = end;
                while s > 0 && is_ident_continue_byte(bytes[s - 1]) {
                    s -= 1;
                }
                let is_prefix = s >= 2 && bytes[s - 2] == b'+' && bytes[s - 1] == b'+';
                let ident = &line[s..end];
                if float_names.contains(ident) && !is_prefix {
                    out.push_str(&line[start..s]);
                    let after_pp = end + 2;
                    if comparison_follows(bytes, after_pp) {
                        // `i++ < x` must not become `i += 1.0 < x` → `i += (1.0 < x)` (bool).
                        // Postfix value = old i = (i += 1.0) - 1.0 when += yields the new float.
                        out.push_str("((");
                        out.push_str(ident);
                        out.push_str(" += 1.0) - 1.0)");
                    } else {
                        out.push_str(ident);
                        out.push_str(" += 1.0");
                    }
                    pos = after_pp;
                    start = pos;
                    continue;
                }
            }
        }
        pos += 1;
    }
    out.push_str(&line[start..]);
    out
}

fn rewrite_prefix_float_pp(line: &str, float_names: &HashSet<String>) -> String {
    let bytes = line.as_bytes();
    let mut out = String::with_capacity(line.len() + 24);
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'+' && bytes[i + 1] == b'+' {
            let prev_ok = i == 0 || !is_ident_continue_byte(bytes[i - 1]);
            if prev_ok {
                let mut j = i + 2;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                let id_start = j;
                while j < bytes.len() && is_ident_continue_byte(bytes[j]) {
                    j += 1;
                }
                if j > id_start {
                    let ident = &line[id_start..j];
                    if float_names.contains(ident) {
                        out.push_str(&line[start..i]);
                        if comparison_follows(bytes, j) {
                            // `++i < x` → `(i += 1.0) < x` (prefix returns new i).
                            out.push_str("(");
                            out.push_str(ident);
                            out.push_str(" += 1.0)");
                        } else {
                            out.push_str(ident);
                            out.push_str(" += 1.0");
                        }
                        i = j;
                        start = i;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    out.push_str(&line[start..]);
    out
}

/// Desktop GLSL allows `++` only on **int** scalars; Shadertoy often uses `float i; … i++`.
///
/// `collect_float_declared_names` is file-wide; a later `for (int m=…; m++)` must not rewrite `m++`
/// when `float m` appeared earlier (e.g. IQ shaders). Drop names declared `int` in this line’s first
/// `for ( … ;` init clause from the float set before `++` → `+= 1.0`.
pub fn rewrite_float_post_increment(source: &str, float_names: &HashSet<String>) -> String {
    if !float_postincrement_enabled() {
        return source.to_string();
    }
    source
        .lines()
        .map(|line| {
            if line.trim_start().starts_with('#') {
                return line.to_string();
            }
            let int_shadow = first_for_loop_init_clause(line)
                .map(int_idents_from_for_init_clause)
                .unwrap_or_default();
            let applicable: HashSet<String> = float_names
                .iter()
                .filter(|n| !int_shadow.contains(*n))
                .cloned()
                .collect();
            let s = rewrite_postfix_float_pp(line, &applicable);
            rewrite_prefix_float_pp(&s, &applicable)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_host_pragmas_linear_clamp() {
        let (s, p) = extract_host_pragmas(
            "// tish:post-main-rgb-linear-clamp\nvoid mainImage(out vec4 c, in vec2 u) {}\n",
        );
        assert!(p.post_main_rgb_linear_clamp);
        assert!(!s.contains("tish:post-main-rgb-linear-clamp"));
        assert!(s.contains("void mainImage"));
    }

    #[test]
    fn main_image_out_vec4_becomes_inout() {
        let src = "void mainImage( out vec4 fragColor, in vec2 fragCoord )\n{\n}\n";
        let out = rewrite_main_image_out_vec4_to_inout(src);
        assert!(
            out.contains("mainImage( inout vec4 fragColor"),
            "got: {out:?}"
        );
        assert!(
            !out.contains("( out vec4 fragColor"),
            "should not keep bare out vec4 first parameter"
        );
    }

    #[test]
    fn vec4_global() {
        let s = preprocess_webgl_compat("vec4 lights;");
        assert_eq!(s, "vec4 lights = vec4(0.0);");
    }

    #[test]
    fn float_triple() {
        let s = preprocess_webgl_compat("    float i,s,d;");
        assert_eq!(s, "    float i = 0.0, s = 0.0, d = 0.0;");
    }

    #[test]
    fn skips_assigned() {
        let s = preprocess_webgl_compat("float x = 1.0;");
        assert_eq!(s, "float x = 1.0;");
    }

    #[test]
    fn mat2_cos_r_define_rewrites_for_apple_gl() {
        let src = "#define R(a) mat2(cos(a+vec4(0, 33, 11, 0)))\n";
        let out = rewrite_mat2_cos_define_lines(src);
        assert!(out.contains("tish_mat2_from_vec4"));
        assert!(!out.contains("mat2(cos(a+vec4"));
    }

    #[test]
    fn strip_f_suffix() {
        assert_eq!(
            strip_float_f_suffix("return 6.2831853f * x;"),
            "return 6.2831853 * x;"
        );
        assert_eq!(
            strip_float_f_suffix("vec3(-0.711000f, 0.275000f, -0.052000f)"),
            "vec3(-0.711000, 0.275000, -0.052000)"
        );
    }

    #[test]
    fn collect_float_names() {
        let src = "float i = 0.0, s = 0.0;\nvoid x() { int j; }\n";
        let n = collect_float_declared_names(src);
        assert!(n.contains("i"));
        assert!(n.contains("s"));
        assert!(!n.contains("j"));
    }

    #[test]
    fn rewrite_float_loop_increment() {
        let src = "float i = 0.0, s = 0.0;\n    for (a; b; i++) { }\n";
        let names = collect_float_declared_names(src);
        let out = rewrite_float_post_increment(src, &names);
        assert!(out.contains("i += 1.0"));
        assert!(!out.contains("i++"));
    }

    #[test]
    fn rewrite_postfix_does_not_duplicate_prefix() {
        let src = "float i = 0.0;\n    for (fragColor *= i; i < 20.0; i++) { }\n";
        let names = collect_float_declared_names(src);
        let out = rewrite_float_post_increment(src, &names);
        assert_eq!(out.matches("for (fragColor").count(), 1);
        assert!(
            out.contains("for (fragColor *= i; i < 20.0; i += 1.0)"),
            "got: {out:?}"
        );
    }

    #[test]
    fn float_pp_postfix_before_comparison_preserves_old_value() {
        let src = "float i = 0.0;\n    for(; i++ < 128.;) {}\n";
        let names = collect_float_declared_names(src);
        let out = rewrite_float_post_increment(src, &names);
        assert!(
            out.contains("((i += 1.0) - 1.0) < 128."),
            "i++ < x must not become i += 1.0 < x (bool): {out:?}"
        );
    }

    #[test]
    fn float_pp_prefix_before_comparison_parenthesizes() {
        let src = "float i = 0.0;\n    for(; ++i < 128.;) {}\n";
        let names = collect_float_declared_names(src);
        let out = rewrite_float_post_increment(src, &names);
        assert!(
            out.contains("(i += 1.0) < 128."),
            "++i < x must not become i += 1.0 < x: {out:?}"
        );
    }

    #[test]
    fn float_pp_postfix_shift_is_not_comparison() {
        let src = "float i = 0.0;\n    x = i++ << 2;\n";
        let names = collect_float_declared_names(src);
        let out = rewrite_float_post_increment(src, &names);
        assert!(
            out.contains("i += 1.0") && !out.contains("((i += 1.0) - 1.0)"),
            "got: {out:?}"
        );
    }

    #[test]
    fn float_pp_does_not_rewrite_int_loop_var_shadowing_float() {
        let src = r#"float m = 1.0;
    for( int m=ZERO; m<4; m++ ) { }
"#;
        let names = collect_float_declared_names(src);
        assert!(names.contains("m"));
        let out = rewrite_float_post_increment(src, &names);
        assert!(
            out.contains("for( int m=ZERO; m<4; m++ )"),
            "int m++ must stay int: {out:?}"
        );
        assert!(!out.contains("m += 1.0"));
    }

    #[test]
    fn for_loop_fragcolor_mul_init_inserts_float_zero() {
        let src = "    for (fragColor *= i; i < 20.0; i++) { }\n";
        let out = rewrite_for_loop_fragcolor_mul_init(src);
        assert_eq!(
            out,
            "    i = 0.0;\n    fragColor *= i;\n    for (; i < 20.0; i++) { }"
        );
    }

    #[test]
    fn for_loop_fragcolor_mul_then_float_increment_pipeline() {
        let src = "float i, s;\n    for (fragColor *= i; i < 20.0; i++) { }\n";
        let z = preprocess_webgl_compat(src);
        let z = rewrite_for_loop_fragcolor_mul_init(&z);
        let names = collect_float_declared_names(&z);
        let out = rewrite_float_post_increment(&z, &names);
        assert!(out.contains("i = 0.0;"));
        assert!(out.contains("fragColor *= i"));
        assert!(out.contains("for (; i < 20.0;"));
        assert!(out.contains("i += 1.0"));
        assert!(!out.contains("i++"));
    }
}
