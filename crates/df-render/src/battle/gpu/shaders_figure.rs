//! WGSL for everything drawn standing on the battlefield: figures and obstacle props (one
//! billboard shader), the status-effect overlay (shapes and particles around a figure) and the
//! arrow. The figure shader is parameterised by three small per-instance vectors: which status
//! treatments apply to the body, to the edge and as overlays (bit masks the pure core resolved
//! with its combination rules), the slump pose, and a few animation inputs. Time-based motion
//! runs on the frame clock.

use super::shaders::FRAME;
use crate::battle::Status;

/// `const ST_<NAME>: u32 = <bit>u;` for every status, so the shader and the Rust bit order can
/// never drift apart.
fn status_consts() -> String {
    Status::ALL
        .iter()
        .enumerate()
        .map(|(bit, status)| {
            format!(
                "const ST_{}: u32 = {bit}u;\n",
                status.id().to_ascii_uppercase()
            )
        })
        .collect()
}

const HELPERS: &str = r#"
fn has_bit(lo: f32, hi: f32, bit: u32) -> bool {
    let word = select(u32(hi), u32(lo), bit < 14u);
    let shift = select(bit - 14u, bit, bit < 14u);
    return ((word >> shift) & 1u) == 1u;
}

fn hash21(p: vec2<f32>) -> f32 {
    var q = fract(p * vec2<f32>(123.34, 456.21));
    q = q + dot(q, q + vec2<f32>(45.32, 45.32));
    return fract(q.x * q.y);
}

fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (vec2<f32>(3.0, 3.0) - 2.0 * f);
    return mix(
        mix(hash21(i), hash21(i + vec2<f32>(1.0, 0.0)), u.x),
        mix(hash21(i + vec2<f32>(0.0, 1.0)), hash21(i + vec2<f32>(1.0, 1.0)), u.x),
        u.y);
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
    @location(4) bounds: vec3<f32>, // visible top, bottom, centre
    @location(5) fx1: vec4<f32>,    // body mask lo/hi, edge mask lo/hi
    @location(6) fx2: vec4<f32>,    // overlay mask lo/hi, slump, thud
    @location(7) fx3: vec4<f32>,    // unused, dim, heartbeat, phase
    @location(8) world: vec3<f32>,
}

@vertex
fn vs(
    @builtin(vertex_index) i: u32,
    @location(0) place: vec4<f32>,   // x, y, z of the feet, visible height
    @location(1) pose: vec4<f32>,    // lean, alpha, red flash, heal glow
    @location(2) tint: vec4<f32>,    // rgb tint, breath
    @location(3) kind: vec4<f32>,    // aspect (or half width), kind, has texture, side + 2 * flip
    @location(4) bounds: vec4<f32>,  // visible top, bottom, centre x (picture fractions)
    @location(5) fx1: vec4<f32>,
    @location(6) fx2: vec4<f32>,
    @location(7) fx3: vec4<f32>,
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
    } else if (kind.y < 1.5) {
        // A round token about 0.9 m across standing on the tile.
        span = 0.95;
        bottom = 0.18;
        half_width = 0.475;
    } else {
        // A plain block standing in for prop art that did not load.
        span = place.w;
        half_width = kind.x;
    }
    var local = vec2<f32>(c.x * half_width - shift, bottom + c.y * span);
    // Slumped to the ground: squashed down and a little wider, tipping toward the camera, and a
    // quick extra squash on landing. The feet stay planted.
    let slump = fx2.z;
    let thud = fx2.w;
    local = vec2<f32>(
        local.x * (1.0 + 0.10 * slump) * (1.0 + 0.08 * thud),
        local.y * (1.0 - 0.48 * slump) * (1.0 - 0.10 * thud));
    let tip = 0.6 * slump;
    let toward = vec3<f32>(-frame.right.z, 0.0, frame.right.x);
    let depth = local.y * sin(tip);
    local.y = local.y * cos(tip);
    let s = sin(pose.x);
    let k = cos(pose.x);
    let rotated = vec2<f32>(local.x * k + local.y * s, -local.x * s + local.y * k);
    let world = vec3<f32>(place.xyz) + frame.right.xyz * rotated.x
        + vec3<f32>(0.0, rotated.y, 0.0) + toward * depth;
    var out: VOut;
    out.pos = frame.view_proj * vec4<f32>(world, 1.0);
    let flip = kind.w > 1.5;
    let u = c.x * 0.5 + 0.5;
    out.uv = vec2<f32>(select(u, 1.0 - u, flip), 1.0 - c.y);
    out.look = vec4<f32>(pose.z, pose.w, pose.y, kind.y);
    out.tint = vec4<f32>(tint.rgb, kind.z);
    out.side = select(kind.w, kind.w - 2.0, flip);
    out.bounds = vec3<f32>(bounds.x, bounds.y, bounds.z);
    out.fx1 = fx1;
    out.fx2 = fx2;
    out.fx3 = fx3;
    out.world = world;
    return out;
}

