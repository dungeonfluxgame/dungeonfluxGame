//! WGSL for the battlefield passes. Colours are display-referred throughout (textures are plain
//! `rgba8unorm`, never sRGB-converted), so the post pass grades what the scene pass drew.

/// Shared by every scene shader: the per-frame uniform block.
pub(super) const FRAME: &str = r#"
struct Frame {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    eye: vec4<f32>,       // xyz eye, w seconds
    right: vec4<f32>,     // xyz camera right flattened to the ground, w aspect
    viewport: vec4<f32>,  // width px, height px, focal px, decal lift toward the camera (m)
    light: vec4<f32>,     // x, z of the lit centre, light radius, restrained content rating
    ground: vec4<f32>,    // centre x, centre z, half size, fall-off
    backdrop: vec4<f32>,  // wall z, half width, base y, height
    spot: vec4<f32>,      // warm pool under the acting figure: x, z, radius, strength
    fire_position: vec4<f32>, // xyz, radius metres
    fire_colour: vec4<f32>,   // rgb, strength
    plane: vec4<f32>,     // play-plane height, ground margin, unused x2
}
@group(0) @binding(0) var<uniform> frame: Frame;

// The depth of a point on the play plane moved toward the camera along the line of sight by
// `viewport.w` metres: where it is on screen does not change, only how near it counts for the
// depth test against the scan's own depth, so grass and rubble at the ground cannot swallow it.
fn lifted_depth(world: vec3<f32>) -> f32 {
    let moved = world + normalize(frame.eye.xyz - world) * frame.viewport.w;
    let clip = frame.view_proj * vec4<f32>(moved, 1.0);
    return clamp(clip.z / clip.w, 0.0, 1.0);
}

// Compact point-light tint: the same finite, inverse-square-like falloff on the scan and actors.
fn fire_spill(world: vec3<f32>) -> vec3<f32> {
    let delta = world - frame.fire_position.xyz;
    let radius = max(frame.fire_position.w, 0.001);
    let d2 = dot(delta, delta) / (radius * radius);
    let cutoff = max(1.0 - d2, 0.0);
    return frame.fire_colour.rgb * frame.fire_colour.w * cutoff * cutoff / (1.0 + d2 * 8.0);
}

struct FOut {
    @location(0) colour: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

// The depth a figure standing on the play plane is tested with. The scan's own ground (turf,
// stones, tufts) may rise up to `plane.y` metres above the plane, so the figure counts as being as
// near as the point where its line of sight crosses that height: ground at or below it never hides
// the feet, while anything standing higher in front of the figure (a trunk, a wall, a palisade)
// is still nearer than that point and still hides it. Above that height the figure's own depth
// stands, since the ground cannot reach it. With no margin (painted grounds) nothing changes.
fn standing_depth(world: vec3<f32>) -> f32 {
    let to_world = world - frame.eye.xyz;
    let direction = normalize(to_world);
    var along = length(to_world);
    if (direction.y < -0.0001) {
        along = min(along, (frame.eye.y - (frame.plane.x + frame.plane.y)) / -direction.y);
    }
    let clip = frame.view_proj * vec4<f32>(frame.eye.xyz + direction * along, 1.0);
    return clamp(clip.z / clip.w, 0.0, 1.0);
}
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
    colour = colour + fire_spill(vec3<f32>(in.world_xz.x, frame.plane.x, in.world_xz.y)) * (colour + vec3<f32>(0.12));
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

const GRID: &str = r#"
// The grid on the flat play plane. Data block (see grid_lines.rs): 32 header words, then one
// code per cell, row by row. 0 no line, 1 neutral, 2 enemy, 3 hero, 4 reachable, 5 target.
@group(1) @binding(0) var<storage, read> grid: array<f32>;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) xz: vec2<f32>,
    // 0 for the pass that draws where the line is in front of the scan, 1 for where it is behind.
    @location(1) @interpolate(flat) behind: f32,
}

