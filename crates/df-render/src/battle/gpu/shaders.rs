//! WGSL for the battlefield passes. Colours are display-referred throughout (textures are plain
//! `rgba8unorm`, never sRGB-converted), so the post pass grades what the scene pass drew.

/// Shared by every scene shader: the per-frame uniform block.
pub(super) const FRAME: &str = r#"
struct Frame {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    eye: vec4<f32>,       // xyz eye, w seconds
    right: vec4<f32>,     // xyz camera right flattened to the ground, w aspect
    viewport: vec4<f32>,  // width px, height px, focal px, unused
    light: vec4<f32>,     // x, z of the lit centre, light radius, restrained content rating
    ground: vec4<f32>,    // centre x, centre z, half size, fall-off
    backdrop: vec4<f32>,  // wall z, half width, base y, height
    spot: vec4<f32>,      // warm pool under the acting figure: x, z, radius, strength
}
@group(0) @binding(0) var<uniform> frame: Frame;
"#;

const BACKDROP: &str = r#"
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) height: f32,
}

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
    let c = corners[i];
    let world = vec3<f32>(
        c.x * frame.backdrop.y + frame.ground.x,
        frame.backdrop.z + c.y * frame.backdrop.w,
        frame.backdrop.x);
    var out: VOut;
    out.pos = frame.view_proj * vec4<f32>(world, 1.0);
    out.uv = vec2<f32>(c.x * 0.5 + 0.5, 1.0 - c.y);
    out.height = world.y;
    return out;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let colour = textureSampleLevel(tex, samp, in.uv, 0.0).rgb;
    // The flat stands behind the far edge of the ground; its foot sinks into darkness there.
    let fog = smoothstep(-0.4, 3.2, in.height);
    return vec4<f32>(colour * mix(0.12, 0.85, fog), 1.0);
}
"#;

const GROUND: &str = r#"
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) world_xz: vec2<f32>,
}

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
    let c = corners[i];
    let xz = frame.ground.xy + c * frame.ground.z;
    var out: VOut;
    out.pos = frame.view_proj * vec4<f32>(xz.x, 0.0, xz.y, 1.0);
    out.world_xz = xz;
    return out;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    // The painting covers the whole ground once; it is not tileable, so it is never repeated.
    let uv = (in.world_xz - frame.ground.xy) / (2.0 * frame.ground.z) + vec2<f32>(0.5);
    // Sharper close up: an unsharp mask against a blurrier mip, plus a fine mottling that fades
    // with distance, so the planks and stones keep detail when the camera is low and near.
    let base = textureSample(tex, samp, uv).rgb;
    let soft = textureSampleLevel(tex, samp, uv, 1.3).rgb;
    // The detail is clamped so a bright highlight never turns into speckle.
    let detail = clamp(base - soft, vec3<f32>(-0.07), vec3<f32>(0.07)) * 1.3;
    var colour = max(base + detail, vec3<f32>(0.0));
    let offset = abs(in.world_xz - frame.ground.xy);
    let edge = frame.ground.z - max(offset.x, offset.y);
    let sides = smoothstep(0.0, max(frame.ground.w, 0.01), edge);
    // The far edge dissolves into the standing painting (wall at z = frame.backdrop.x).
    let far = smoothstep(frame.backdrop.x + 0.2, frame.backdrop.x + 4.5, in.world_xz.y);
    let alpha = sides * far;
    // A pool of light around the fight lifts the middle; the outer edges stay dark.
    let pool = 1.0 - smoothstep(0.0, frame.light.z, distance(in.world_xz, frame.light.xy));
    let to_spot = in.world_xz - frame.spot.xy;
    let spot = frame.spot.w * exp(-dot(to_spot, to_spot) / (frame.spot.z * frame.spot.z));
    let warm = vec3<f32>(1.0, 0.72, 0.4);
    let lit = colour * (0.62 + 1.05 * pool) + colour * warm * (spot * 2.1);
    return vec4<f32>(lit * alpha, alpha);
}
"#;

const DECAL: &str = r#"
struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) c: vec2<f32>,
    @location(1) col: vec4<f32>,
    @location(2) params: vec4<f32>,
}