fn shade(in: VOut) -> vec4<f32> {
    let aa = max(fwidth(length(in.uv * 2.0 - vec2<f32>(1.0))), 0.01);
    // Pick the mip from the screen-space derivatives while the control flow is still uniform, so
    // a sprite shrunk far below its 1024 pixels (and one squashed to the ground) stays smooth.
    let dims = vec2<f32>(textureDimensions(tex));
    let ddx = dpdx(in.uv * dims);
    let ddy = dpdy(in.uv * dims);
    let lod = max(0.5 * log2(max(dot(ddx, ddx), dot(ddy, ddy))) - 0.35, 0.0);
    let tt = frame.eye.w;
    let restrained = frame.light.w > 0.5;
    let alpha = in.look.z;

    if (in.look.w > 1.5) {
        // Missing prop art: a dark block with a lit top edge, never nothing.
        let across = abs(in.uv.x * 2.0 - 1.0);
        let shade = 0.62 + 0.38 * (1.0 - across * across);
        var block = vec3<f32>(0.17, 0.16, 0.18) * shade * (0.75 + 0.45 * (1.0 - in.uv.y));
        let lit_top = 1.0 - smoothstep(0.0, 0.09, in.uv.y);
        block = mix(block, vec3<f32>(0.5, 0.46, 0.4), lit_top);
        return vec4<f32>(block * alpha, alpha);
    }

    let top = in.bounds.x;
    let bottom = in.bounds.y;
    let hfrac = clamp((bottom - in.uv.y) / max(bottom - top, 0.05), 0.0, 1.0);
    let body_lo = in.fx1.x;
    let body_hi = in.fx1.y;
    let edge_lo = in.fx1.z;
    let edge_hi = in.fx1.w;
    let burning = has_bit(edge_lo, edge_hi, ST_BURNING);
    let invisible = has_bit(edge_lo, edge_hi, ST_INVISIBLE);
    var uv = in.uv;
    if (burning) {
        uv.x = uv.x + sin(tt * 7.0 + uv.y * 38.0) * 0.004 * (0.4 + 0.6 * hfrac);
    }
    if (invisible) {
        uv.x = uv.x + sin(tt * 3.0 + uv.y * 30.0) * 0.006;
    }
    let sample = textureSampleLevel(tex, samp, uv, lod);
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

    // ---- the body: status colour treatments (the pure core already resolved which apply)
    let luma = dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    if (has_bit(body_lo, body_hi, ST_PETRIFIED)) {
        // Full desaturation to cracked grey stone with a slow specular sheen.
        let crack_n = vnoise(in.uv * vec2<f32>(18.0, 26.0));
        let crack = 1.0 - smoothstep(0.0, 0.035, abs(crack_n - 0.5));
        let speck = vnoise(in.uv * 90.0);
        var stone = vec3<f32>(luma) * vec3<f32>(0.8, 0.8, 0.83) * (0.85 + 0.3 * speck);
        stone = stone * (1.0 - 0.55 * crack * step(0.5, a));
        let sheen = pow(max(0.0, sin((in.uv.x + in.uv.y * 0.6) * 9.0 - tt * 0.8)), 14.0) * 0.32;
        rgb = stone + vec3<f32>(sheen) * a;
    } else {
        if (has_bit(body_lo, body_hi, ST_POISONED)) {
            let pulse = 0.55 + 0.15 * sin(tt * 1.6);
            rgb = mix(rgb, rgb * vec3<f32>(0.55, 1.05, 0.45) + vec3<f32>(0.0, 0.05, 0.0) * a, pulse);
        }
        if (has_bit(body_lo, body_hi, ST_BURNING)) {
            rgb = rgb * vec3<f32>(1.12, 0.88, 0.72);
            let flicker = 0.3 + 0.12 * sin(tt * 9.0 + in.uv.x * 20.0);
            rgb = rgb + vec3<f32>(1.0, 0.45, 0.1) * a * pow(1.0 - hfrac, 2.0) * flicker;
        }
        if (has_bit(body_lo, body_hi, ST_FRIGHTENED)) {
            rgb = mix(rgb, vec3<f32>(luma) * vec3<f32>(0.92, 0.96, 1.08), 0.5);
        }
        if (has_bit(body_lo, body_hi, ST_CHARMED)) {
            rgb = rgb * vec3<f32>(1.08, 0.93, 1.0);
        }
        if (has_bit(body_lo, body_hi, ST_CURSED)) {
            rgb = rgb * vec3<f32>(0.78, 0.68, 1.08);
        }
        if (has_bit(body_lo, body_hi, ST_PARALYZED)) {
            rgb = mix(rgb, vec3<f32>(luma) * vec3<f32>(0.85, 0.95, 1.2) + vec3<f32>(0.06, 0.08, 0.1) * a, 0.55);
        }
        if (has_bit(body_lo, body_hi, ST_RAGING)) {
            rgb = rgb * vec3<f32>(1.22, 0.74, 0.68);
        }
        if (has_bit(body_lo, body_hi, ST_HIDDEN)) {
            rgb = mix(rgb * 0.55, vec3<f32>(0.02, 0.02, 0.04) * a, 0.3);
        }
        if (has_bit(body_lo, body_hi, ST_EXHAUSTION)) {
            rgb = mix(rgb, vec3<f32>(luma), 0.3);
        }
        if (has_bit(body_lo, body_hi, ST_BLOODIED)) {
            // A subtle red wash on the lower half; grey when the rating is restrained.
            let low = 1.0 - smoothstep(0.0, 0.55, hfrac);
            let grain = vnoise(in.uv * vec2<f32>(10.0, 14.0));
            let wash = low * (0.35 + 0.5 * grain);
            let blood = select(vec3<f32>(0.55, 0.03, 0.03) * a, vec3<f32>(luma) * 0.6, restrained);
            rgb = mix(rgb, blood, wash * 0.45);
        }
        if (has_bit(body_lo, body_hi, ST_BLINDED)) {
            // A dark band across the eye line.
            let band = 1.0 - smoothstep(0.03, 0.06, abs(hfrac - 0.86));
            rgb = mix(rgb, vec3<f32>(0.01) * a, band * 0.9 * step(0.5, a));
        }
    }
    // Downed, unconscious or incapacitated: dimmed and greyed.
    let dim = in.fx3.y;
    rgb = mix(rgb, vec3<f32>(luma) * 0.75, dim * 0.65) * (1.0 - dim * 0.5);
    rgb = rgb * in.tint.rgb;
    rgb = mix(rgb, vec3<f32>(0.95, 0.12, 0.08) * a, in.look.x * 0.6);
    rgb = rgb + vec3<f32>(0.45, 0.95, 0.55) * a * in.look.y * 0.4;

    // ---- the edge: rim light, crackle, outline
    var glow = vec3<f32>(0.0);
    var glow_a = 0.0;
    var rim = 0.0;
    var rim_out = 0.0;
    if ((edge_lo + edge_hi) > 0.0) {
        let r = 0.011;
        let a1 = textureSampleLevel(tex, samp, uv + vec2<f32>(r, 0.0), lod).a;
        let a2 = textureSampleLevel(tex, samp, uv - vec2<f32>(r, 0.0), lod).a;
        let a3 = textureSampleLevel(tex, samp, uv + vec2<f32>(0.0, r), lod).a;
        let a4 = textureSampleLevel(tex, samp, uv - vec2<f32>(0.0, r), lod).a;
        rim_out = (1.0 - a) * max(max(a1, a2), max(a3, a4));
        let rim_in = a * (1.0 - min(min(a1, a2), min(a3, a4)));
        rim = clamp(rim_out + rim_in, 0.0, 1.0);
        if (burning) {
            glow = glow + vec3<f32>(1.0, 0.5, 0.15) * (0.8 + 0.4 * sin(tt * 9.0 + uv.y * 20.0));
            glow_a = glow_a + 0.9;
        }
        if (has_bit(edge_lo, edge_hi, ST_PARALYZED)) {
            let zap = step(0.62, vnoise(uv * vec2<f32>(46.0, 46.0) + vec2<f32>(floor(tt * 14.0), 0.0)));
            glow = glow + vec3<f32>(0.7, 0.92, 1.0) * (0.5 + 0.9 * zap);
            glow_a = glow_a + 0.7;
        }
        if (has_bit(edge_lo, edge_hi, ST_CHARMED)) {
            glow = glow + vec3<f32>(1.0, 0.55, 0.72) * 0.8;
            glow_a = glow_a + 0.6;
        }
        if (has_bit(edge_lo, edge_hi, ST_RAGING)) {
            glow = glow + vec3<f32>(1.0, 0.14, 0.08) * (0.9 + 0.3 * sin(tt * 6.0));
            glow_a = glow_a + 0.8;
        }
        if (has_bit(edge_lo, edge_hi, ST_HIDDEN)) {
            // Outlined only for its own side.
            glow = glow + select(vec3<f32>(0.5, 0.86, 1.0), vec3<f32>(1.0, 0.34, 0.28), in.side > 0.5) * 0.45;
            glow_a = glow_a + 0.35;
        }
        if (invisible) {
            glow = glow + vec3<f32>(0.75, 0.95, 1.0) * 0.6;
            glow_a = glow_a + 0.7;
            rgb = rgb * vec3<f32>(0.95, 1.1, 1.25);
        }
        glow_a = clamp(glow_a, 0.0, 1.0);
    }

    let out_a = max(a * alpha, rim_out * glow_a);
    if (out_a < 0.004) {
        discard;
    }
    let out_rgb = rgb * alpha + glow * rim * min(glow_a, 1.0) * 0.8;
    return vec4<f32>(out_rgb, out_a);
}