@vertex
fn vs(@builtin(vertex_index) i: u32, @builtin(instance_index) pass_index: u32) -> VOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0));
    let c = corners[i];
    let origin = vec2<f32>(grid[0], grid[1]);
    let size = vec2<f32>(grid[3], grid[4]) * grid[2];
    let margin = grid[19];
    let xz = mix(origin - vec2<f32>(margin), origin + size + vec2<f32>(margin), c);
    let on_plane = vec3<f32>(xz.x, grid[5], xz.y);
    var out: VOut;
    out.pos = frame.view_proj * vec4<f32>(on_plane, 1.0);
    out.xz = xz;
    out.behind = f32(pass_index);
    return out;
}

fn cell_code(col: i32, row: i32) -> f32 {
    let cols = i32(grid[3]);
    let rows = i32(grid[4]);
    if (col < 0 || row < 0 || col >= cols || row >= rows) {
        return 0.0;
    }
    return grid[36 + row * cols + col];
}

fn code_colour(code: f32) -> vec3<f32> {
    // 1 neutral gold, 2 enemy red, 3 hero blue; on the outline of a reachable (4) or target (5)
    // region and inside one (6 and 7).
    if ((code > 4.5 && code < 5.5) || code > 6.5 || (code > 1.5 && code < 2.5)) {
        return vec3<f32>(grid[28], grid[29], grid[30]);
    }
    if (code > 2.5) {
        return vec3<f32>(grid[24], grid[25], grid[26]);
    }
    return vec3<f32>(grid[20], grid[21], grid[22]);
}

// Only the outline of a highlighted region is emphasised; the lines inside it (codes 6 and 7)
// keep the base grid's strength, tinted.
fn code_boost(code: f32) -> f32 {
    return select(1.0, grid[14], code > 3.5 && code < 5.5);
}

// The code of the line between two cells: the higher of the two, except that a line between two
// cells of the same highlighted region lies inside it.
fn line_code(a: f32, b: f32) -> f32 {
    let high = max(a, b);
    if (min(a, b) > 3.5 && abs(a - b) < 0.5) {
        return high + 2.0;
    }
    return high;
}

