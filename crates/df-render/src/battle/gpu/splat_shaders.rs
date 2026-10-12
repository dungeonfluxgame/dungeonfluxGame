//! WGSL for streamed Gaussian splat scans: GPU decode of SOG chunks, per-frame projection and
//! culling, a counting depth sort, the splat draw and the depth resolve.
//!
//! Ported from PlayCanvas engine v2.23.2 (MIT; see `THIRD_PARTY_NOTICES.md`):
//! - SOG v2 decode (`getCenter`, `getRotation`, `getScale`, `getColor`) follows
//!   `scene/shader-lib/wgsl/chunks/gsplat/vert/formats/sog.js`.
//! - The projection, low-pass term, anti-aliasing factor, culls and the oversized-splat
//!   covariance rescale follow `.../gsplat/compute-gsplat-common.js` (`computeSplatCov`),
//!   `.../vert/gsplatCorner.js` (`initCornerCov`) and `.../compute-gsplat-projector.js`.
//! - The `normExp` falloff and the premultiplied blend follow `.../frag/gsplat.js`.
//! - The depth accumulated through the same blend follows `writeSceneTextureDepth` in that file.
//!   Ours resolves it into the depth buffer in a separate full-screen pass.
//!
//! Our own: the record layout, the 16-bit camera-relative counting sort (PlayCanvas radix-sorts
//! 32-bit keys with its own compute sorter), the grade and light pool, and the pixel units (their
//! focal length is twice ours, which is why their thresholds are scaled here).

/// Uniforms shared by the project, draw and resolve passes.
const COMMON: &str = r#"
struct U {
    view: mat4x4<f32>,
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,       // xyz eye, w sort range metres
    viewport: vec4<f32>,  // width px, height px, focal px, near cull metres
    limits: vec4<f32>,    // min extent px, min contribution, alpha clip, splat count
    grade: vec4<f32>,     // exposure gain, saturation, ambient, pool radius metres
    tint: vec4<f32>,      // rgb tint
    pool: vec4<f32>,      // lit pool centre x, z
    depth: vec4<f32>,     // projection m10, m14, depth bias metres, resolve alpha threshold
    sky: vec4<f32>,       // sky colour behind the scan (rgb)
    clip: vec4<f32>,      // cut height, cut radius squared, centre x, centre z (radius 0: no cut)
}
"#;

const PROJECT: &str = r#"
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var<storage, read> records: array<u32>;
@group(0) @binding(2) var<storage, read_write> cache: array<u32>;
@group(0) @binding(3) var<storage, read_write> keys: array<u32>;
@group(0) @binding(4) var<storage, read_write> hist: array<atomic<u32>>;
@group(0) @binding(5) var<storage, read_write> counter: array<atomic<u32>>;
@group(0) @binding(6) var<storage, read_write> args: array<u32>;
@group(0) @binding(7) var<storage, read_write> order: array<u32>;

var<workgroup> wg_count: atomic<u32>;
var<workgroup> wg_base: u32;
var<workgroup> segment: array<u32, 256>;

