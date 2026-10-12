//! Impact particles and ambient motes: sparks off steel, dust off the ground, a few drifting
//! motes in the scene's own colours. Pure and time-based: a deterministic generator seeded by the
//! stage, simulated in real seconds, handed to the renderer as flat instance data.

use super::math::Vec3;

/// Most particles alive at once.
pub const MAX_PARTICLES: usize = 320;
/// Floats per instance handed to the renderer: x y z size, premultiplied r g b, additive share.
pub const INSTANCE_FLOATS: usize = 8;

/// Sparks: how many a blow throws at weight 1, their speed (m/s) and life (s).
pub const SPARKS_AT_FULL_WEIGHT: u32 = 26;
pub const SPARK_SPEED_MIN: f32 = 2.2;
pub const SPARK_SPEED_MAX: f32 = 6.2;
pub const SPARK_LIFE_S: (f32, f32) = (0.3, 0.62);
pub const SPARK_SIZE_M: (f32, f32) = (0.03, 0.06);
pub const SPARK_COLOUR: [f32; 3] = [1.0, 0.82, 0.46];
/// Dust from a fall: puff count, size growth and life.
pub const DUST_PUFFS: u32 = 9;
pub const DUST_SIZE_M: (f32, f32) = (0.18, 0.55);
pub const DUST_LIFE_S: (f32, f32) = (0.7, 1.15);
pub const DUST_ALPHA: f32 = 0.26;
pub const DUST_COLOUR: [f32; 3] = [0.58, 0.52, 0.42];
/// Ambient motes alive at once, their drift speed (m/s), size and life.
pub const MOTES_ALIVE: usize = 14;
pub const MOTE_SPEED: f32 = 0.16;
pub const MOTE_SIZE_M: (f32, f32) = (0.028, 0.05);
pub const MOTE_LIFE_S: (f32, f32) = (5.0, 9.0);
pub const MOTE_ALPHA: f32 = 0.55;
/// Gravity on sparks, metres per second squared.
const GRAVITY: f32 = 9.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    Spark,
    Dust,
    Mote,
}

#[derive(Clone, Copy, Debug)]
struct Particle {
    kind: Kind,
    position: Vec3,
    velocity: Vec3,
    age_s: f32,
    life_s: f32,
    size_m: f32,
    colour: [f32; 3],
}

/// A tiny deterministic generator (no randomness crate, same effects for the same blow).
#[derive(Clone, Debug)]
struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) as f32 / 16_777_216.0
    }

    fn between(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.next()
    }
}

#[derive(Clone, Debug)]
pub struct Particles {
    list: Vec<Particle>,
    random: Lcg,
    /// Where motes are kept alive around, and the palette they take.
    mote_centre: Vec3,
    mote_radius: f32,
    mote_colour: [f32; 3],
}

impl Default for Particles {
    fn default() -> Self {
        Self::new(0x9E37_79B9)
    }
}

impl Particles {
    pub fn new(seed: u32) -> Self {
        Self {
            list: Vec::new(),
            random: Lcg(seed),
            mote_centre: Vec3::ZERO,
            mote_radius: 0.0,
            mote_colour: [1.0, 0.8, 0.5],
        }
    }

    pub fn count(&self) -> usize {
        self.list.len()
    }

    pub fn count_of(&self, kind: Kind) -> usize {
        self.list
            .iter()
            .filter(|particle| particle.kind == kind)
            .count()
    }

    fn push(&mut self, particle: Particle) {
        if self.list.len() < MAX_PARTICLES {
            self.list.push(particle);
        }
    }

    /// Sparks thrown from `at`, away from the attacker (`away` is a flat unit direction), more and
    /// faster for a heavier blow (`weight` 0 to 1).
    pub fn sparks(&mut self, at: Vec3, away: Vec3, weight: f32) {
        let count = (SPARKS_AT_FULL_WEIGHT as f32 * weight.clamp(0.0, 1.0)).round() as u32;
        for _ in 0..count {
            let spread = Vec3::new(
                self.random.between(-0.8, 0.8),
                self.random.between(0.2, 1.1),
                self.random.between(-0.8, 0.8),
            );
            let speed =
                self.random.between(SPARK_SPEED_MIN, SPARK_SPEED_MAX) * (0.55 + 0.6 * weight);
            let velocity = (away * 1.1 + spread).normalized() * speed;
            let life_s = self.random.between(SPARK_LIFE_S.0, SPARK_LIFE_S.1);
            let size_m = self.random.between(SPARK_SIZE_M.0, SPARK_SIZE_M.1);
            self.push(Particle {
                kind: Kind::Spark,
                position: at,
                velocity,
                age_s: 0.0,
                life_s,
                size_m,
                colour: SPARK_COLOUR,
            });
        }
    }