@fragment
fn fs(in: VOut) -> FOut {
    var out: FOut;
    out.colour = shade(in);
    out.depth = standing_depth(in.world);
    return out;
}
"#;

const EFFECTS: &str = r#"
struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) p: vec2<f32>,      // position around the figure in units of its height
    @location(1) fx2: vec4<f32>,
    @location(2) fx3: vec4<f32>,
    @location(3) world: vec3<f32>,
}

@vertex
fn vs(
    @builtin(vertex_index) i: u32,
    @location(0) place: vec4<f32>,
    @location(1) pose: vec4<f32>,
    @location(2) tint: vec4<f32>,
    @location(3) kind: vec4<f32>,
    @location(4) bounds: vec4<f32>,
    @location(5) fx1: vec4<f32>,
    @location(6) fx2: vec4<f32>,
    @location(7) fx3: vec4<f32>,
) -> VOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
    let c = corners[i];
    let h = place.w;
    let x = c.x * 0.95;
    let y = mix(-0.05, 1.55, c.y);
    let world = place.xyz + frame.right.xyz * (x * h) + vec3<f32>(0.0, y * h, 0.0);
    var out: VOut;
    out.pos = frame.view_proj * vec4<f32>(world, 1.0);
    out.p = vec2<f32>(x, y);
    out.fx2 = fx2;
    out.fx3 = fx3;
    out.world = world;
    return out;
}

