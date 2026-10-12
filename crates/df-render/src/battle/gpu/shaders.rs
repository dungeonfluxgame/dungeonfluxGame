//! WGSL for the battlefield passes. Colours are display-referred throughout (textures are plain
//! `rgba8unorm`, never sRGB-converted), so the post pass grades what the scene pass drew.

/// Shared by every scene shader: the per-frame uniform block.
const FRAME: &str = r#"
struct Frame {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    eye: vec4<f32>,       // xyz eye, w seconds
    right: vec4<f32>,     // xyz camera right flattened to the ground, w aspect
    viewport: vec4<f32>,  // width px, height px, focal px, unused
    light: vec4<f32>,     // x, z of the lit centre, light radius, unused
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
    let colour = textureSample(tex, samp, uv).rgb;
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
// Ground height under the grid: [origin x, origin z, step, columns, rows, 0, 0, 0, heights...].
@group(1) @binding(0) var<storage, read> terrain: array<f32>;

fn ground_y(x: f32, z: f32) -> f32 {
    let step = terrain[2];
    let cols = i32(terrain[3]);
    let rows = i32(terrain[4]);
    let u = clamp((x - terrain[0]) / step, 0.0, f32(cols - 1));
    let v = clamp((z - terrain[1]) / step, 0.0, f32(rows - 1));
    let c = min(i32(floor(u)), cols - 2);
    let r = min(i32(floor(v)), rows - 2);
    let fx = u - f32(c);
    let fz = v - f32(r);
    let base = 8 + r * cols + c;
    return mix(
        mix(terrain[base], terrain[base + 1], fx),
        mix(terrain[base + cols], terrain[base + cols + 1], fx),
        fz);
}

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
    // Each decal is a 6 by 6 patch of quads so it follows uneven ground.
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0));
    let quad = i / 6u;
    let cell = vec2<f32>(f32(quad % 6u), f32(quad / 6u));
    let c = (cell + corners[i % 6u]) / 3.0 - vec2<f32>(1.0);
    let x = shape.x + c.x * shape.z;
    let z = shape.y + c.y * shape.w;
    var out: VOut;
    out.pos = frame.view_proj * vec4<f32>(x, ground_y(x, z) + 0.012, z, 1.0);
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

const FIGURE: &str = r#"
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) look: vec4<f32>,   // red flash, heal glow, alpha, kind
    @location(2) tint: vec4<f32>,   // tint rgb, has texture
    @location(3) side: f32,
}

@vertex
fn vs(
    @builtin(vertex_index) i: u32,
    @location(0) place: vec4<f32>,   // x, y, z of the feet, visible height
    @location(1) pose: vec4<f32>,    // lean, alpha, red flash, heal glow
    @location(2) tint: vec4<f32>,    // rgb tint, breath
    @location(3) kind: vec4<f32>,    // aspect, kind (0 cut-out, 1 token), has texture, side
    @location(4) bounds: vec4<f32>,  // visible top, bottom, centre x (picture fractions)
) -> VOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
    let c = corners[i];
    var bottom = 0.0;
    var span = 1.0;
    var half_width = 0.0;
    var shift = 0.0;
    if (kind.y < 0.5) {
        // The visible figure (not the canvas) is `place.w` metres tall, its feet on the ground and
        // centred under the point it stands on.
        let full = place.w / max(bounds.y - bounds.x, 0.05);
        span = full * tint.w;
        bottom = -(1.0 - bounds.y) * span;
        half_width = full * kind.x * 0.5;
        shift = (bounds.z - 0.5) * full * kind.x;
    } else {
        // A round token about 0.9 m across standing on the tile.
        span = 0.95;
        bottom = 0.18;
        half_width = 0.475;
    }
    let local = vec2<f32>(c.x * half_width - shift, bottom + c.y * span);
    let s = sin(pose.x);
    let k = cos(pose.x);
    var rotated = vec2<f32>(local.x * k + local.y * s, -local.x * s + local.y * k);
    let world = vec3<f32>(place.xyz) + frame.right.xyz * rotated.x + vec3<f32>(0.0, rotated.y, 0.0);
    var out: VOut;
    out.pos = frame.view_proj * vec4<f32>(world, 1.0);
    out.uv = vec2<f32>(c.x * 0.5 + 0.5, 1.0 - c.y);
    out.look = vec4<f32>(pose.z, pose.w, pose.y, kind.y);
    out.tint = vec4<f32>(tint.rgb, kind.z);
    out.side = kind.w;
    return out;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let sample = textureSampleLevel(tex, samp, in.uv, 0.0);
    let aa = max(fwidth(length(in.uv * 2.0 - vec2<f32>(1.0))), 0.01);
    var rgb = sample.rgb;
    var a = sample.a;
    if (in.look.w > 0.5) {
        // Token: a round disc with a gold or blood rim; flat colour when the portrait is absent.
        let p = in.uv * 2.0 - vec2<f32>(1.0);
        let d = length(p);
        let disc = 1.0 - smoothstep(1.0 - aa, 1.0, d);
        let rim = smoothstep(0.82, 0.86, d);
        let rim_colour = select(vec3<f32>(0.93, 0.76, 0.42), vec3<f32>(0.78, 0.2, 0.18), in.side > 0.5);
        var body = rgb;
        if (in.tint.w < 0.5) {
            body = mix(vec3<f32>(0.12, 0.16, 0.22), vec3<f32>(0.3, 0.34, 0.42), 1.0 - d);
            if (in.side > 0.5) {
                body = mix(vec3<f32>(0.2, 0.07, 0.07), vec3<f32>(0.42, 0.14, 0.12), 1.0 - d);
            }
        }
        rgb = mix(body, rim_colour, rim) * disc;
        a = disc;
    }
    rgb = rgb * in.tint.rgb;
    let red = in.look.x;
    rgb = mix(rgb, vec3<f32>(0.95, 0.12, 0.08) * a, red * 0.6);
    let glow = in.look.y;
    rgb = rgb + vec3<f32>(0.45, 0.95, 0.55) * a * glow * 0.4;
    let alpha = in.look.z;
    if (a < 0.004) {
        discard;
    }
    return vec4<f32>(rgb * alpha, a * alpha);
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

pub fn figure() -> String {
    format!("{FRAME}{FIGURE}")
}

pub fn post() -> String {
    POST.to_owned()
}