fn project_splat(i: u32, out: ptr<function, array<u32, 8>>, key: ptr<function, u32>) -> bool {
    let b = i * 8u;
    let pos = vec3<f32>(bitcast<f32>(records[b]), bitcast<f32>(records[b + 1u]), bitcast<f32>(records[b + 2u]));
    let vc = (u.view * vec4<f32>(pos, 1.0)).xyz;
    if (vc.z > -u.viewport.w) {
        return false;
    }
    if (u.clip.y > 0.0 && pos.y > u.clip.x) {
        let from_centre = pos.xz - u.clip.zw;
        if (dot(from_centre, from_centre) < u.clip.y) {
            return false;
        }
    }
    let rgba = unpack4x8unorm(records[b + 3u]);
    let opacity = rgba.a;
    if (opacity < u.limits.z) {
        return false;
    }
    let clip = u.view_proj * vec4<f32>(pos, 1.0);
    let ndc = clip.xy / clip.w;

    let q = normalize(vec4<f32>(unpack2x16snorm(records[b + 4u]), unpack2x16snorm(records[b + 5u])));
    let s01 = unpack2x16float(records[b + 6u]);
    let s = vec3<f32>(s01, unpack2x16float(records[b + 7u]).x);
    let x = q.x; let y = q.y; let z = q.z; let w = q.w;
    // Columns of the rotation matrix.
    let c0 = vec3<f32>(1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + w * z), 2.0 * (x * z - w * y));
    let c1 = vec3<f32>(2.0 * (x * y - w * z), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z + w * x));
    let c2 = vec3<f32>(2.0 * (x * z + w * y), 2.0 * (y * z - w * x), 1.0 - 2.0 * (x * x + y * y));

    // Perspective Jacobian times the view rotation (computeSplatCov): tt0 and tt1 are the world
    // directions that move the projected centre one pixel along screen x and y.
    let f = u.viewport.z;
    let vz = min(vc.z, -0.001);
    let j1 = f / vz;
    let j2 = -j1 / vz * vc.xy;
    let w0 = vec3<f32>(u.view[0].x, u.view[1].x, u.view[2].x);
    let w1 = vec3<f32>(u.view[0].y, u.view[1].y, u.view[2].y);
    let w2 = vec3<f32>(u.view[0].z, u.view[1].z, u.view[2].z);
    let tt0 = j1 * w0 + j2.x * w2;
    let tt1 = j1 * w1 + j2.y * w2;
    // cov2d = (S R^T T)^T (S R^T T)
    let b0 = s * vec3<f32>(dot(c0, tt0), dot(c1, tt0), dot(c2, tt0));
    let b1 = s * vec3<f32>(dot(c0, tt1), dot(c1, tt1), dot(c2, tt1));
    let a_raw = dot(b0, b0);
    let b_raw = dot(b0, b1);
    let c_raw = dot(b1, b1);
    // Low-pass: every splat covers at least about a pixel.
    var a = a_raw + 0.3;
    var bb = b_raw;
    var c = c_raw + 0.3;
    let det = a * c - bb * bb;
    if (det <= 0.0) {
        return false;
    }
    // Anti-aliasing: the energy the low-pass added is taken back out of the opacity.
    let aa = sqrt(max((a_raw * c_raw - bb * bb) / det, 0.0));
    if (opacity * 6.283185 * sqrt(det) < u.limits.y) {
        return false;
    }
    // A splat that would exceed the radius cap gets its covariance rescaled so the Gaussian
    // reaches its cutoff at the cap instead of being clipped to a hard-edged quad.
    let vmin = min(1024.0, min(u.viewport.x, u.viewport.y));
    let uncapped = 2.0 * sqrt(2.0 * max(a, c));
    let cap = max(1.0, uncapped / vmin);
    let inv = 1.0 / (cap * cap);
    a = a * inv;
    bb = bb * inv;
    c = c * inv;
    let mid = 0.5 * (a + c);
    let radius = length(vec2<f32>(0.5 * (a - c), bb));
    let l1 = mid + radius;
    let l2 = max(mid - radius, 0.1);
    // Quad half-extent in standard deviations: where opacity times the Gaussian falls below one
    // 8-bit step (at most 2.83, PlayCanvas's fixed cutoff). Faint splats get smaller quads.
    let k = clamp(sqrt(2.0 * log(max(255.0 * opacity, 1.001))), 1.0, 2.8284271);
    let e1 = k * sqrt(l1);
    let e2 = k * sqrt(l2);
    if (max(e1, e2) < u.limits.x) {
        return false;
    }
    // Frustum cull on the quad's extent.
    let px = abs(ndc) * u.viewport.xy * 0.5;
    if (px.x - max(e1, e2) > u.viewport.x * 0.5 || px.y - max(e1, e2) > u.viewport.y * 0.5) {
        return false;
    }
    let dir = normalize(vec2<f32>(bb, l1 - a));
    let v1 = e1 * dir;
    let v2 = e2 * vec2<f32>(dir.y, -dir.x);

    // Grade: exposure, saturation, tint, then a warm pool of light around the fight.
    var col = rgba.rgb * u.grade.x;
    let luma = dot(col, vec3<f32>(0.2126, 0.7152, 0.0722));
    col = mix(vec3<f32>(luma), col, u.grade.y) * u.tint.rgb;
    var lit = vec3<f32>(1.0);
    if (u.grade.w > 0.01) {
        let d = length(pos.xz - u.pool.xy);
        let weight = 1.0 - smoothstep(0.3 * u.grade.w, u.grade.w, d);
        lit = mix(vec3<f32>(u.grade.z), vec3<f32>(1.08, 1.0, 0.86), weight);
    }
    col = max(col * lit, vec3<f32>(0.0));

    let depth = -vc.z;
    (*out)[0] = bitcast<u32>(ndc.x);
    (*out)[1] = bitcast<u32>(ndc.y);
    (*out)[2] = bitcast<u32>(depth);
    (*out)[3] = pack2x16float(v1);
    (*out)[4] = pack2x16float(v2);
    (*out)[5] = pack2x16float(col.rg);
    (*out)[6] = pack2x16float(vec2<f32>(col.b, opacity * aa));
    (*out)[7] = bitcast<u32>(0.5 * k * k);
    // Far splats get small keys, so ascending key order is back to front. The square root gives
    // the finest steps near the camera (PlayCanvas weights its sort bins the same way).
    let t = clamp(depth / u.eye.w, 0.0, 1.0);
    *key = 65535u - u32(sqrt(t) * 65535.0);
    return true;
}