fn shade(in: VOut) -> vec4<f32> {
    let cell_m = grid[2];
    let p = (in.xz - vec2<f32>(grid[0], grid[1])) / cell_m;
    // Derivatives are taken once, before any branch.
    let per_px = max(fwidth(p), vec2<f32>(1e-5));
    let to_line = abs(fract(p + vec2<f32>(0.5)) - vec2<f32>(0.5));
    let line_px = to_line / per_px;
    let metres_px = per_px * cell_m;

    // Core: world width, never thinner than a pixel and a half; thinner lines fade, not shimmer.
    let width_px = vec2<f32>(grid[6]) / metres_px;
    let core_px = max(width_px, vec2<f32>(grid[7]));
    let core_dim = clamp(width_px / grid[7], vec2<f32>(0.0), vec2<f32>(1.0));
    let core = (vec2<f32>(1.0) - smoothstep(core_px * 0.5 - vec2<f32>(0.5), core_px * 0.5 + vec2<f32>(0.5), line_px)) * core_dim;
    // Halo: a Gaussian of the distance to the line, fading as it narrows past the minimum.
    let halo_w_px = vec2<f32>(grid[9]) / metres_px;
    let halo_px = max(halo_w_px, vec2<f32>(grid[10]));
    let halo_dim = clamp(halo_w_px / grid[10], vec2<f32>(0.0), vec2<f32>(1.0));
    let ratio = line_px / halo_px;
    let halo = exp(-2.0 * ratio * ratio) * halo_dim;
    // Lines crowding together (a distant or grazing view) fade out.
    let spacing = smoothstep(vec2<f32>(grid[15] * 0.5), vec2<f32>(grid[15]), vec2<f32>(1.0) / per_px);

    // Which cells each line separates: vertical lines (fixed x) and horizontal ones (fixed z).
    let line_x = i32(round(p.x));
    let line_z = i32(round(p.y));
    let row = i32(floor(p.y));
    let col = i32(floor(p.x));
    let code_x = line_code(cell_code(line_x - 1, row), cell_code(line_x, row));
    let code_z = line_code(cell_code(col, line_z - 1), cell_code(col, line_z));
    // A blocked cell (0.5) keeps its lines, fainter, so the lattice reads as one continuous grid.
    let faint = vec2<f32>(select(1.0, grid[31], code_x < 0.75), select(1.0, grid[31], code_z < 0.75));
    let alive = vec2<f32>(step(0.25, code_x), step(0.25, code_z)) * spacing * faint;
    let colour_x = code_colour(code_x);
    let colour_z = code_colour(code_z);
    let boost = vec2<f32>(code_boost(code_x), code_boost(code_z));

    // Dimmer toward the edge of the arena, like the lit pool the ground has.
    let from_centre = distance(in.xz, frame.light.xy);
    let centre_fade = mix(1.0, grid[18], smoothstep(grid[16], grid[17], from_centre));

    let core_a = core * alive * boost;
    let halo_a = halo * alive * boost;
    let core_tint = vec3<f32>(grid[23]);
    let core_rgb_x = mix(colour_x, vec3<f32>(1.0), core_tint) * core_a.x;
    let core_rgb_z = mix(colour_z, vec3<f32>(1.0), core_tint) * core_a.y;
    let core_alpha = clamp(max(core_a.x, core_a.y), 0.0, 1.0) * grid[8];
    let core_rgb = max(core_rgb_x, core_rgb_z) * grid[8];
    let glow_rgb = (colour_x * halo_a.x + colour_z * halo_a.y) * grid[11];
    let underlay = max(halo.x * alive.x, halo.y * alive.y) * grid[12];
    var alpha = clamp(core_alpha + underlay * (1.0 - core_alpha), 0.0, 1.0);
    var rgb = core_rgb + glow_rgb * (1.0 - core_alpha);
    // And dimmer toward the border of the play area; reachable and target lines keep most of
    // their strength so they stay clearly above the base grid.
    let edge_cells = min(min(p.x, grid[3] - p.x), min(p.y, grid[4] - p.y));
    let edge_fade = mix(grid[33], 1.0, smoothstep(0.0, grid[32], edge_cells));
    let lit_code = max(code_x, code_z);
    let lit = step(3.5, lit_code) * step(lit_code, 5.5);
    var strength = mix(centre_fade * edge_fade, mix(centre_fade * edge_fade, 1.0, 0.75), lit);
    if (in.behind > 0.5) {
        strength = strength * grid[13];
    }
    rgb = rgb * strength;
    alpha = alpha * strength;
    if (alpha < 0.002 && max(rgb.r, max(rgb.g, rgb.b)) < 0.002) {
        discard;
    }
    // Premultiplied: the colour adds, the alpha only darkens what is beneath (the underlay).
    return vec4<f32>(rgb, alpha);
}

// Only a scan surface well in front of the line (see `lifted_depth`) counts as hiding it: the
// front pass keeps the line where nothing is, the behind pass dims it where something is.
@fragment
fn fs(in: VOut) -> FOut {
    var out: FOut;
    out.colour = shade(in);
    out.depth = lifted_depth(vec3<f32>(in.xz.x, grid[5], in.xz.y));
    return out;
}
"#;

const DECAL: &str = r#"
struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) c: vec2<f32>,
    @location(1) col: vec4<f32>,
    @location(2) params: vec4<f32>,
    @location(3) world: vec3<f32>,
}

@vertex
fn vs(
    @builtin(vertex_index) i: u32,
    @location(0) shape: vec4<f32>,   // centre x, centre z, half x, half z
    @location(1) col: vec4<f32>,
    @location(2) params: vec4<f32>,  // kind, glow, active, local ground height
) -> VOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
    let c = corners[i];
    let x = shape.x + c.x * shape.z;
    let z = shape.y + c.y * shape.w;
    var out: VOut;
    // Each marker uses its own surveyed ground height; depth is lifted per pixel in `fs`.
    let on_ground = vec3<f32>(x, params.w + 0.012, z);
    out.pos = frame.view_proj * vec4<f32>(on_ground, 1.0);
    out.world = on_ground;
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

