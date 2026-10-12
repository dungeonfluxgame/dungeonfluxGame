//! Pure rules of the adaptive score: which tempo a display view implies, how that tempo mixes
//! the layers, which ambient bed fits a scene and which one-shot stinger a story change earns.
//!
//! Nothing here touches the browser, so the rules are checked natively. The Web Audio player
//! (`gameplay_browser::live_music`) only applies what this module decides. The music follows
//! what the server already shows; it never decides game state.
// Native builds compile this module only to run its tests; the player that uses all of it is
// browser-only, so the native build sees part of it as unused.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use df_protocol::common as rpc;

/// Tempo, key and loop length shared by every `score-*` layer and every stinger in
/// `assets/music`. Layers are cut from the same bar grid, so they stack sample-aligned.
pub(crate) const BPM: f64 = 84.0;
pub(crate) const BEAT_SECONDS: f64 = 60.0 / BPM;
/// 16 bars of 4/4: exactly 2 016 000 samples at 44.1 kHz (45.714 s).
pub(crate) const LOOP_SECONDS: f64 = 64.0 * BEAT_SECONDS;

/// How long a Release keeps the resolving motif before the mix settles to the calm bed.
/// Replaced by the server cue's `hold_millis` once the tempo engine supplies one.
pub(crate) const RELEASE_HOLD_MILLIS: f64 = 14_000.0;
/// Narration ducking: depth and the attack/release of the ramp.
pub(crate) const DUCK_DECIBELS: f32 = -9.0;
pub(crate) const DUCK_ATTACK_SECONDS: f64 = 0.25;
pub(crate) const DUCK_RELEASE_SECONDS: f64 = 0.6;

/// Moment-to-moment phase of the story, in the order the tempo engine describes them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Phase {
    Calm,
    Build,
    Anticipation,
    Peak,
    Release,
}

impl Phase {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Phase::Calm => "calm",
            Phase::Build => "build",
            Phase::Anticipation => "anticipation",
            Phase::Peak => "peak",
            Phase::Release => "release",
        }
    }
}

/// A continuous intensity (0..=1000) with its phase and the view sequence it came from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Tempo {
    pub(crate) phase: Phase,
    pub(crate) intensity: u16,
    pub(crate) sequence: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Outcome {
    Victory,
    Defeat,
}

/// Where the story is, as far as stingers and the ambient bed care.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Facts {
    pub(crate) in_combat: bool,
    pub(crate) antagonist_speaking: bool,
    pub(crate) outcome: Option<Outcome>,
    pub(crate) bed: Bed,
}

/// Location family of the ambient bed. Beds carry no melody, so every place shares one score.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Bed {
    Harbour,
    Market,
    Forest,
    Mountain,
    Library,
    Marsh,
    Graveyard,
    Farm,
    Night,
}

impl Bed {
    /// The served track name, `/assets/music/<track>`.
    pub(crate) fn track(self) -> &'static str {
        match self {
            Bed::Harbour => "amb-harbour",
            Bed::Market => "amb-market",
            Bed::Forest => "amb-forest",
            Bed::Mountain => "amb-mountain",
            Bed::Library => "amb-library",
            Bed::Marsh => "amb-marsh",
            Bed::Graveyard => "amb-graveyard",
            Bed::Farm => "amb-farm",
            Bed::Night => "amb-night",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Stinger {
    Reveal,
    Combat,
    Victory,
    Defeat,
}

impl Stinger {
    pub(crate) const ALL: [Stinger; 4] = [
        Stinger::Reveal,
        Stinger::Combat,
        Stinger::Victory,
        Stinger::Defeat,
    ];

    pub(crate) fn track(self) -> &'static str {
        match self {
            Stinger::Reveal => "sting-reveal",
            Stinger::Combat => "sting-combat",
            Stinger::Victory => "sting-victory",
            Stinger::Defeat => "sting-defeat",
        }
    }
}

/// The four sample-aligned layers, in the order the player starts them.
pub(crate) const LAYER_TRACKS: [&str; 4] =
    ["score-bed", "score-motif", "score-tension", "score-combat"];

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LayerGains {
    pub(crate) bed: f32,
    pub(crate) motif: f32,
    pub(crate) tension: f32,
    pub(crate) combat: f32,
}

impl LayerGains {
    pub(crate) fn in_track_order(self) -> [f32; 4] {
        [self.bed, self.motif, self.tension, self.combat]
    }
}

/// What one phase asks of the mixer. Gains are linear; the low-pass opens as the story heats up.
/// `rise_seconds` and `fall_seconds` are the smoothing of a layer getting louder or quieter, so
/// a Release can drop percussion quickly while the rest settles slowly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Mix {
    pub(crate) layers: LayerGains,
    pub(crate) ambient: f32,
    pub(crate) filter_hertz: f32,
    pub(crate) rise_seconds: f64,
    pub(crate) fall_seconds: f64,
}

