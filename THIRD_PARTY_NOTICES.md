# Third-party notices

## PlayCanvas engine (MIT)

Ported to Rust in `crates/df-render/src/battle/splat.rs` and `crates/df-render/src/battle/gpu/shaders.rs`
(the Gaussian splat pass):

- the PlayCanvas SOG bundle reading rules: positions as two 8-bit planes joined into 16 bits with a
  min/max range and a signed log transform, "smallest three" quaternions with the largest-component
  index in alpha, scale and base colour as 256-entry codebooks (version 2) or min/max ranges
  (version 1), colour as `0.5 + value * 0.28209479`, opacity from the alpha plane;
- the screen-space ellipse construction for a projected 3D Gaussian (covariance rotated into view
  space, Jacobian of the perspective projection, eigen decomposition of the 2x2 result, a quad
  stretched along the two axes, Gaussian falloff in the fragment stage).

Source: PlayCanvas engine 2.x, <https://github.com/playcanvas/engine>.

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
