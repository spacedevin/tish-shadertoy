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

## Tish API (`tish:shadertoy`)

- **`run(code)`** — opens the native viewer (blocks until the window closes).

The CLI script uses **`readFile`** from **`tish:fs`** to load the path you pass on the command line.

## MVP limits

- **Shadertoy preprocessor:** the viewer defines **`HW_PERFORMANCE`** to **`1`** when your shader does not (the website sets this; without it, `#if HW_PERFORMANCE==0` can fail to compile on desktop GL).
- **Single full-screen fragment pass** — no buffer/sound multipass pipeline.
- **iChannel0–3** are bound to **1×1 black** textures so simple shaders compile; real texture URLs / cubemaps are not loaded yet.
- Uniforms include `iResolution`, `iTime`, `iTimeDelta`, `iFrame`, `iMouse`, `iDate`, `iSampleRate`, `iChannelResolution`, and `iGlobalTime` (alias of `iTime`). See the [VS Code Shader Toy extension](https://github.com/stevensona/shader-toy) for `mainImage` conventions.

## License

MIT