const fn mix(
    gains: [f32; 4],
    ambient: f32,
    filter_hertz: f32,
    rise_seconds: f64,
    fall_seconds: f64,
) -> Mix {
    Mix {
        layers: LayerGains {
            bed: gains[0],
            motif: gains[1],
            tension: gains[2],
            combat: gains[3],
        },
        ambient,
        filter_hertz,
        rise_seconds,
        fall_seconds,
    }
}

/// The tuning table: layer thresholds and gains per phase, `[bed, motif, tension, combat]`.
/// Kept as plain data so a tuning file can replace it later without touching the player.
/// Inertia is built in: every ramp is 1.5 to 4 seconds except the deliberate 0.8 second drop
/// of percussion and tension when a Release begins.
pub(crate) const MIX_TABLE: [(Phase, Mix); 5] = [
    // Calm: the bed alone, a little muffled, with the location's ambience forward.
    (
        Phase::Calm,
        mix([0.90, 0.0, 0.0, 0.0], 1.00, 1_400.0, 4.0, 4.0),
    ),
    // Build: the melodic motif enters over the bed.
    (
        Phase::Build,
        mix([0.90, 0.75, 0.0, 0.0], 0.85, 3_200.0, 3.0, 4.0),
    ),
    // Anticipation: the low pulse and tremolo strings join; the filter opens.
    (
        Phase::Anticipation,
        mix([0.80, 0.50, 0.80, 0.0], 0.60, 6_000.0, 2.5, 3.0),
    ),
    // Peak: percussion and brass at full over a quieter bed, filter wide open.
    (
        Phase::Peak,
        mix([0.70, 0.35, 0.55, 1.00], 0.35, 18_000.0, 1.5, 2.0),
    ),
    // Release: percussion and tension fall away in 0.8 s, the motif resolves.
    (
        Phase::Release,
        mix([0.90, 0.80, 0.0, 0.0], 0.85, 4_000.0, 3.0, 0.8),
    ),
];

/// A defeat is sombre: the motif stays low and the sound muffled.
const SOMBRE_MOTIF_SCALE: f32 = 0.45;
const SOMBRE_FILTER_HERTZ: f32 = 1_800.0;
/// Within Anticipation and Peak the defining layer scales with intensity between these floors
/// and 1000, never below this fraction of its table gain.
const MINIMUM_EMPHASIS: f32 = 0.7;
const ANTICIPATION_FLOOR: u16 = 450;
const PEAK_FLOOR: u16 = 750;

pub(crate) fn mix_for(phase: Phase, intensity: u16, sombre: bool) -> Mix {
    let mut mix = MIX_TABLE
        .iter()
        .find(|(candidate, _)| *candidate == phase)
        .map_or(MIX_TABLE[0].1, |(_, mix)| *mix);
    match phase {
        Phase::Anticipation => {
            mix.layers.tension *= emphasis(intensity, ANTICIPATION_FLOOR);
        }
        Phase::Peak => {
            mix.layers.combat *= emphasis(intensity, PEAK_FLOOR);
        }
        _ => {}
    }
    if sombre {
        mix.layers.motif *= SOMBRE_MOTIF_SCALE;
        mix.filter_hertz = mix.filter_hertz.min(SOMBRE_FILTER_HERTZ);
    }
    mix
}

fn emphasis(intensity: u16, floor: u16) -> f32 {
    let span = f32::from(1_000 - floor);
    let above = f32::from(intensity.clamp(floor, 1_000) - floor);
    MINIMUM_EMPHASIS + (1.0 - MINIMUM_EMPHASIS) * above / span
}