fn blob(p: vec2<f32>, c: vec2<f32>, r: f32) -> f32 {
    let d = length(p - c) / r;
    return exp(-d * d * 2.0);
}

fn heart(q: vec2<f32>) -> f32 {
    // Implicit heart curve: negative inside. `q` is in units of the heart's size.
    let x = q.x;
    let y = -q.y + 0.25;
    let f = pow(x * x + y * y - 1.0, 3.0) - x * x * y * y * y;
    return 1.0 - smoothstep(-0.25, 0.1, f);
}

fn shade(in: VOut) -> vec4<f32> {
    let lo = in.fx2.x;
    let hi = in.fx2.y;
    let slump = in.fx2.z;
    let tt = frame.eye.w;
    let phase = in.fx3.w;
    let restrained = frame.light.w > 0.5;
    let p = in.p;
    let head = 1.1 - 0.5 * slump;
    var add = vec3<f32>(0.0);
    var cov = vec4<f32>(0.0);

    if (has_bit(lo, hi, ST_POISONED)) {
        // Sickly motes drifting slowly up.
        for (var n = 0u; n < 8u; n = n + 1u) {
            let fn_ = f32(n);
            let s = hash21(vec2<f32>(fn_, phase));
            let t = fract(tt * 0.11 + s);
            let x = (hash21(vec2<f32>(fn_ * 3.1, phase)) - 0.5) * 0.6 + 0.04 * sin(tt * 0.9 + fn_);
            let fade = sin(t * 3.14159);
            add = add + vec3<f32>(0.5, 1.0, 0.35) * blob(p, vec2<f32>(x, t * 1.2), 0.035) * fade * 0.9;
        }
    }
    if (has_bit(lo, hi, ST_BURNING)) {
        // Small sparks rising and flickering out.
        for (var n = 0u; n < 8u; n = n + 1u) {
            let fn_ = f32(n);
            let s = hash21(vec2<f32>(fn_ + 9.0, phase));
            let t = fract(tt * 0.38 + s);
            let x = (hash21(vec2<f32>(fn_ * 2.3, phase + 4.0)) - 0.5) * 0.55 + 0.05 * sin(tt * 5.0 + fn_);
            let spark = blob(p, vec2<f32>(x, 0.1 + t * 1.3), 0.02) * (1.0 - t);
            add = add + vec3<f32>(1.0, 0.55 + 0.3 * s, 0.2) * spark * 1.4;
        }
    }
    if (has_bit(lo, hi, ST_STUNNED)) {
        // A ring of orbiting stars above the head.
        for (var n = 0u; n < 4u; n = n + 1u) {
            let angle = tt * 3.0 + f32(n) * 1.5708 + phase;
            let c = vec2<f32>(0.26 * cos(angle), head + 0.07 + 0.07 * sin(angle));
            let near = 0.55 + 0.45 * sin(angle);
            let d = p - c;
            let spikes = max(exp(-d.x * d.x * 4000.0 - d.y * d.y * 90.0), exp(-d.y * d.y * 4000.0 - d.x * d.x * 90.0));
            add = add + vec3<f32>(1.0, 0.9, 0.4) * (blob(p, c, 0.028) + 0.6 * spikes) * near;
        }
    }
    if (has_bit(lo, hi, ST_CHARMED)) {
        // Soft hearts drifting up from the figure.
        for (var n = 0u; n < 4u; n = n + 1u) {
            let fn_ = f32(n);
            let s = hash21(vec2<f32>(fn_ + 3.0, phase));
            let t = fract(tt * 0.14 + fn_ * 0.25 + s * 0.1);
            let c = vec2<f32>(0.2 * sin(tt * 1.3 + fn_ * 2.0), 0.35 + t * 0.95);
            let size = 0.045 * (0.7 + 0.5 * s);
            let m = heart((p - c) / size) * sin(t * 3.14159);
            cov = cov + vec4<f32>(vec3<f32>(1.0, 0.45, 0.65) * m * 0.85, m * 0.85);
        }
    }
    if (has_bit(lo, hi, ST_BLESSED) || has_bit(lo, hi, ST_INSPIRED)) {
        // A warm gold aura from below, with rising specks.
        let column = exp(-pow(p.x / 0.4, 2.0)) * pow(1.0 - clamp(p.y / 1.15, 0.0, 1.0), 1.6);
        add = add + vec3<f32>(1.0, 0.82, 0.4) * column * (0.7 + 0.12 * sin(tt * 2.0));
        let specks = select(4u, 8u, has_bit(lo, hi, ST_INSPIRED));
        for (var n = 0u; n < specks; n = n + 1u) {
            let fn_ = f32(n);
            let s = hash21(vec2<f32>(fn_ + 21.0, phase));
            let t = fract(tt * 0.2 + s);
            let x = (hash21(vec2<f32>(fn_ * 1.7, phase + 2.0)) - 0.5) * 0.5;
            add = add + vec3<f32>(1.0, 0.9, 0.55) * blob(p, vec2<f32>(x, t * 1.1), 0.018) * sin(t * 3.14159);
        }
    }
    if (has_bit(lo, hi, ST_CURSED)) {
        // Cold violet smoke curling up from the ground.
        let n = vnoise(vec2<f32>(p.x * 3.5, p.y * 2.2 - tt * 0.5));
        let m = smoothstep(0.35, 0.75, n) * (1.0 - smoothstep(0.0, 0.8, p.y)) * (1.0 - smoothstep(0.2, 0.6, abs(p.x)));
        cov = cov + vec4<f32>(vec3<f32>(0.45, 0.25, 0.8) * m * 0.55, m * 0.55);
    }
    if (has_bit(lo, hi, ST_RESTRAINED)) {
        // Two chain bands around the body.
        for (var n = 0u; n < 2u; n = n + 1u) {
            let yc = 0.3 + 0.25 * f32(n);
            let band = 1.0 - smoothstep(0.016, 0.03, abs(p.y - yc - 0.025 * sin(p.x * 18.0)));
            let inside = 1.0 - smoothstep(0.24, 0.28, abs(p.x));
            let links = 0.55 + 0.45 * step(0.5, fract(p.x * 14.0 + f32(n) * 0.5));
            let m = band * inside;
            cov = cov + vec4<f32>(vec3<f32>(0.6, 0.63, 0.7) * links * m, m);
        }
    }
    if (has_bit(lo, hi, ST_GRAPPLED)) {
        // A pair of gripping hands at the figure's sides.
        for (var n = 0u; n < 2u; n = n + 1u) {
            let side = select(-1.0, 1.0, n == 1u);
            let c = vec2<f32>(0.27 * side, 0.5);
            let d = (p - c) * vec2<f32>(1.0, 0.55);
            let palm = 1.0 - smoothstep(0.045, 0.06, length(d));
            var fingers = 0.0;
            for (var f = 0u; f < 4u; f = f + 1u) {
                let fc = c + vec2<f32>(-0.05 * side, 0.034 * (f32(f) - 1.5));
                fingers = max(fingers, 1.0 - smoothstep(0.012, 0.02, length((p - fc) * vec2<f32>(0.55, 1.0))));
            }
            let m = max(palm, fingers);
            cov = cov + vec4<f32>(vec3<f32>(0.5, 0.44, 0.4) * m, m);
        }
    }
    if (has_bit(lo, hi, ST_DEAFENED)) {
        // A small muted-wave glyph beside the head.
        let c = vec2<f32>(0.34, head + 0.02);
        let d = p - c;
        let disc = (1.0 - smoothstep(0.055, 0.07, length(d))) * 0.55;
        let arcs = max(
            1.0 - smoothstep(0.004, 0.009, abs(length(d) - 0.034)),
            1.0 - smoothstep(0.004, 0.009, abs(length(d) - 0.054)));
        let slash = (1.0 - smoothstep(0.004, 0.009, abs(d.x + d.y))) * (1.0 - smoothstep(0.06, 0.07, length(d)));
        let ink = max(arcs * step(0.0, d.x), slash);
        cov = cov + vec4<f32>(vec3<f32>(0.02, 0.02, 0.04) * disc + vec3<f32>(0.95) * ink, max(disc, ink));
    }
    if (has_bit(lo, hi, ST_DYING)) {
        // A heartbeat flush behind a figure that is down.
        let beat = in.fx3.z;
        let glow = blob(p, vec2<f32>(0.0, 0.22), 0.3 + 0.12 * beat) * beat * 0.55;
        let tone = select(vec3<f32>(0.9, 0.1, 0.1), vec3<f32>(0.8, 0.8, 0.85), restrained);
        add = add + tone * glow;
    }
    let total_a = clamp(cov.a, 0.0, 1.0);
    return vec4<f32>(add + cov.rgb, total_a);
}

