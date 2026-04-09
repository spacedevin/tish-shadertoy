//! Approximate common **WebGL / GLES** behavior for shaders copied from Shadertoy.
//!
//! Shadertoy's own docs say to initialize variables and not rely on zero
//! ([Natron Shadertoy notes](https://github.com/NatronGitHub/openfx-misc/blob/master/Shadertoy/Shadertoy.md)).
//! Many published shaders still leave globals or `float a,b,c;` locals uninitialized; desktop **GLSL 330**
//! treats those reads as **undefined**, while several WebGL stacks effectively zero them — leading to
//! black or garbage only on native OpenGL.

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