/// The next beat of the layer grid that is still far enough ahead to schedule. The layers
/// started together at `started_at`, so a one-shot placed on a beat lands in time with them.
/// Before the layers run there is no grid and the one-shot plays at once.
pub(crate) fn next_beat(now: f64, started_at: Option<f64>) -> f64 {
    const MINIMUM_LEAD_SECONDS: f64 = 0.02;
    let Some(start) = started_at.filter(|start| *start <= now) else {
        return now;
    };
    let beat = start + ((now - start) / BEAT_SECONDS).ceil() * BEAT_SECONDS;
    if beat - now < MINIMUM_LEAD_SECONDS {
        beat + BEAT_SECONDS
    } else {
        beat
    }
}

/// A Release settles to the calm bed once its hold has passed.
pub(crate) fn effective_phase(tempo: Tempo, release_held_millis: f64) -> Phase {
    if tempo.phase == Phase::Release && release_held_millis >= RELEASE_HOLD_MILLIS {
        Phase::Calm
    } else {
        tempo.phase
    }
}

/// THE SINGLE PLACE THE TEMPO COMES FROM. Today it is derived from what the display view
/// already carries. When the server sends its `TempoCue { intensity_permille, phase, sequence,
/// hold_millis }`, replace this function body to return that cue (and feed `hold_millis` to
/// `RELEASE_HOLD_MILLIS`); nothing else in the music code needs to change.
pub(crate) fn tempo_of(
    scene: &rpc::GameplayScene,
    journey: &rpc::JourneyView,
    sequence: u64,
) -> Tempo {
    let facts = facts_of(scene, journey, "");
    let (phase, intensity) = match rpc::JourneyPhase::try_from(journey.phase) {
        Ok(rpc::JourneyPhase::Opening) => (Phase::Build, 350),
        Ok(rpc::JourneyPhase::Dialogue) if facts.antagonist_speaking => (Phase::Anticipation, 700),
        Ok(rpc::JourneyPhase::Dialogue) => (Phase::Anticipation, 550),
        Ok(rpc::JourneyPhase::Combat) => (Phase::Peak, combat_intensity(journey)),
        Ok(rpc::JourneyPhase::Complete) => complete_tempo(scene, journey, facts.outcome),
        _ => (Phase::Calm, 150),
    };
    Tempo {
        phase,
        intensity,
        sequence,
    }
}

/// 750 to 1000: starts high and climbs as the most hurt hero's health fraction falls and as
/// the rounds go on.
fn combat_intensity(journey: &rpc::JourneyView) -> u16 {
    let lowest_permille = journey
        .party
        .iter()
        .filter(|hero| hero.maximum_hit_points > 0)
        .map(|hero| {
            u64::from(hero.hit_points.min(hero.maximum_hit_points)) * 1_000
                / u64::from(hero.maximum_hit_points)
        })
        .min()
        .unwrap_or(1_000);
    let rounds = journey
        .combat
        .as_ref()
        .map_or(0, |combat| combat.round.min(5));
    let wounded = 1_000_u64.saturating_sub(lowest_permille) / 5;
    let intensity = 750 + wounded + u64::from(rounds) * 10;
    u16::try_from(intensity.min(1_000)).unwrap_or(1_000)
}

fn complete_tempo(
    scene: &rpc::GameplayScene,
    journey: &rpc::JourneyView,
    outcome: Option<Outcome>,
) -> (Phase, u16) {
    if scene.destination == rpc::HarborDestination::HarborInn as i32 {
        return (Phase::Calm, 200);
    }
    match outcome {
        Some(Outcome::Victory) => (Phase::Release, 450),
        Some(Outcome::Defeat) => (Phase::Release, 200),
        None if journey.combat.is_some() => (Phase::Release, 350),
        None => (Phase::Release, 300),
    }
}