@vertex
fn vs(
    @builtin(vertex_index) i: u32,
    @location(0) shape: vec4<f32>,   // centre x, centre z, half x, half z
    @location(1) col: vec4<f32>,
    @location(2) params: vec4<f32>,  // kind, glow, active, unused
) -> VOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
    let c = corners[i];
    var out: VOut;
    out.pos = frame.view_proj * vec4<f32>(shape.x + c.x * shape.z, 0.012, shape.y + c.y * shape.w, 1.0);
    out.c = c;
    out.col = col;
    out.params = params;
    return out;
}

fn square_edge(c: vec2<f32>) -> f32 {
    return 1.0 - max(abs(c.x), abs(c.y));
}

fn line(edge: f32, aa: f32, width: f32) -> f32 {
    return 1.0 - smoothstep(width, width + aa * 1.5, edge);
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let kind = i32(in.params.x + 0.5);
    let pulse = 0.5 + 0.5 * sin(frame.eye.w * 4.0);
    let edge = square_edge(in.c);
    // Derivatives are taken once, outside the branches, to stay in uniform control flow.
    let aa = max(fwidth(edge), 0.002);
    let aa_inner = max(fwidth(square_edge(in.c / 0.74)), 0.002);
    if (kind == 0) {
        let core = line(edge, aa, 0.018);
        let halo = exp(-edge * 22.0) * 0.32;
        let l = max(core, halo);
        return vec4<f32>(in.col.rgb * l * in.col.a, l * in.col.a);
    }
    if (kind == 1 || kind == 2) {
        let l = line(edge, aa, 0.05);
        let fill = 0.16 + 0.1 * pulse;
        let a = max(l, fill) * in.col.a;
        return vec4<f32>(in.col.rgb * a, a);
    }
    if (kind == 3) {
        let d = length(in.c);
        let a = (1.0 - smoothstep(0.15, 1.0, d)) * in.col.a;
        return vec4<f32>(in.col.rgb * a, a);
    }
    if (kind == 5) {
        // Difficult ground: a faint diagonal hatch inside the tile.
        let stripe = fract((in.c.x + in.c.y) * 2.6);
        let line = smoothstep(0.38, 0.46, stripe) - smoothstep(0.54, 0.62, stripe);
        let inside = smoothstep(0.0, 0.1, edge);
        let a = line * inside * in.col.a;
        return vec4<f32>(in.col.rgb * a, a);
    }
    if (kind == 6) {
        // A gap (water, a pit): a dark void with a soft lip.
        let lip = smoothstep(0.0, 0.18, edge);
        let a = (0.5 + 0.2 * lip) * in.col.a;
        return vec4<f32>(in.col.rgb * a, a);
    }
    if (kind == 7) {
        // A cover pip: a small shield on the tile edge facing the cover.
        let q = in.c;
        let half_w = 0.75 * (1.0 - 0.9 * clamp(q.y / 0.95, 0.0, 1.0));
        let dx = abs(q.x) - half_w;
        let dy = max(-0.8 - q.y, q.y - 0.95);
        let d = max(dx, dy);
        let fill = 1.0 - smoothstep(-0.02, 0.06, d);
        let rim = (1.0 - smoothstep(-0.2, -0.1, d)) * fill;
        let level = in.params.y;
        var tone = vec3<f32>(0.62, 0.75, 0.92);
        if (level > 1.5) { tone = vec3<f32>(0.95, 0.78, 0.38); }
        if (level > 2.5) { tone = vec3<f32>(0.96, 0.96, 0.98); }
        let body = mix(tone, vec3<f32>(0.06, 0.08, 0.12), rim);
        let a = fill * 0.95;
        return vec4<f32>(body * a, a);
    }
    if (kind == 8) {
        // The ring on the figure being aimed at.
        let d = length(in.c);
        let ring = 1.0 - smoothstep(0.0, 0.1, abs(d - 0.8));
        let glow = exp(-pow(d - 0.8, 2.0) * 40.0) * 0.4;
        let a = max(ring, glow) * in.col.a * (0.75 + 0.25 * pulse);
        return vec4<f32>(in.col.rgb * a, a);
    }
    if (kind == 9) {
        // A rotating reticle: a ring with four ticks.
        let c = in.c;
        let ang = frame.eye.w * 0.9 * (1.0 - in.params.y);
        let r = vec2<f32>(c.x * cos(ang) - c.y * sin(ang), c.x * sin(ang) + c.y * cos(ang));
        let d = length(c);
        let ring = 1.0 - smoothstep(0.0, 0.1, abs(d - 0.62));
        let tick_x = (1.0 - smoothstep(0.04, 0.08, abs(r.x))) * step(0.5, abs(r.y)) * step(abs(r.y), 0.95);
        let tick_y = (1.0 - smoothstep(0.04, 0.08, abs(r.y))) * step(0.5, abs(r.x)) * step(abs(r.x), 0.95);
        let a = max(ring, max(tick_x, tick_y)) * in.col.a;
        return vec4<f32>(in.col.rgb * a, a);
    }
    if (kind == 10) {
        // A thin rotating dashed focus ring.
        let d = length(in.c);
        let ang = atan2(in.c.y, in.c.x) + frame.eye.w * 0.8 * (1.0 - in.params.y);
        let dash = step(0.5, fract(ang * 1.9099));
        let ring = 1.0 - smoothstep(0.0, 0.1, abs(d - 0.74));
        let a = ring * (0.5 + 0.5 * dash) * in.col.a;
        return vec4<f32>(in.col.rgb * a, a);
    }
    if (kind == 11) {
        // Where a figure went down: a dark pool and a dim cross on its tile.
        let wobble = 0.12 * sin(atan2(in.c.y, in.c.x) * 3.0 + 1.0);
        let d = length(in.c * vec2<f32>(1.0, 1.15)) + wobble;
        let pool = (1.0 - smoothstep(0.35, 0.9, d)) * in.col.a;
        let bar_a = 1.0 - smoothstep(0.03, 0.07, abs(in.c.x - in.c.y));
        let bar_b = 1.0 - smoothstep(0.03, 0.07, abs(in.c.x + in.c.y));
        let cross_m = max(bar_a, bar_b) * step(max(abs(in.c.x), abs(in.c.y)), 0.55) * 0.55;
        let a = max(pool, cross_m * in.params.y);
        return vec4<f32>(in.col.rgb * a, a);
    }
    // Markers under figures: a warm glow disc, an inner outlined tile and, when acting, a pulse.
    let d = length(in.c);
    let glow = exp(-d * d * 2.6) * (0.42 + 0.5 * in.params.z * (0.7 + 0.3 * pulse));
    let inner = square_edge(in.c / 0.74);
    let outline = line(inner, aa_inner, 0.07) * step(0.0, inner);
    let warm = vec3<f32>(1.0, 0.72, 0.36);
    let rgb = warm * glow * 0.6 + in.col.rgb * outline * (0.75 + 0.5 * in.params.z);
    let a = outline * in.col.a * 0.9;
    return vec4<f32>(rgb, a);
}
"#;