fn shade(in: VOut) -> vec4<f32> {
    let kind = i32(in.params.x + 0.5);
    let pulse = 0.5 + 0.5 * sin(frame.eye.w * 4.0);
    let edge = square_edge(in.c);
    // Derivatives are taken once, outside the branches, to stay in uniform control flow.
    let aa = max(fwidth(edge), 0.002);
    let aa_inner = max(fwidth(square_edge(in.c / 0.74)), 0.002);
    if (kind == 1) {
        // The reachable region: a faint wash over each cell. The grid pass draws one thin outline
        // around the region's edge; nothing here draws a border or hides the ground.
        let a = (0.05 + 0.02 * pulse) * in.col.a;
        return vec4<f32>(in.col.rgb * a, a * 0.5);
    }
    if (kind == 2) {
        // A legal target: a thin ring inside the cell and a slightly stronger wash. With the
        // figure's own ring, the only strong marks on the field.
        let ring = line(square_edge(in.c / 0.86), aa, 0.045) * step(0.0, square_edge(in.c / 0.86));
        let a = max(ring * 0.8, 0.09 + 0.03 * pulse) * in.col.a;
        return vec4<f32>(in.col.rgb * a, a * 0.7);
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
        let ang = frame.eye.w * 0.9;
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
        let ang = atan2(in.c.y, in.c.x) + frame.eye.w * 0.8;
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
    if (kind == 12) {
        // Whose side a standing figure is on: a thin ring on the ground around its feet, with a
        // faint wash inside so a small creature on busy ground can be found at a glance.
        let d = length(in.c);
        let ring = 1.0 - smoothstep(0.0, 0.09, abs(d - 0.84));
        let wash = (1.0 - smoothstep(0.2, 0.84, d)) * 0.16;
        let a = max(ring, wash) * in.col.a;
        return vec4<f32>(in.col.rgb * a, a);
    }
    // Markers under figures: a warm glow disc, an inner outlined tile and, when acting, a pulse.
    let d = length(in.c);
    let glow = exp(-d * d * 2.6) * (0.42 + 0.5 * in.params.z * (0.7 + 0.3 * pulse));
    let inner = square_edge(in.c / 0.74);
    let outline = line(inner, aa_inner, 0.035) * step(0.0, inner);
    let warm = vec3<f32>(1.0, 0.72, 0.36);
    let rgb = warm * glow * 0.22 + in.col.rgb * outline * (0.6 + 0.3 * in.params.z);
    let a = outline * in.col.a * 0.9;
    return vec4<f32>(rgb, a);
}

@fragment
fn fs(in: VOut) -> FOut {
    var out: FOut;
    out.colour = shade(in);
    out.depth = lifted_depth(in.world);
    return out;
}
"#;

const PARTICLE: &str = r#"
struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) p: vec2<f32>,
    @location(1) colour: vec4<f32>,
}

@vertex
fn vs(
    @builtin(vertex_index) i: u32,
    @location(0) place: vec4<f32>,   // x, y, z, size in metres
    @location(1) tint: vec4<f32>,    // premultiplied rgb, additive share (1 adds light, 0 blends over)
) -> VOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
    let c = corners[i];
    // A small camera-facing quad: right is the camera's right, up is the world's up.
    let world = place.xyz + frame.right.xyz * (c.x * place.w) + vec3<f32>(0.0, c.y * place.w, 0.0);
    var out: VOut;
    out.pos = frame.view_proj * vec4<f32>(world, 1.0);
    out.p = c;
    out.colour = tint;
    return out;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let d2 = dot(in.p, in.p);
    if (d2 > 1.0) {
        discard;
    }
    let soft = exp(-d2 * 3.2);
    // Premultiplied: additive particles carry no alpha (they only add light); dust blends over.
    let add = in.colour.a;
    let rgb = in.colour.rgb * soft;
    let alpha = (1.0 - add) * soft * max(max(in.colour.r, in.colour.g), in.colour.b) * 1.6;
    return vec4<f32>(rgb, alpha);
}
"#;