@fragment
fn fs(in: VOut) -> FOut {
    var out: FOut;
    out.colour = shade(in);
    out.depth = standing_depth(in.world);
    return out;
}
"#;

const BOLT: &str = r#"
struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) along: f32,
    @location(1) across: f32,
    @location(2) part: f32,     // 0 shaft, 1 trail
    @location(3) alpha: f32,
}

@vertex
fn vs(
    @builtin(vertex_index) i: u32,
    @location(0) head: vec4<f32>,    // xyz, alpha
    @location(1) tail: vec4<f32>,    // xyz, width in pixels at 1080p
    @location(2) trail: vec4<f32>,   // xyz of the end of the faint trail
) -> VOut {
    var along = array<f32, 6>(0.0, 1.0, 1.0, 0.0, 1.0, 0.0);
    var side = array<f32, 6>(-1.0, -1.0, 1.0, -1.0, 1.0, 1.0);
    let part = f32(i / 6u);
    let k = i % 6u;
    // Shaft runs tail to head; the trail runs trail-end to tail.
    let from_point = select(tail.xyz, trail.xyz, part > 0.5);
    let to_point = select(head.xyz, tail.xyz, part > 0.5);
    let a = frame.view_proj * vec4<f32>(from_point, 1.0);
    let b = frame.view_proj * vec4<f32>(to_point, 1.0);
    let na = a.xy / a.w;
    let nb = b.xy / b.w;
    let size = frame.viewport.xy;
    let dir_px = (nb - na) * size * 0.5;
    let len = max(length(dir_px), 0.001);
    let dir = dir_px / len;
    let perp = vec2<f32>(-dir.y, dir.x);
    let scale = size.y / 1080.0;
    let width = tail.w * scale * select(1.0, 0.7, part > 0.5);
    let t = along[k];
    let centre = mix(na, nb, t);
    let w = select(a.w, b.w, t > 0.5);
    var out: VOut;
    out.pos = vec4<f32>((centre + perp * side[k] * width / size) * w, mix(a.z, b.z, t), w);
    out.along = t;
    out.across = side[k];
    out.part = part;
    out.alpha = head.w;
    return out;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let soft = 1.0 - smoothstep(0.55, 1.0, abs(in.across));
    if (in.part > 0.5) {
        let a = in.along * in.along * 0.35 * soft * in.alpha;
        return vec4<f32>(vec3<f32>(0.9, 0.9, 0.82) * a, a);
    }
    var colour = vec3<f32>(0.62, 0.45, 0.28);
    if (in.along > 0.93) {
        colour = vec3<f32>(0.92, 0.94, 0.98);
    }
    if (in.along < 0.16) {
        colour = vec3<f32>(0.92, 0.9, 0.82);
    }
    let a = soft * in.alpha;
    return vec4<f32>(colour * a, a);
}
"#;

pub fn figure() -> String {
    format!("{FRAME}{}{HELPERS}{FIGURE}", status_consts())
}

pub fn effects() -> String {
    format!("{FRAME}{}{HELPERS}{EFFECTS}", status_consts())
}

pub fn bolt() -> String {
    format!("{FRAME}{BOLT}")
}