const SPLAT: &str = r#"
struct Splat {
    centre: vec3<f32>,
    colour: u32,
    cov_a: vec4<f32>,   // xx, xy, xz, yy
    cov_b: vec2<f32>,   // yz, zz
}
@group(1) @binding(0) var<storage, read> splats: array<Splat>;
@group(1) @binding(1) var<storage, read> order: array<u32>;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) p: vec2<f32>,
    @location(1) colour: vec4<f32>,
}

@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VOut {
    var corners = array<vec2<f32>, 4>(
        vec2<f32>(-2.0, -2.0), vec2<f32>(2.0, -2.0), vec2<f32>(-2.0, 2.0), vec2<f32>(2.0, 2.0));
    var out: VOut;
    out.pos = vec4<f32>(0.0, 0.0, 2.0, 1.0);
    let s = splats[order[ii]];
    let view_pos = (frame.view * vec4<f32>(s.centre, 1.0)).xyz;
    if (view_pos.z > -0.15) {
        return out;
    }
    let clip = frame.view_proj * vec4<f32>(s.centre, 1.0);
    let d = -view_pos.z;
    let cov = mat3x3<f32>(
        vec3<f32>(s.cov_a.x, s.cov_a.y, s.cov_a.z),
        vec3<f32>(s.cov_a.y, s.cov_a.w, s.cov_b.x),
        vec3<f32>(s.cov_a.z, s.cov_b.x, s.cov_b.y));
    let rv = mat3x3<f32>(frame.view[0].xyz, frame.view[1].xyz, frame.view[2].xyz);
    let cv = rv * cov * transpose(rv);
    let f = frame.viewport.z;
    let j0 = vec3<f32>(f / d, 0.0, f * view_pos.x / (d * d));
    let j1 = vec3<f32>(0.0, f / d, f * view_pos.y / (d * d));
    let cj0 = cv * j0;
    let cj1 = cv * j1;
    let a = dot(j0, cj0) + 0.3;
    let b = dot(j0, cj1);
    let c = dot(j1, cj1) + 0.3;
    let mid = 0.5 * (a + c);
    let radius = length(vec2<f32>(0.5 * (a - c), b));
    let l1 = mid + radius;
    let l2 = max(mid - radius, 0.1);
    let diag = normalize(vec2<f32>(b, l1 - a));
    let major = min(sqrt(2.0 * l1), 1024.0) * diag;
    let minor = min(sqrt(2.0 * l2), 1024.0) * vec2<f32>(diag.y, -diag.x);
    let corner = corners[vi];
    let offset_px = corner.x * major + corner.y * minor;
    let ndc = clip.xy / clip.w + offset_px * 2.0 / frame.viewport.xy;
    out.pos = vec4<f32>(ndc * clip.w, clip.z, clip.w);
    out.p = corner;
    out.colour = unpack4x8unorm(s.colour);
    return out;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let a = -dot(in.p, in.p);
    if (a < -4.0) {
        discard;
    }
    let weight = exp(a) * in.colour.a;
    return vec4<f32>(in.colour.rgb * weight, weight);
}
"#;

