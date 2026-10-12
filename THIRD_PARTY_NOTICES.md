# Third-party notices

## PlayCanvas engine (MIT)

Ported to Rust and WGSL in `crates/df-render/src/battle/splat.rs`, `crates/df-render/src/battle/lod.rs`,
`crates/df-render/src/battle/gpu/splat_shaders.rs`, `splat_engine.rs` and `splat_stream.rs` (the Gaussian
splat pipeline), reading engine **v2.23.2** (the latest release when the work was done; the owner's
"PlayCanvas 3" is not a published release yet):

- SOG v2 decode (`src/scene/shader-lib/wgsl/chunks/gsplat/vert/formats/sog.js`, with
  `src/framework/parsers/sog.js`): positions as two 8-bit planes joined into 16 bits over a min/max
  range in log space, "smallest three" quaternions with the largest-component index in alpha,
  scale and base colour as 256-entry codebooks (version 2) or min/max ranges (version 1), colour as
  `0.5 + value * 0.28209479`, opacity from the alpha plane. Ours: `SogLoader` in `splat.rs` (CPU
  reference, tests) and the `decode` entry point in `splat_shaders.rs` (GPU, no read-back).
- Projection of a 3D Gaussian to a screen ellipse (`.../gsplat/compute-gsplat-common.js`
  `computeSplatCov`, `.../vert/gsplatCorner.js` `initCornerCov`, `.../vert/gsplatCenter.js`,
  `.../compute-gsplat-projector.js`): covariance rotated into view space, perspective Jacobian,
  the 0.3 pixel low-pass term, the anti-aliasing opacity compensation `sqrt(detOrig/detBlur)`,
  eigen decomposition of the 2x2 result, the minimum-size and off-screen culls, the radius cap with
  covariance rescale for oversized splats, the per-workgroup atomic compaction, and the
  camera-relative sort-key precision idea (their bin weighting, our square-root depth mapping).
  Ours: the `project` entry point in `splat_shaders.rs`. Their units are doubled (their focal
  length is twice the pixel focal length), so their thresholds are rescaled there.
- Fragment falloff and blending (`.../frag/gsplat.js`): `normExp` falloff that reaches zero at the quad
  edge and premultiplied "over" blending; depth accumulated through the same blend
  (`writeSceneTextureDepth`) and resolved into the depth buffer in a separate pass.
- Level-of-detail octree scenes (`src/scene/gsplat-unified/gsplat-octree.js`, `gsplat-octree-instance.js`,
  `src/framework/parsers/gsplat-octree.js`): the `lod-meta.json` index and tree and the idea of choosing
  detail by distance under a splat budget. Ours: `lod.rs` (pure selection) and `splat_stream.rs`.

Not adopted from their WebGPU path: their radix sort (we use a 16-bit counting sort over
camera-relative depth bins, which needs only atomics and one workgroup scan), their work-buffer
and interval-compaction machinery, spherical harmonics (base colour only here), stochastic/dithered
transparency and fisheye/XR variants.

Source: PlayCanvas engine v2.23.2, <https://github.com/playcanvas/engine>.

```
MIT License

Copyright (c) 2011-2026 PlayCanvas Ltd.

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and
associated documentation files (the "Software"), to deal in the Software without restriction,
including without limitation the rights to use, copy, modify, merge, publish, distribute,
sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or
substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT
NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES
OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```

## antimatter15/splat (MIT)

The 32-byte-per-splat `.splat` file layout read by `SplatFileLoader`, and the per-splat quad
technique with Gaussian falloff, follow <https://github.com/antimatter15/splat>.

```
MIT License

Copyright (c) 2023 Kevin Kwok

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and
associated documentation files (the "Software"), to deal in the Software without restriction,
including without limitation the rights to use, copy, modify, merge, publish, distribute,
sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or
substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT
NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES
OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```

## DungeonFlux earlier build (owner's own code)

Ideas and constants ported from `monstercameron/DungeonFlux` (Go + PlayCanvas, the owner's earlier
hackathon build): per-theme colour grades (`theme_grades.mjs`), camera pan and shake bounds
(`camera_motion.mjs`), the eight-way grid path and cell/world maths (`grid_math.go`), the stage
protocol shape (`protocol.go`), the tilt-shift band blur parameters (`tilt_shift.mjs`) and the
warm grid-glow styling (`grid_glow.mjs`). No JavaScript was copied.

## Gaussian splat scan "Osada Slavotovo. Poland 2025" (CC BY 4.0)

By Andrii Shramko, <https://superspl.at/scene/1994c65c>, licensed under the Creative Commons
Attribution 4.0 International licence (<https://creativecommons.org/licenses/by/4.0/>). Used as the
battle scene `scan-village` (`content/battle-scenes/scan-village.json`); the scan data is not part of
this repository and is served from the local scan directory (`DF_GAMEPLAY_SPLAT_DIR`). Changes: the
level-of-detail chunks are decoded, rotated upright, scaled, cut above a height near the fight and
colour-graded at draw time.

The other three scenes (`scan-riverbed`, `scan-valley-bridges`, `scan-summit`) are scans by
Shachar Weis and Zhi Wang that have **no licence** and downloads disabled by their authors. They are
used for private local testing only: no scan data or derived image is committed, and they are
excluded from the default scene rotation.
