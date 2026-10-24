use super::*;
use std::collections::HashSet;

#[derive(Default)]
struct VariantHub {
    requests: Mutex<Vec<MusicSetSpec>>,
    urgent_ready: Mutex<bool>,
}

impl MusicHub for VariantHub {
    fn get_or_request(&self, set: &MusicSetSpec) -> MusicStatus {
        locked(&self.requests).push(set.clone());
        self.status(set)
    }

    fn status(&self, set: &MusicSetSpec) -> MusicStatus {
        if set.band() == PacingBand::Urgent && !*locked(&self.urgent_ready) {
            return MusicStatus::Pending;
        }
        MusicStatus::Ready(ReadyMusicSet {
            layer_paths: ["bed", "motif", "tension", "combat"]
                .map(|layer| format!("/gameplay/music/{}/{layer}", set.key())),
            ambience_path: None,
        })
    }

    fn invalidate(&self, _: &MusicSetSpec) {}
    fn regenerate(&self, _: &MusicSetSpec) {}
}

#[test]
fn pacing_variant_keys_keep_the_spec_key_palette_and_motif_identity() {
    let spec = test_support::shop_block().spec().unwrap();
    let base = MusicSetSpec::new(spec.clone());
    let mut keys = HashSet::new();
    for band in PacingBand::ALL {
        let set = MusicSetSpec::in_band(spec.clone(), band);
        assert_eq!(set.spec(), &spec);
        assert_eq!(set.spec().key_name(), "G major");
        assert_eq!(set.motif_reference(), base.motif_reference());
        assert!(keys.insert(set.key()));
        let prompt = set.spec().layer_prompt_at(spec::Layer::Motif, band);
        assert!(prompt.contains(&format!("Exactly {} BPM", band.bpm())));
        assert!(prompt.contains(&format!("{} bars", band.bars())));
        assert!(prompt.contains("G major"));
    }
}

#[tokio::test]
async fn ready_sets_request_neighbours_and_pending_variants_keep_the_running_set() {
    let hub = Arc::new(VariantHub::default());
    let music = PlaceMusic::new(
        ApproachModel::default(),
        Some(hub.clone()),
        tokio::runtime::Handle::current(),
        || {},
    );
    let block = test_support::shop_block();
    let spec = block.spec().unwrap();
    music.override_spec("shop", &block).unwrap();
    let first = music.plan_for("shop", &spec);
    let requested = locked(&hub.requests)
        .iter()
        .map(|set| set.band().label())
        .collect::<Vec<_>>();
    assert_eq!(requested, ["steady", "calm", "driving"]);
    music.set_band(PacingBand::Urgent);
    let waiting = music.plan_for("shop", &spec);
    assert_eq!(waiting.spec_key, first.spec_key);
    assert_eq!(waiting.layer_paths, first.layer_paths);
    assert_eq!(waiting.bpm, 84);
    *locked(&hub.urgent_ready) = true;
    let switched = music.plan_for("shop", &spec);
    assert_ne!(switched.spec_key, first.spec_key);
    assert_eq!(switched.bpm, 126);
    assert_eq!(switched.loop_samples, LOOP_SAMPLES);
    let before = locked(&hub.requests).len();
    let variants = music.variants("shop").unwrap();
    assert_eq!(variants.as_array().unwrap().len(), 4);
    assert_eq!(variants[3]["playing"], true);
    assert_eq!(variants[3]["status"], "ready");
    assert_eq!(
        locked(&hub.requests).len(),
        before,
        "dev reads never request work"
    );
}