/// Facts for stingers, the sombre defeat mix and the ambient bed. `display_asset` is the
/// display view's own scene asset, used when the scene carries none.
pub(crate) fn facts_of(
    scene: &rpc::GameplayScene,
    journey: &rpc::JourneyView,
    display_asset: &str,
) -> Facts {
    let phase = rpc::JourneyPhase::try_from(journey.phase).ok();
    let finished = journey
        .combat
        .as_ref()
        .is_some_and(|combat| combat.finished);
    let outcome = (phase == Some(rpc::JourneyPhase::Complete)
        && finished
        && scene.destination != rpc::HarborDestination::HarborInn as i32)
        .then(|| {
            if journey
                .party
                .iter()
                .any(|hero| hero.hit_points > 0 && !hero.unconscious)
            {
                Outcome::Victory
            } else {
                Outcome::Defeat
            }
        });
    let asset = if scene.scene_asset.is_empty() {
        display_asset
    } else {
        scene.scene_asset.as_str()
    };
    Facts {
        in_combat: phase == Some(rpc::JourneyPhase::Combat),
        antagonist_speaking: phase == Some(rpc::JourneyPhase::Dialogue)
            && (scene.speaker_portrait.contains("portrait-ant-")
                || scene.speaker_portrait.contains("bandit")
                || journey.speaker == "Bandit"),
        outcome,
        bed: bed_of(asset),
    }
}

/// The ambient family named by a server-provided scene asset path; unknown scenes get the
/// neutral night bed.
pub(crate) fn bed_of(asset: &str) -> Bed {
    let name = asset.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|word| name.contains(word));
    if has(&["market"]) {
        Bed::Market
    } else if has(&["pinewood", "forest", "trail"]) {
        Bed::Forest
    } else if has(&["windward", "mountain"]) {
        Bed::Mountain
    } else if has(&["library", "orrery"]) {
        Bed::Library
    } else if has(&["reedmoor", "marsh", "boardwalk"]) {
        Bed::Marsh
    } else if has(&["rookhill", "graveyard"]) {
        Bed::Graveyard
    } else if has(&["thistledown", "farm"]) {
        Bed::Farm
    } else if has(&["quay", "wharf", "harbor", "harbour", "dock"]) {
        Bed::Harbour
    } else {
        Bed::Night
    }
}