const POST: &str = r#"
struct Post {
    grade: vec4<f32>,      // exposure, contrast, saturation, enabled
    shadow: vec4<f32>,
    highlight: vec4<f32>,
    tilt: vec4<f32>,       // centre, band, fall-off, blur px
    misc: vec4<f32>,       // vignette, flash strength, unused, tilt-shift on
    flash: vec4<f32>,      // flash colour rgb
    shake: vec4<f32>,      // dx px, dy px, texel w, texel h
    motion: vec4<f32>,     // motion blur direction x, y (unit), blur px, radial punch
    split: vec4<f32>,      // chromatic split px, picture roll (radians), vignette pulse, unused
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
    for (var k = 1; k <= 18; k = k + 1) {
        let t = f32(k) / 18.0;
        let angle = f32(k) * 2.39996;
        let o = vec2<f32>(cos(angle), sin(angle)) * sqrt(t) * radius * post.shake.zw;
        sum = sum + fetch(uv + o);
        weight = weight + 1.0;
    }
    return sum / weight;
}

// The picture rolled about its centre (aspect-correct) by `post.split.y` radians.
fn rolled(uv: vec2<f32>) -> vec2<f32> {
    let aspect = post.shake.w / post.shake.z;
    let c = (uv - vec2<f32>(0.5)) * vec2<f32>(aspect, 1.0);
    let s = sin(post.split.y);
    let k = cos(post.split.y);
    let r = vec2<f32>(c.x * k - c.y * s, c.x * s + c.y * k);
    return r / vec2<f32>(aspect, 1.0) + vec2<f32>(0.5);
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let uv = rolled(in.uv) + post.shake.xy * post.shake.zw;
    var colour = blurred(uv);
    // Directional blur along a fast camera move, and a radial punch-in blur on a critical: a few
    // taps of the already tilt-shifted picture along the direction, averaged with it.
    let blur_px = post.motion.z;
    let radial = post.motion.w;
    if (blur_px > 0.5 || radial > 0.002) {
        var sum = colour;
        var weight = 1.0;
        let toward_centre = (vec2<f32>(0.5) - uv);
        for (var k = 1; k <= 6; k = k + 1) {
            let t = f32(k) / 6.0;
            let along = post.motion.xy * post.shake.zw * blur_px * (t - 0.5) * 2.0;
            let punch = toward_centre * radial * t;
            sum = sum + fetch(uv + along + punch);
            weight = weight + 1.0;
        }
        colour = sum / weight;
    }
    // Chromatic split on heavy impacts: red and blue slide apart toward the screen edge.
    if (post.split.x > 0.05) {
        let edge = (uv - vec2<f32>(0.5));
        let offset = edge * post.split.x * 2.0 * post.shake.zw;
        colour = vec3<f32>(fetch(uv + offset).r, colour.g, fetch(uv - offset).b);
    }
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
    colour = colour * (1.0 - vignette * (post.misc.x + post.split.z));
    // The pulse when a hero is hit: the vignette flushes red for a moment.
    colour = colour + vec3<f32>(0.55, 0.03, 0.03) * vignette * post.split.z;
    colour = colour + post.flash.rgb * post.misc.y * 0.55;
    return vec4<f32>(clamp(colour, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
"#;

pub fn backdrop() -> String {
    format!("{FRAME}{BACKDROP}")
}

pub fn ground() -> String {
    format!("{FRAME}{GROUND}")
}

pub fn grid() -> String {
    format!("{FRAME}{GRID}")
}

pub fn decal() -> String {
    format!("{FRAME}{DECAL}")
}

pub fn particle() -> String {
    format!("{FRAME}{PARTICLE}")
}

pub fn post() -> String {
    POST.to_owned()
}

/// Asset-free soft volumes: one bounded instanced draw, no textures or extra scene copy.
pub(super) fn technique() -> String {
    format!("{FRAME}{TECHNIQUE}")
}

const TECHNIQUE: &str = r#"
struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) p: vec2<f32>,
    @location(1) centre: vec4<f32>,
    @location(2) look: vec4<f32>,
}
@vertex
fn vs(@builtin(vertex_index) i: u32, @location(0) place: vec4<f32>,
    @location(1) look: vec4<f32>) -> VOut {
    var corners = array<vec2<f32>, 6>(vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
    let p = corners[i];
    let right = vec3<f32>(frame.view[0].x, frame.view[1].x, frame.view[2].x);
    let up = vec3<f32>(frame.view[0].y, frame.view[1].y, frame.view[2].y);
    var out: VOut;
    out.pos = frame.view_proj * vec4<f32>(place.xyz + (right * p.x + up * p.y) * place.w, 1.0);
    out.p = p;
    out.centre = place;
    out.look = look;
    return out;
}
fn noise(p: vec3<f32>) -> f32 {
    return 0.5 + 0.5 * sin(p.x * 5.1 + sin(p.y * 4.7)) * sin(p.z * 5.3 + p.y * 3.2);
}
@fragment
fn fs(in: VOut) -> FOut {
    let d2 = dot(in.p, in.p);
    if (d2 > 1.0) { discard; }
    let cap = sqrt(max(1.0 - d2, 0.0));
    let right = vec3<f32>(frame.view[0].x, frame.view[1].x, frame.view[2].x);
    let up = vec3<f32>(frame.view[0].y, frame.view[1].y, frame.view[2].y);
    let forward = normalize(frame.eye.xyz - in.centre.xyz);
    let surface = in.centre.xyz + (right * in.p.x + up * in.p.y + forward * cap) * in.centre.w;
    let age = in.look.y;
    let kind = in.look.x;
    let opacity = in.look.z;
    var rgb = vec3<f32>(0.0);
    var alpha = 0.0;
    if (kind == 5.0) {
        // A filmed glass ward: quiet interior and a brief pale Fresnel rim, no glyphs or neon disc.
        let rim = pow(1.0 - cap, 2.5) * (1.0 - smoothstep(0.86, 1.0, sqrt(d2)));
        alpha = (rim * 0.7 + cap * 0.035) * opacity;
        rgb = vec3<f32>(0.65, 0.8, 0.95) * alpha;
    } else if (kind == 6.0) {
        // Action Surge is breath and stirred dust, never a magical aura on the fighter.
        alpha = (1.0 - smoothstep(0.2, 1.0, d2)) * 0.15 * opacity;
        rgb = vec3<f32>(0.6, 0.49, 0.32) * alpha;
    } else if (kind == 1.0) {
        alpha = exp(-d2 * 4.5) * opacity * 0.65;
        rgb = vec3<f32>(0.7, 0.8, 1.0) * alpha * 1.3;
    } else {
        // Low-frequency rolling fire, white-hot centre, orange envelope, a dark smoke tail.
        let flow = vec3<f32>(in.p * 2.0, cap * 2.0) - vec3<f32>(0.0, age * 1.4, age * 0.4);
        let rolling = noise(flow) * 0.65 + noise(flow * 2.1) * 0.35;
        let edge = 1.0 - smoothstep(0.55 + rolling * 0.3, 1.0, sqrt(d2));
        let heat = clamp(cap * (0.5 + rolling * 0.5) - age * 0.22, 0.0, 1.0);
        let fire = mix(vec3<f32>(0.35, 0.065, 0.009), vec3<f32>(1.6, 0.7, 0.14), heat);
        rgb = mix(fire, vec3<f32>(1.7, 1.35, 0.8), pow(heat, 7.0));
        alpha = edge * opacity * mix(0.35, 0.72, heat);
        rgb = rgb * alpha;
    }
    var out: FOut;
    out.colour = vec4<f32>(rgb, alpha);
    let clip = frame.view_proj * vec4<f32>(surface, 1.0);
    out.depth = clamp(clip.z / clip.w, 0.0, 1.0);
    return out;
}
"#;
