# tish-shadertoy

Minimal **Shadertoy-style** [Tish](https://tishlang.com/) **viewer**: read local GLSL (fragment with `mainImage`) and run it in a native **OpenGL 3.3** window (`winit` + `glutin` + `glow`). No editor.

Example shaders live under [`shaders/`](shaders/).

## Requirements

- Rust toolchain (`cargo`)
- GPU with OpenGL 3.3 (or compatible GLES fallback)

## Build

```bash
cd tish-shadertoy
cargo build --release --bin tish-shadertoy-run
```

Paths assume this repo sits next to the main [`tish`](https://github.com/tishlang/tish) tree (`../tish/crates/...`).

## Run

```bash
cargo run --bin tish-shadertoy-run -- src/main.tish shaders/XstXR2
```

`argv[2]` is any path to a UTF-8 file containing the shader body (Shadertoy image-pass style).

Close the window or press **Escape** to quit.

## Performance

The window uses a **fixed physical pixel size** (default **1280×720**), not a logical size, so HiDPI displays do not silently render at 2× or 3× resolution (which can make heavy raymarchers feel stuck under ~20 FPS).

- **`TISH_SHADER_WIDTH`** / **`TISH_SHADER_HEIGHT`** — override backing resolution (physical pixels).
- **`TISH_SHADER_RENDER_SCALE`** — internal render scale in **(0, 1]** (default **1**). The heavy `mainImage` pass runs at `floor(window × scale)` pixels, then the result is **linearly upscaled** to the window. Example: `0.5` → about **4× fewer** fragment invocations; `iResolution` and `fragCoord` still match the full window so most Shadertoy math stays consistent. Clamped to **0.05..1.0**.
- **`TISH_SHADER_VSYNC=0`** — disable vsync (`DontWait`); useful to see uncapped FPS while tuning (may tear).

The GL config picker prefers **no multisampling** (MSAA). The old behavior preferred the *highest* sample count, which could multiply fragment cost for little gain on a single full-screen triangle.

Some shaders (e.g. IQ’s distance-function demos with **`AA 2`**) intentionally shade **2×2 supersampled** pixels — that is ~4× more work inside `mainImage`; lower `AA` in the file if you need speed.

## Shadertoy compatibility and inputs

The [Shadertoy site](https://www.shadertoy.com/) runs **WebGL** (GLSL ES). Shadertoy’s docs tell authors to [initialize variables](https://github.com/NatronGitHub/openfx-misc/blob/master/Shadertoy/Shadertoy.md) — the runtime does **not** inject zero for you. Many community shaders still rely on **undefined reads** behaving like zero on common browsers; **desktop OpenGL 3.3** often does not, which shows up as black or garbage.

- **`TISH_SHADER_COMPAT_ZEROINIT`** — default **on**. A line-wise preprocessor adds `= 0` / `= vec4(0.0)` for common patterns (`vec4 lights;`, `float i,s,d;`, etc.). Set to **`0`** to disable if it breaks a shader.

**Uniforms** (besides those listed under MVP): **`iChannelTime[4]`** ( **`0`** for slots backed by an image file, **`iTime`** for the default 1×1 slot — video/audio sync is not modeled yet), **`iFrameRate`** (smoothed from frame delta), plus the existing **`iChannelResolution`** from real texture sizes.

**`iMouse` (Shadertoy rules)** — **`xy`** are the cursor in pixels **only while the left button is held**; when it is **up**, **`xy` are negative** (we use **`−1`**). **`zw`** carry the click position while dragging. Feeding live **`xy` all the time** breaks shaders that rotate with **`iMouse.x / iResolution.x`** (e.g. Alien Core / `4tcXRr`).

**Texture inputs (`sampler2D` iChannel0–3)** — not automatic from Shadertoy JSON here; you wire files or built-ins:

- **`TISH_SHADER_CHANNEL_DIR`** — base directory for relative paths (default **`.`**).
- **`TISH_SHADER_CHANNEL0`** … **`TISH_SHADER_CHANNEL3`** — filesystem paths to PNG/JPEG/GIF (override comments for that slot).
- Or a line near the top of the shader: **`// iChannel0: relative/noise.png`** (same for 1–3). Env wins if both are set.
- **`// iChannel0: builtin:noise`** — procedural tiling grayscale noise (**256²**), similar to using a noise slot on the site. If **iChannel0** is unset and the source **calls** `texture(iChannel0,…)` / `textureLod` / `texelFetch`, we **auto-bind** this unless **`TISH_SHADER_AUTO_CHANNEL0_NOISE=0`**.

**Not implemented** (would need parsing a full Shadertoy export or similar): buffer passes, cubemaps, keyboard/sound/mic inputs, remote texture URLs.

## Tish API (`tish:shadertoy`)

- **`run(code)`** — opens the native viewer (blocks until the window closes).

The CLI script uses **`readFile`** from **`tish:fs`** to load the path you pass on the command line.

## MVP limits

- **Shadertoy preprocessor:** the viewer defines **`HW_PERFORMANCE`** to **`1`** when your shader does not (the website sets this; without it, `#if HW_PERFORMANCE==0` can fail to compile on desktop GL).
- **Single full-screen fragment pass** — no buffer/sound multipass pipeline.
- **iChannel0–3** default to **1×1 black**; optional **image files** via env or `// iChannelN:` comments (see above). Cubemaps / buffer inputs / URLs are not supported.
- Uniforms include `iResolution`, `iTime`, `iTimeDelta`, `iFrame`, `iMouse`, `iDate`, `iSampleRate`, `iChannelResolution`, `iChannelTime`, `iFrameRate`, and `iGlobalTime` (alias of `iTime`). See the [VS Code Shader Toy extension](https://github.com/stevensona/shader-toy) for `mainImage` conventions.

## License

MIT