@compute @workgroup_size(256)
fn project(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    if (li == 0u) {
        atomicStore(&wg_count, 0u);
    }
    workgroupBarrier();
    var data = array<u32, 8>();
    var key = 0u;
    var valid = false;
    if (gid.x < u32(u.limits.w)) {
        valid = project_splat(gid.x, &data, &key);
    }
    var local_slot = 0u;
    if (valid) {
        local_slot = atomicAdd(&wg_count, 1u);
    }
    workgroupBarrier();
    // One global atomic per workgroup reserves a contiguous range of cache slots.
    if (li == 0u) {
        wg_base = atomicAdd(&counter[0], atomicLoad(&wg_count));
    }
    workgroupBarrier();
    if (valid) {
        let slot = wg_base + local_slot;
        for (var k = 0u; k < 8u; k = k + 1u) {
            cache[slot * 8u + k] = data[k];
        }
        keys[slot] = key;
        atomicAdd(&hist[key], 1u);
    }
}

// Exclusive prefix sum over the 65536 bins by one workgroup, and the indirect draw arguments.
@compute @workgroup_size(256)
fn scan(@builtin(local_invocation_index) li: u32) {
    var sum = 0u;
    for (var k = 0u; k < 256u; k = k + 1u) {
        sum = sum + atomicLoad(&hist[li * 256u + k]);
    }
    segment[li] = sum;
    workgroupBarrier();
    if (li == 0u) {
        var running = 0u;
        for (var k = 0u; k < 256u; k = k + 1u) {
            let value = segment[k];
            segment[k] = running;
            running = running + value;
        }
        args[0] = 4u;
        args[1] = running;
        args[2] = 0u;
        args[3] = 0u;
    }
    workgroupBarrier();
    var running = segment[li];
    for (var k = 0u; k < 256u; k = k + 1u) {
        let index = li * 256u + k;
        let value = atomicLoad(&hist[index]);
        atomicStore(&hist[index], running);
        running = running + value;
    }
}

@compute @workgroup_size(256)
fn scatter(@builtin(global_invocation_id) gid: vec3<u32>) {
    let visible = atomicLoad(&counter[0]);
    if (gid.x < visible) {
        let slot = atomicAdd(&hist[keys[gid.x]], 1u);
        order[slot] = gid.x;
    }
}
"#;

const DRAW: &str = r#"
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var<storage, read> cache: array<u32>;
@group(0) @binding(2) var<storage, read> order: array<u32>;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) colour: vec4<f32>,
    @location(2) @interpolate(flat) depth: f32,
    @location(3) @interpolate(flat) falloff: f32,
}