/// The stinger a change between two accepted views earns, if any. One stinger per change:
/// an outcome outranks a battle start, which outranks an antagonist reveal.
pub(crate) fn stinger_between(previous: &Facts, next: &Facts) -> Option<Stinger> {
    if next.outcome != previous.outcome {
        match next.outcome {
            Some(Outcome::Victory) => return Some(Stinger::Victory),
            Some(Outcome::Defeat) => return Some(Stinger::Defeat),
            None => {}
        }
    }
    if !previous.in_combat && next.in_combat {
        return Some(Stinger::Combat);
    }
    (!previous.antagonist_speaking && next.antagonist_speaking).then_some(Stinger::Reveal)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hero(hit_points: u32, maximum: u32) -> rpc::PartyMemberView {
        rpc::PartyMemberView {
            hit_points,
            maximum_hit_points: maximum,
            unconscious: hit_points == 0,
            ..Default::default()
        }
    }

    fn journey(phase: rpc::JourneyPhase) -> rpc::JourneyView {
        rpc::JourneyView {
            phase: phase as i32,
            party: vec![hero(20, 20), hero(20, 20)],
            ..Default::default()
        }
    }

    fn combat(round: u32, finished: bool) -> rpc::CombatView {
        rpc::CombatView {
            round,
            finished,
            ..Default::default()
        }
    }

    fn scene(asset: &str) -> rpc::GameplayScene {
        rpc::GameplayScene {
            scene_asset: asset.to_owned(),
            ..Default::default()
        }
    }

    fn tempo(scene: &rpc::GameplayScene, journey: &rpc::JourneyView) -> Tempo {
        tempo_of(scene, journey, 7)
    }

    #[test]
    fn score_loop_is_exactly_sixteen_bars_of_sample_aligned_layers() {
        assert_eq!((LOOP_SECONDS * 44_100.0).round() as u64, 2_016_000);
        assert!((LOOP_SECONDS - 45.714_285).abs() < 1e-5);
    }

    #[test]
    fn story_phases_map_to_rising_then_releasing_tempo() {
        let quiet = scene("");
        let lobby = tempo(&quiet, &journey(rpc::JourneyPhase::Room));
        let opening = tempo(&quiet, &journey(rpc::JourneyPhase::Opening));
        let dialogue = tempo(&quiet, &journey(rpc::JourneyPhase::Dialogue));
        let mut fighting = journey(rpc::JourneyPhase::Combat);
        fighting.combat = Some(combat(1, false));
        let battle = tempo(&quiet, &fighting);
        let mut won = journey(rpc::JourneyPhase::Complete);
        won.combat = Some(combat(3, true));
        let victory = tempo(&quiet, &won);
        let mut inn = scene("");
        inn.destination = rpc::HarborDestination::HarborInn as i32;
        let destination = tempo(&inn, &won);

        let phases = [lobby, opening, dialogue, battle, victory, destination].map(|t| t.phase);
        assert_eq!(
            phases,
            [
                Phase::Calm,
                Phase::Build,
                Phase::Anticipation,
                Phase::Peak,
                Phase::Release,
                Phase::Calm
            ]
        );
        assert_eq!(lobby.sequence, 7);
        assert!(lobby.intensity < opening.intensity && opening.intensity < dialogue.intensity);
        assert!(dialogue.intensity < battle.intensity && battle.intensity > victory.intensity);
        assert!(destination.intensity < victory.intensity);
    }

    #[test]
    fn combat_climbs_as_a_hero_is_hurt_and_as_rounds_pass() {
        let quiet = scene("");
        let mut fresh = journey(rpc::JourneyPhase::Combat);
        fresh.combat = Some(combat(1, false));
        let mut hurt = fresh.clone();
        hurt.party = vec![hero(20, 20), hero(4, 20)];
        let mut late = hurt.clone();
        late.combat = Some(combat(9, false));

        let first = tempo(&quiet, &fresh).intensity;
        let wounded = tempo(&quiet, &hurt).intensity;
        let long_fight = tempo(&quiet, &late).intensity;
        assert!((750..=1_000).contains(&first));
        assert!(first < wounded && wounded < long_fight && long_fight <= 1_000);
    }

    #[test]
    fn an_antagonist_speaker_raises_dialogue_and_is_a_reveal() {
        let mut bandit = scene("");
        bandit.speaker_portrait = "/assets/ui/generated/portrait-ant-bandit".to_owned();
        let talk = journey(rpc::JourneyPhase::Dialogue);
        let friendly = tempo(&scene(""), &talk);
        let hostile = tempo(&bandit, &talk);
        assert_eq!(hostile.phase, Phase::Anticipation);
        assert!(hostile.intensity > friendly.intensity);
        let before = facts_of(&scene(""), &talk, "");
        let after = facts_of(&bandit, &talk, "");
        assert_eq!(stinger_between(&before, &after), Some(Stinger::Reveal));
        assert_eq!(stinger_between(&after, &after), None);
    }

    #[test]
    fn victory_defeat_and_battle_start_each_earn_one_stinger() {
        let quiet = scene("");
        let calm = facts_of(&quiet, &journey(rpc::JourneyPhase::Room), "");
        let mut fighting = journey(rpc::JourneyPhase::Combat);
        fighting.combat = Some(combat(1, false));
        let battle = facts_of(&quiet, &fighting, "");
        let mut won = journey(rpc::JourneyPhase::Complete);
        won.combat = Some(combat(2, true));
        let victory = facts_of(&quiet, &won, "");
        let mut lost = won.clone();
        lost.party = vec![hero(0, 20), hero(0, 20)];
        let defeat = facts_of(&quiet, &lost, "");

        assert_eq!(stinger_between(&calm, &battle), Some(Stinger::Combat));
        assert_eq!(stinger_between(&battle, &victory), Some(Stinger::Victory));
        assert_eq!(stinger_between(&battle, &defeat), Some(Stinger::Defeat));
        assert_eq!(stinger_between(&victory, &victory), None);
        assert_eq!(tempo(&quiet, &lost).phase, Phase::Release);
        assert!(tempo(&quiet, &lost).intensity < tempo(&quiet, &won).intensity);
    }

    #[test]
    fn location_family_comes_from_the_scene_asset_name() {
        let cases = [
            ("/assets/ui/generated/scene-salt-quay-arrival", Bed::Harbour),
            (
                "/assets/ui/generated/scene-lamplit-market-interior",
                Bed::Market,
            ),
            (
                "/assets/ui/generated/scene-pinewood-trail-arrival",
                Bed::Forest,
            ),
            (
                "/assets/ui/generated/scene-windward-pass-arrival",
                Bed::Mountain,
            ),
            (
                "/assets/ui/generated/scene-orrery-library-interior",
                Bed::Library,
            ),
            (
                "/assets/ui/generated/scene-reedmoor-boardwalk-arrival",
                Bed::Marsh,
            ),
            (
                "/assets/ui/generated/scene-rookhill-gate-interior",
                Bed::Graveyard,
            ),
            (
                "/assets/ui/generated/scene-thistledown-farm-arrival",
                Bed::Farm,
            ),
            ("/assets/ui/generated/scene-courier-road", Bed::Night),
            ("", Bed::Night),
        ];
        for (asset, expected) in cases {
            assert_eq!(bed_of(asset), expected, "{asset}");
        }
        let facts = facts_of(
            &scene(""),
            &journey(rpc::JourneyPhase::Room),
            "scene-lobby-harbor",
        );
        assert_eq!(facts.bed, Bed::Harbour);
    }

    #[test]
    fn mix_adds_layers_with_tempo_and_a_release_drops_percussion() {
        let calm = mix_for(Phase::Calm, 150, false).layers;
        let build = mix_for(Phase::Build, 350, false).layers;
        let anticipation = mix_for(Phase::Anticipation, 550, false).layers;
        let peak = mix_for(Phase::Peak, 1_000, false).layers;
        let release = mix_for(Phase::Release, 450, false);
        assert!(calm.bed > 0.0 && calm.motif == 0.0 && calm.tension == 0.0 && calm.combat == 0.0);
        assert!(build.motif > 0.0 && build.tension == 0.0);
        assert!(anticipation.tension > 0.0 && anticipation.combat == 0.0);
        assert!(peak.combat > 0.0 && peak.tension > 0.0);
        assert_eq!(release.layers.tension, 0.0);
        assert_eq!(release.layers.combat, 0.0);
        assert!(release.layers.motif > 0.0);
        assert!(release.fall_seconds < 1.0);
    }

    #[test]
    fn every_ramp_except_the_release_drop_is_a_slow_one() {
        for (phase, mix) in MIX_TABLE {
            assert!((1.5..=4.0).contains(&mix.rise_seconds), "{phase:?} rise");
            let release_drop = phase == Phase::Release;
            assert!(
                release_drop || (1.5..=4.0).contains(&mix.fall_seconds),
                "{phase:?} fall"
            );
        }
    }

    #[test]
    fn intensity_scales_the_defining_layer_and_defeat_is_sombre() {
        let soft = mix_for(Phase::Peak, 750, false).layers.combat;
        let full = mix_for(Phase::Peak, 1_000, false).layers.combat;
        assert!(soft < full && (full - 1.0).abs() < 1e-6);
        let plain = mix_for(Phase::Release, 450, false);
        let sombre = mix_for(Phase::Release, 200, true);
        assert!(sombre.layers.motif < plain.layers.motif);
        assert!(sombre.filter_hertz < plain.filter_hertz);
    }

    #[test]
    fn one_shots_land_on_the_next_beat_of_the_layer_grid() {
        assert_eq!(next_beat(5.0, None), 5.0);
        assert_eq!(next_beat(5.0, Some(6.0)), 5.0);
        let start = 10.0;
        let beat = next_beat(start + 1.0, Some(start));
        assert!((beat - (start + 2.0 * BEAT_SECONDS)).abs() < 1e-9);
        assert!(beat > start + 1.0 && beat - (start + 1.0) <= BEAT_SECONDS);
        // Too close to the beat to schedule: take the following one.
        let almost = start + BEAT_SECONDS - 0.005;
        assert!((next_beat(almost, Some(start)) - (start + 2.0 * BEAT_SECONDS)).abs() < 1e-9);
    }

    #[test]
    fn a_release_settles_to_calm_only_after_its_hold() {
        let release = Tempo {
            phase: Phase::Release,
            intensity: 450,
            sequence: 1,
        };
        assert_eq!(
            effective_phase(release, RELEASE_HOLD_MILLIS - 1.0),
            Phase::Release
        );
        assert_eq!(effective_phase(release, RELEASE_HOLD_MILLIS), Phase::Calm);
        let peak = Tempo {
            phase: Phase::Peak,
            ..release
        };
        assert_eq!(effective_phase(peak, 1.0e9), Phase::Peak);
    }
}