    /// Dust puffs off the ground around `at` (a figure falling).
    pub fn dust(&mut self, at: Vec3) {
        for _ in 0..DUST_PUFFS {
            let angle = self.random.between(0.0, std::f32::consts::TAU);
            let reach = self.random.between(0.1, 0.55);
            let velocity = Vec3::new(
                angle.cos() * 0.35,
                self.random.between(0.12, 0.35),
                angle.sin() * 0.35,
            );
            let life_s = self.random.between(DUST_LIFE_S.0, DUST_LIFE_S.1);
            let size_m = self.random.between(DUST_SIZE_M.0, DUST_SIZE_M.1);
            self.push(Particle {
                kind: Kind::Dust,
                position: at + Vec3::new(angle.cos() * reach, 0.05, angle.sin() * reach),
                velocity,
                age_s: 0.0,
                life_s,
                size_m,
                colour: DUST_COLOUR,
            });
        }
    }

    /// Where the ambient motes drift, how far from the centre they roam and what colour they are.
    pub fn set_motes(&mut self, centre: Vec3, radius_m: f32, colour: [f32; 3]) {
        self.mote_centre = centre;
        self.mote_radius = radius_m.max(0.0);
        self.mote_colour = colour;
    }

    /// Advances by `dt_s` real seconds: moves, ages, retires, and tops up the motes.
    pub fn advance(&mut self, dt_s: f32) {
        let dt = dt_s.clamp(0.0, 0.25);
        for particle in &mut self.list {
            particle.age_s += dt;
            match particle.kind {
                Kind::Spark => {
                    particle.velocity.y -= GRAVITY * dt;
                    // Drag, so a spark's flight arcs over rather than flying straight.
                    let drag = (1.0 - 2.2 * dt).max(0.0);
                    particle.velocity = particle.velocity * drag;
                }
                Kind::Dust => {
                    particle.velocity = particle.velocity * (1.0 - 1.4 * dt).max(0.0);
                }
                Kind::Mote => {}
            }
            particle.position = particle.position + particle.velocity * dt;
        }
        self.list
            .retain(|particle| particle.age_s < particle.life_s);
        if self.mote_radius > 0.0 {
            while self.count_of(Kind::Mote) < MOTES_ALIVE && self.list.len() < MAX_PARTICLES {
                let angle = self.random.between(0.0, std::f32::consts::TAU);
                let reach = self.mote_radius * self.random.next().sqrt();
                let position = self.mote_centre
                    + Vec3::new(
                        angle.cos() * reach,
                        self.random.between(0.2, 2.4),
                        angle.sin() * reach,
                    );
                let velocity = Vec3::new(
                    self.random.between(-1.0, 1.0),
                    self.random.between(-0.2, 0.7),
                    self.random.between(-1.0, 1.0),
                ) * MOTE_SPEED;
                let life_s = self.random.between(MOTE_LIFE_S.0, MOTE_LIFE_S.1);
                let size_m = self.random.between(MOTE_SIZE_M.0, MOTE_SIZE_M.1);
                // Stagger the first motes through their lives so they do not all fade together.
                let age_s = self.random.between(0.0, life_s * 0.9);
                self.list.push(Particle {
                    kind: Kind::Mote,
                    position,
                    velocity,
                    age_s,
                    life_s,
                    size_m,
                    colour: self.mote_colour,
                });
            }
        }
    }