@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VOut {
    var corners = array<vec2<f32>, 4>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, 1.0));
    let b = order[ii] * 8u;
    let ndc = vec2<f32>(bitcast<f32>(cache[b]), bitcast<f32>(cache[b + 1u]));
    let e1 = unpack2x16float(cache[b + 3u]);
    let e2 = unpack2x16float(cache[b + 4u]);
    let rg = unpack2x16float(cache[b + 5u]);
    let ba = unpack2x16float(cache[b + 6u]);
    let corner = corners[vi];
    let offset_px = corner.x * e1 + corner.y * e2;
    var out: VOut;
    out.pos = vec4<f32>(ndc + offset_px * 2.0 / u.viewport.xy, 0.5, 1.0);
    out.uv = corner;
    out.colour = vec4<f32>(rg, ba);
    out.depth = bitcast<f32>(cache[b + 2u]);
    out.falloff = bitcast<f32>(cache[b + 7u]);
    return out;
}

struct FOut {
    @location(0) colour: vec4<f32>,
    @location(1) depth: vec4<f32>,
}

@fragment
fn fs(in: VOut) -> FOut {
    let a = dot(in.uv, in.uv);
    if (a > 1.0) {
        discard;
    }
    // Falloff that reaches exactly zero at the quad's edge (PlayCanvas normExp, with this splat's
    // own cutoff in place of their fixed one).
    let edge = exp(-in.falloff);
    let alpha = (exp(-in.falloff * a) - edge) / (1.0 - edge) * in.colour.a;
    if (alpha < 0.0039) {
        discard;
    }
    var out: FOut;
    out.colour = vec4<f32>(in.colour.rgb * alpha, alpha);
    // The same blend accumulates depth weighted by coverage; the resolve pass divides it out.
    out.depth = vec4<f32>(in.depth * alpha, alpha, 0.0, 0.0);
    return out;
}
"#;

const COMPOSITE: &str = r#"
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var accum: texture_2d<f32>;
@group(0) @binding(2) var splat_colour: texture_2d<f32>;
@group(0) @binding(3) var smp: sampler;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(p[i], 0.0, 1.0);
}

