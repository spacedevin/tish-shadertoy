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