    /// Instance data for the renderer: [`INSTANCE_FLOATS`] floats per particle, sparks and motes
    /// adding light and dust blending over.
    pub fn instances(&self) -> Vec<f32> {
        let mut data = Vec::with_capacity(self.list.len() * INSTANCE_FLOATS);
        for particle in &self.list {
            let t = (particle.age_s / particle.life_s).clamp(0.0, 1.0);
            let (size, alpha, additive) = match particle.kind {
                Kind::Spark => (particle.size_m * (1.0 - 0.5 * t), 1.0 - t * t, 1.0),
                Kind::Dust => (
                    particle.size_m * (0.5 + 0.5 * t.sqrt()),
                    DUST_ALPHA * (1.0 - t) * (t * 6.0).min(1.0),
                    0.0,
                ),
                Kind::Mote => (
                    particle.size_m,
                    MOTE_ALPHA * (std::f32::consts::PI * t).sin(),
                    1.0,
                ),
            };
            data.extend_from_slice(&[
                particle.position.x,
                particle.position.y,
                particle.position.z,
                size,
                particle.colour[0] * alpha,
                particle.colour[1] * alpha,
                particle.colour[2] * alpha,
                additive,
            ]);
        }
        data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(particles: &mut Particles, seconds: f32) {
        for _ in 0..(seconds / 0.01).ceil() as u32 {
            particles.advance(0.01);
        }
    }

    #[test]
    fn a_heavier_blow_throws_more_sparks_and_a_graze_throws_few() {
        let mut light = Particles::new(1);
        light.sparks(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), 0.15);
        let mut heavy = Particles::new(1);
        heavy.sparks(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), 1.0);
        assert!(heavy.count_of(Kind::Spark) > light.count_of(Kind::Spark) * 3);
        assert_eq!(heavy.count_of(Kind::Spark), SPARKS_AT_FULL_WEIGHT as usize);
    }

    #[test]
    fn sparks_fly_away_fall_and_are_gone_within_their_life() {
        let mut particles = Particles::new(7);
        particles.sparks(Vec3::new(0.0, 1.0, 0.0), Vec3::new(1.0, 0.0, 0.0), 1.0);
        run(&mut particles, 0.25);
        let data = particles.instances();
        assert!(!data.is_empty());
        let mean_x: f32 = data.chunks(INSTANCE_FLOATS).map(|p| p[0]).sum::<f32>()
            / (data.len() / INSTANCE_FLOATS) as f32;
        assert!(mean_x > 0.1, "thrown away from the attacker: {mean_x}");
        run(&mut particles, SPARK_LIFE_S.1 + 0.1);
        assert_eq!(particles.count_of(Kind::Spark), 0);
    }

    #[test]
    fn dust_rises_slowly_and_fades_out() {
        let mut particles = Particles::new(3);
        particles.dust(Vec3::new(2.0, 0.0, 2.0));
        assert_eq!(particles.count_of(Kind::Dust), DUST_PUFFS as usize);
        run(&mut particles, DUST_LIFE_S.1 + 0.1);
        assert_eq!(particles.count_of(Kind::Dust), 0);
    }

    #[test]
    fn motes_are_kept_alive_in_the_scene_colour_and_never_exceed_the_cap() {
        let mut particles = Particles::new(5);
        particles.set_motes(Vec3::ZERO, 6.0, [1.0, 0.6, 0.3]);
        run(&mut particles, 30.0);
        assert_eq!(particles.count_of(Kind::Mote), MOTES_ALIVE);
        let data = particles.instances();
        assert!(
            data.chunks(INSTANCE_FLOATS).all(|p| p[4] >= p[6]),
            "warm: red at least blue"
        );
        for _ in 0..40 {
            particles.sparks(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), 1.0);
        }
        assert!(particles.count() <= MAX_PARTICLES);
        // Without a radius there are no motes.
        let mut none = Particles::new(5);
        run(&mut none, 5.0);
        assert_eq!(none.count(), 0);
    }

    #[test]
    fn the_same_blow_gives_the_same_sparks() {
        let make = || {
            let mut particles = Particles::new(11);
            particles.sparks(Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0), 0.7);
            run(&mut particles, 0.2);
            particles.instances()
        };
        assert_eq!(make(), make());
    }

    #[test]
    fn particles_move_by_seconds_not_frames() {
        let simulate = |frames_per_second: u32| {
            let mut particles = Particles::new(13);
            particles.sparks(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), 1.0);
            for _ in 0..(frames_per_second / 6) {
                particles.advance(1.0 / frames_per_second as f32);
            }
            let data = particles.instances();
            data.chunks(INSTANCE_FLOATS).map(|p| p[0]).sum::<f32>()
                / (data.len() / INSTANCE_FLOATS) as f32
        };
        // A sixth of a second at 24 and at 30 frames per second lands in nearly the same place
        // (the integrator differs by a hair; the clock does not).
        assert!((simulate(24) - simulate(30)).abs() < 0.05);
    }
}