struct FOut {
    @location(0) colour: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

// Puts the scan into the full-resolution scene: its colour over the sky (the scan may have been
// drawn at a lower resolution), and its own depth into the depth buffer where the splats are
// opaque enough to be a surface, pushed back by the scene's bias so figures standing in grass
// are not shaved off.
@fragment
fn fs(@builtin(position) p: vec4<f32>) -> FOut {
    let uv = p.xy * u.pool.zw;
    let lo = textureSampleLevel(splat_colour, smp, uv, 0.0);
    var out: FOut;
    out.colour = vec4<f32>(lo.rgb + u.sky.rgb * (1.0 - lo.a), 1.0);
    let size = vec2<f32>(textureDimensions(accum));
    let t = textureLoad(accum, vec2<i32>(min(uv * size, size - vec2<f32>(1.0))), 0);
    if (t.g < u.depth.w) {
        out.depth = 1.0;
    } else {
        let d = max(t.r / t.g - u.depth.z, u.viewport.w);
        out.depth = clamp(-u.depth.x + u.depth.y / d, 0.0, 1.0);
    }
    return out;
}
"#;

const DECODE: &str = r#"
struct Chunk {
    mins: vec4<f32>,
    maxs: vec4<f32>,
    info: vec4<u32>,   // texture width, first range, range count, splats to decode
}
@group(0) @binding(0) var<uniform> chunk: Chunk;
@group(0) @binding(1) var means_l: texture_2d<f32>;
@group(0) @binding(2) var means_u: texture_2d<f32>;
@group(0) @binding(3) var quats: texture_2d<f32>;
@group(0) @binding(4) var scales: texture_2d<f32>;
@group(0) @binding(5) var sh0: texture_2d<f32>;
@group(0) @binding(6) var<storage, read> codebooks: array<f32>;

struct Placement {
    rotation: vec4<f32>,  // scan to world, x y z w
    shift: vec4<f32>,     // translation xyz, uniform scale w
}
@group(1) @binding(0) var<uniform> place: Placement;
@group(1) @binding(1) var<storage, read> ranges: array<vec4<u32>>;
@group(1) @binding(2) var<storage, read_write> records: array<u32>;

const SH_C0: f32 = 0.28209479177387814;
const NORM: f32 = 1.4142135623730951;

fn qmul(a: vec4<f32>, b: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(a.w * b.xyz + b.w * a.xyz + cross(a.xyz, b.xyz), a.w * b.w - dot(a.xyz, b.xyz));
}

fn qrot(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    return v + 2.0 * cross(q.xyz, cross(q.xyz, v) + q.w * v);
}

fn byte_of(v: f32) -> u32 {
    return u32(v * 255.0 + 0.5);
}

@compute @workgroup_size(256)
fn decode(@builtin(global_invocation_id) gid: vec3<u32>) {
    let t = gid.x;
    if (t >= chunk.info.w) {
        return;
    }
    // Which contiguous range of the chunk this thread's splat belongs to.
    var lo = 0u;
    var hi = chunk.info.z;
    while (hi - lo > 1u) {
        let mid = (lo + hi) / 2u;
        if (ranges[chunk.info.y + mid].w <= t) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let r = ranges[chunk.info.y + lo];
    let within = t - r.w;
    let src = r.x + within;
    let dst = r.z + within;
    let width = chunk.info.x;
    let px = vec2<i32>(i32(src % width), i32(src / width));

    // Position: 16 bits per axis over the chunk's log-space range, then undo the log.
    let l = textureLoad(means_l, px, 0).xyz;
    let hgh = textureLoad(means_u, px, 0).xyz;
    let n = (l + hgh * 256.0) / 257.0;
    let v = mix(chunk.mins.xyz, chunk.maxs.xyz, n);
    let local_pos = sign(v) * (exp(abs(v)) - 1.0);

    // Rotation: smallest three components in RGB, index of the largest in alpha minus 252.
    let qp = textureLoad(quats, px, 0);
    let mode = byte_of(qp.w) - 252u;
    let abc = (qp.xyz - vec3<f32>(0.5)) * NORM;
    let d = sqrt(max(0.0, 1.0 - dot(abc, abc)));
    // The stored order is (w, x, y, z); this builds (x, y, z, w).
    var quat = vec4<f32>(abc.x, abc.y, abc.z, d);
    if (mode == 0u) {
        quat = vec4<f32>(abc.x, abc.y, abc.z, d);
    } else if (mode == 1u) {
        quat = vec4<f32>(d, abc.y, abc.z, abc.x);
    } else if (mode == 2u) {
        quat = vec4<f32>(abc.y, d, abc.z, abc.x);
    } else {
        quat = vec4<f32>(abc.y, abc.z, d, abc.x);
    }

    let sp = textureLoad(scales, px, 0).xyz;
    let log_scale = vec3<f32>(codebooks[byte_of(sp.x)], codebooks[byte_of(sp.y)], codebooks[byte_of(sp.z)]);
    let scale = exp(log_scale) * place.shift.w;

    let cp = textureLoad(sh0, px, 0);
    let colour = vec3<f32>(
        codebooks[256u + byte_of(cp.x)],
        codebooks[256u + byte_of(cp.y)],
        codebooks[256u + byte_of(cp.z)]) * SH_C0 + vec3<f32>(0.5);

    let world_pos = qrot(place.rotation, local_pos * place.shift.w) + place.shift.xyz;
    let world_rot = normalize(qmul(place.rotation, quat));

    let base = dst * 8u;
    records[base] = bitcast<u32>(world_pos.x);
    records[base + 1u] = bitcast<u32>(world_pos.y);
    records[base + 2u] = bitcast<u32>(world_pos.z);
    records[base + 3u] = pack4x8unorm(vec4<f32>(clamp(colour, vec3<f32>(0.0), vec3<f32>(1.0)), cp.w));
    records[base + 4u] = pack2x16snorm(world_rot.xy);
    records[base + 5u] = pack2x16snorm(world_rot.zw);
    records[base + 6u] = pack2x16float(scale.xy);
    records[base + 7u] = pack2x16float(vec2<f32>(scale.z, 0.0));
}
"#;

pub fn project() -> String {
    format!("{COMMON}{PROJECT}")
}

pub fn draw() -> String {
    format!("{COMMON}{DRAW}")
}

pub fn composite() -> String {
    format!("{COMMON}{COMPOSITE}")
}

pub fn decode() -> String {
    DECODE.to_owned()
}