const POST: &str = r#"
struct Post {
    grade: vec4<f32>,      // exposure, contrast, saturation, enabled
    shadow: vec4<f32>,
    highlight: vec4<f32>,
    tilt: vec4<f32>,       // centre, band, fall-off, blur px
    misc: vec4<f32>,       // vignette, flash strength, fade, tilt-shift on
    flash: vec4<f32>,      // flash colour rgb
    shake: vec4<f32>,      // dx px, dy px, texel w, texel h
}
@group(0) @binding(0) var<uniform> post: Post;
@group(0) @binding(1) var scene: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VOut {
    var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var out: VOut;
    out.pos = vec4<f32>(p[i], 0.0, 1.0);
    out.uv = vec2<f32>(p[i].x * 0.5 + 0.5, 0.5 - p[i].y * 0.5);
    return out;
}

fn fetch(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(scene, samp, uv, 0.0).rgb;
}

fn blurred(uv: vec2<f32>) -> vec3<f32> {
    let distance_from_centre = abs(uv.y - post.tilt.x);
    let edge = post.tilt.y * 0.5;
    let amount = smoothstep(edge, edge + max(post.tilt.z * 0.5, 0.0001), distance_from_centre);
    let radius = post.tilt.w * amount * post.misc.w;
    if (radius < 0.3) {
        return fetch(uv);
    }
    var sum = fetch(uv);
    var weight = 1.0;
    for (var k = 1; k <= 10; k = k + 1) {
        let t = f32(k) / 10.0;
        let angle = f32(k) * 2.39996;
        let o = vec2<f32>(cos(angle), sin(angle)) * sqrt(t) * radius * post.shake.zw;
        sum = sum + fetch(uv + o);
        weight = weight + 1.0;
    }
    return sum / weight;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let uv = in.uv + post.shake.xy * post.shake.zw;
    var colour = blurred(uv);
    if (post.grade.w > 0.5) {
        var rgb = max(colour, vec3<f32>(0.0)) * exp2(post.grade.x);
        let luma = dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
        rgb = mix(vec3<f32>(luma), rgb, post.grade.z);
        rgb = pow(max(rgb, vec3<f32>(0.0)), vec3<f32>(post.grade.y));
        rgb = rgb * mix(post.shadow.xyz, post.highlight.xyz, clamp(luma, 0.0, 1.0));
        colour = clamp(rgb, vec3<f32>(0.0), vec3<f32>(4.0));
    }
    let centred = (in.uv - vec2<f32>(0.5)) * vec2<f32>(1.15, 1.0);
    let vignette = smoothstep(0.32, 0.95, length(centred) * 1.25);
    colour = colour * (1.0 - vignette * post.misc.x);
    colour = colour + post.flash.rgb * post.misc.y * 0.55;
    colour = colour * (1.0 - post.misc.z);
    return vec4<f32>(clamp(colour, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
"#;

pub fn backdrop() -> String {
    format!("{FRAME}{BACKDROP}")
}

pub fn ground() -> String {
    format!("{FRAME}{GROUND}")
}

pub fn decal() -> String {
    format!("{FRAME}{DECAL}")
}

pub fn splat() -> String {
    format!("{FRAME}{SPLAT}")
}

pub fn post() -> String {
    POST.to_owned()
}
