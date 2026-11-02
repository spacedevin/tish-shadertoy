# Shaders

Example shaders from [Shadertoy](https://www.shadertoy.com/), stored locally so the viewer runs without an
API key. Each file is named by its Shadertoy id; the original, with its author's comments and any updates,
is at `https://www.shadertoy.com/view/<id>`.

**These shaders are not covered by this repository's MIT license.** Each one belongs to its author and keeps
its author's license. Where a file states a license, that license applies. Where it does not, the shader is
under Shadertoy's default license, [CC BY-NC-SA 3.0](https://creativecommons.org/licenses/by-nc-sa/3.0/):
credit the author, non-commercial use only, share alike. Check the original page before reusing one.

| File | Shadertoy | Author / credit | License |
|---|---|---|---|
| `4tcXRr.glsl` | [4tcXRr](https://www.shadertoy.com/view/4tcXRr) | GLKITTY (2016), per the file header | Shadertoy default (CC BY-NC-SA 3.0) |
| `7cfGzn.glsl` | [7cfGzn](https://www.shadertoy.com/view/7cfGzn) | @WorkingClassHacker, based on *Abstract Shine* by @Frosbyte | CC BY-NC-SA 4.0 (stated in the file) |
| `7cfSzf.glsl` | [7cfSzf](https://www.shadertoy.com/view/7cfSzf) | *Eminance visualizer* by Orblivius | Shadertoy default (CC BY-NC-SA 3.0) |
| `7cjGRR.glsl` | [7cjGRR](https://www.shadertoy.com/view/7cjGRR) | see the Shadertoy page | Shadertoy default (CC BY-NC-SA 3.0) |
| `Dsf3WH.glsl` | [Dsf3WH](https://www.shadertoy.com/view/Dsf3WH) | see the Shadertoy page | Shadertoy default (CC BY-NC-SA 3.0) |
| `Xds3zN.glsl` | [Xds3zN](https://www.shadertoy.com/view/Xds3zN) | Inigo Quilez (2013) | MIT (stated in the file) |
| `XstXR2.glsl` | [XstXR2](https://www.shadertoy.com/view/XstXR2) | see the Shadertoy page | Shadertoy default (CC BY-NC-SA 3.0) |
| `llfXW8.glsl` | [llfXW8](https://www.shadertoy.com/view/llfXW8) | see the Shadertoy page | Shadertoy default (CC BY-NC-SA 3.0) |
| `ls3SRr.glsl` | [ls3SRr](https://www.shadertoy.com/view/ls3SRr) | Sébastien Bérubé (2014, updated 2016) | Shadertoy default (CC BY-NC-SA 3.0) |
| `mtyGWy.glsl` | [mtyGWy](https://www.shadertoy.com/view/mtyGWy) | the shader from the tutorial linked in its header ([video](https://youtu.be/f4s1h2YETNY)) | Shadertoy default (CC BY-NC-SA 3.0) |
| `scS3Wm.glsl` | [scS3Wm](https://www.shadertoy.com/view/scS3Wm) | see the Shadertoy page | Shadertoy default (CC BY-NC-SA 3.0) |
| `scXSRf.glsl` | [scXSRf](https://www.shadertoy.com/view/scXSRf) | mrange | CC0 1.0 (stated in the file) |
| `tsXBzS.glsl` | [tsXBzS](https://www.shadertoy.com/view/tsXBzS) | see the Shadertoy page | Shadertoy default (CC BY-NC-SA 3.0) |

## Textures

The two images in [`../textures/`](../textures/) are Shadertoy's built-in channel textures, named by the hash
Shadertoy serves them under. `llfXW8.glsl` binds one with a `// iChannel1:` comment. They come from
Shadertoy's media library and are included only so that shader runs as it does on the site.

## Adding a shader

Copy the **Image** tab's code into a `.glsl` file here and run it:

```bash
npm run build:pumped
target/pumped shaders/<id>.glsl
```

Only single-pass image shaders are supported (see the main README for what is and isn't implemented). Keep
the author's header comment, and add a row to the table above.
