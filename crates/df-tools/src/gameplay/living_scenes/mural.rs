//! Session mural admission shares the canonical worker, hub, cache and stale-result fencing.
use super::*;
use crate::gameplay::imagery::{FluxPerson, party_mural, party_sheet};

#[derive(Clone, Debug)]
pub(super) struct MuralHero {
    person: FluxPerson,
    image: ImageKey,
}

impl References {
    /// A real scene projection settles the party. Reconnecting only re-observes this demand.
    pub(in crate::gameplay::living_scenes) fn admit_mural(&mut self) {
        self.mural_enabled = true;
        self.refresh_mural();
    }

    pub(super) fn refresh_mural(&mut self) {
        if !self.mural_enabled {
            return;
        }
        let roster = self.seats.values().copied().collect::<BTreeSet<_>>();
        if roster.len() != self.mural_expected_count {
            self.entries.remove(&Target::Mural);
            return;
        }
        let heroes = roster
            .iter()
            .map(|entity| {
                let entry = self.entries.get(&Target::Hero(*entity))?;
                let Source::Hero {
                    name,
                    public_design,
                    ..
                } = &entry.work.source
                else {
                    return None;
                };
                Some((
                    entry.work.binding.clone(),
                    MuralHero {
                        person: FluxPerson {
                            name: name.clone(),
                            appearance: Some(public_design.clone()),
                        },
                        image: self.hero(*entity)?,
                    },
                ))
            })
            .collect::<Option<Vec<_>>>();
        let Some(heroes) = heroes.filter(|heroes| !heroes.is_empty()) else {
            // A changed roster must never show or publish the old party while references settle.
            self.entries.remove(&Target::Mural);
            return;
        };
        let revision = digest(
            format!(
                "party-mural-v1|{}",
                heroes
                    .iter()
                    .map(|(binding, hero)| format!("{binding}:{}", hero.image.as_str()))
                    .collect::<Vec<_>>()
                    .join("|")
            )
            .as_bytes(),
        );
        self.select(
            Target::Mural,
            Source::Mural(heroes.into_iter().map(|(_, hero)| hero).collect()),
            &revision,
        );
    }

    /// Safe additive extension; callers preserve the other scene contents verbatim as JSON.
    pub(in crate::gameplay::living_scenes) fn mural_view(&self) -> serde_json::Value {
        let entry = self.entries.get(&Target::Mural);
        let state = match entry.map(|entry| &entry.status) {
            Some(Status::Ready(_)) => mural_view::MuralState::Ready,
            Some(Status::Failed(_)) => mural_view::MuralState::Failed,
            _ if self.seats.values().any(|entity| {
                matches!(
                    self.status(&Target::Hero(*entity)),
                    ReferenceStatus::Failed(_)
                )
            }) =>
            {
                mural_view::MuralState::Failed
            }
            _ => mural_view::MuralState::Pending,
        };
        let url = entry
            .and_then(|entry| match &entry.status {
                Status::Ready(key) => Some(key.path()),
                _ => None,
            })
            .unwrap_or_default();
        let view = mural_view::MuralView {
            state,
            url,
            key: entry
                .map(|entry| entry.work.binding.clone())
                .unwrap_or_default(),
        };
        // Validate the exact serialized extension with the same boundary the browser uses.
        let contents = serde_json::json!({"party_mural":view});
        if mural_view::MuralView::from_scene_contents(&contents.to_string()).is_none() {
            super::super::record("gameplay.party_mural.invalid_projection", &[]);
            return serde_json::json!({"state":"failed","url":"","key":""});
        }
        contents["party_mural"].clone()
    }

    pub(in crate::gameplay::living_scenes) fn project_mural(
        &mut self,
        scene: &mut rpc::GameplayScene,
    ) {
        self.admit_mural();
        let extension = serde_json::json!({"party_mural":self.mural_view()}).to_string();
        if !mural_view::merge_scene_contents(&mut scene.scene_contents_json, &extension)
            && mural_view::MuralView::from_scene_contents(&scene.scene_contents_json).is_none()
        {
            super::super::record("gameplay.party_mural.invalid_scene_contents", &[]);
        }
    }
}

impl Inner {
    pub(super) async fn render_mural(
        &self,
        work: &Work,
        heroes: &[MuralHero],
    ) -> Result<ImageKey, ImageFailure> {
        // This photograph uses the approved reference-edit route, never a provider substitution.
        if !self.imagery.is_on() {
            return Err(ImageFailure::Off);
        }
        if self.imagery.model_label() != df_providers::ImageEditModel::Gpt25Flare.model_id() {
            return Err(ImageFailure::InvalidRequest);
        }
        let mut references = Vec::with_capacity(heroes.len());
        for hero in heroes {
            match self.imagery.image(hero.image.as_str()).await {
                Lookup::Image { bytes, format } => {
                    references.push(ReferenceImage::new(format, bytes))
                }
                // A temporarily missing approved reference can become available again.
                _ => return Err(ImageFailure::Interrupted),
            }
        }
        if !locked(&self.state)
            .canonical
            .entries
            .get(&Target::Mural)
            .is_some_and(|entry| entry.work.binding == work.binding)
        {
            return Err(ImageFailure::Interrupted);
        }
        let sheet = tokio::task::spawn_blocking(move || party_sheet(&references))
            .await
            .map_err(|_| ImageFailure::Interrupted)?
            .map_err(|_| ImageFailure::InvalidRequest)?;
        let people = heroes
            .iter()
            .map(|hero| hero.person.clone())
            .collect::<Vec<_>>();
        // ScenePlate carries SceneParty metadata through the existing photo/count/custody gate.
        let request = ImageRequest {
            kind: ImageKind::ScenePlate,
            references: vec![sheet],
            prompt: party_mural(&people, &work.binding)?,
            output: OutputSpec::BACKDROP,
        };
        if work.regenerate {
            use crate::gameplay::resources::{Invalidate, PlateSpec, ResourceKey, ResourceStatus};
            let image_key = self.imagery.key_for(&request);
            let spec = PlateSpec::new(
                "party-mural",
                image_key.as_str(),
                art_contract::Neutral::Nothing,
            );
            let key = ResourceKey::of(&spec);
            if matches!(
                self.imagery.hub().status(&key),
                ResourceStatus::Failed(_, _)
            ) {
                self.imagery.hub().invalidate(Invalidate::Key(&key));
            }
        }
        self.imagery.generate("party-mural", request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay::imagery::{Imagery, tests::DirectionFixture};
    use std::path::PathBuf;

    fn inner(imagery: Imagery) -> Inner {
        let timing = Timing::from_interval(Duration::from_secs(30));
        Inner {
            imagery,
            references: ReferenceStore::new(
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ui/generated"),
            ),
            social: SocialLedger::none(),
            rating: ContentRating::Restrained,
            timing,
            state: Mutex::new(State::new(timing)),
            wake: watch::channel(0).0,
            origin: Instant::now(),
        }
    }

    async fn ready_party(inner: &Inner) -> ImageKey {
        let photo = ReferenceImage::new(df_providers::ImageFormat::Jpeg, canonical_photo().into());
        let key = inner
            .imagery
            .register_reviewed("mural-fixture-heroes", photo)
            .await
            .unwrap();
        let mut state = locked(&inner.state);
        state.canonical.fixture_hero(1, "Brakka", key.clone());
        state.canonical.fixture_hero(2, "Sigrun", key.clone());
        state.canonical.mural_expected_count = 2;
        state.canonical.admit_mural();
        key
    }

    fn canonical_photo() -> Vec<u8> {
        super::super::hero_identity_fixture::portrait_bytes()
    }

    fn take_mural(inner: &Inner) -> Work {
        locked(&inner.state)
            .canonical
            .take_jobs()
            .into_iter()
            .find_map(|job| match job {
                Job::Canonical(work) if work.target == Target::Mural => Some(work),
                _ => None,
            })
            .expect("one mural request")
    }

    #[tokio::test]
    async fn hs54_transient_mural_reference_miss_requeues_only_after_cooldown() {
        let cache = std::env::temp_dir().join(format!("hs54-mural-retry-{}", std::process::id()));
        let fixture = DirectionFixture::photographic(&cache);
        let inner = inner(fixture.imagery.clone());
        ready_party(&inner).await;
        let work = take_mural(&inner);
        let mut missing = work.clone();
        let Source::Mural(heroes) = &mut missing.source else {
            panic!("mural")
        };
        heroes[0].image = ImageKey::parse(&"f".repeat(64)).unwrap();
        assert_eq!(
            inner.render_canonical(&missing).await,
            Err(ImageFailure::Interrupted)
        );
        inner.finish_canonical(work.clone(), Err(ImageFailure::Interrupted));
        // Simulate an exhausted hub entry for the exact request, so retry must invalidate the
        // ScenePlate resource key rather than the distinct Depiction kind.
        let Source::Mural(heroes) = &work.source else {
            panic!("mural")
        };
        let mut references = Vec::new();
        for hero in heroes {
            let Lookup::Image { bytes, format } = inner.imagery.image(hero.image.as_str()).await
            else {
                panic!("reference")
            };
            references.push(ReferenceImage::new(format, bytes));
        }
        let request = ImageRequest {
            kind: ImageKind::ScenePlate,
            references: vec![party_sheet(&references).unwrap()],
            prompt: party_mural(
                &heroes
                    .iter()
                    .map(|hero| hero.person.clone())
                    .collect::<Vec<_>>(),
                &work.binding,
            )
            .unwrap(),
            output: OutputSpec::BACKDROP,
        };
        let image_key = inner.imagery.key_for(&request);
        let spec = crate::gameplay::resources::PlateSpec::new(
            "party-mural",
            image_key.as_str(),
            art_contract::Neutral::Nothing,
        );
        assert!(
            inner
                .imagery
                .hub()
                .render(
                    &spec,
                    Box::pin(async {
                        Err(crate::gameplay::resources::ResourceFailure::Permanent(
                            "fixture",
                        ))
                    })
                )
                .await
                .is_err()
        );
        {
            let mut state = locked(&inner.state);
            state.canonical.admit_mural();
            assert!(state.canonical.take_jobs().is_empty());
            state
                .canonical
                .entries
                .get_mut(&Target::Mural)
                .unwrap()
                .started = Some(Instant::now() - ROOM_RETRY_COOLDOWN);
            state.canonical.admit_mural();
        }
        let retry = take_mural(&inner);
        assert_eq!(retry.binding, work.binding);
        assert!(retry.regenerate);
        assert_eq!(fixture.calls(), 0);
        assert!(
            inner.render_canonical(&retry).await.is_ok(),
            "cooldown retry clears the exhausted PlateSpec"
        );
        assert_eq!(fixture.calls(), 1);
    }

    #[tokio::test]
    async fn party_mural_requests_once_reuses_after_reconnect_and_restart_and_changes_with_roster()
    {
        let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../artifacts/tmp/lanes/HS49/mural-cache-{}",
            std::process::id()
        ));
        let fixture = DirectionFixture::photographic(&cache);
        let first = inner(fixture.imagery.clone());
        let reference = ready_party(&first).await;
        let work = take_mural(&first);
        let key = first.render_canonical(&work).await.unwrap();
        first.finish_canonical(work, Ok(key.clone()));
        assert_eq!(fixture.calls(), 1);
        assert_eq!(
            fixture.image_count(),
            1,
            "one reference sheet containing every canonical hero"
        );
        let prompt = fixture.prompt();
        assert!(prompt.contains("Exactly 2 heroes"));
        assert!(prompt.contains("Brakka") && prompt.contains("Sigrun"));
        {
            let mut state = locked(&first.state);
            state.canonical.admit_mural();
            assert!(
                state.canonical.take_jobs().is_empty(),
                "reconnecting never repeats a request"
            );
            let mut scene = rpc::GameplayScene {
                scene_contents_json: "{\"weather\":\"rain\"}".into(),
                ..Default::default()
            };
            state.canonical.project_mural(&mut scene);
            let value: serde_json::Value =
                serde_json::from_str(&scene.scene_contents_json).unwrap();
            assert_eq!(value["weather"], "rain");
            assert_eq!(value["party_mural"]["state"], "ready");
            assert_eq!(value["party_mural"]["url"], key.path());
        }
        let restarted_fixture = DirectionFixture::photographic(&cache);
        let restarted = inner(restarted_fixture.imagery.clone());
        ready_party(&restarted).await;
        let work = take_mural(&restarted);
        assert_eq!(restarted.render_canonical(&work).await.unwrap(), key);
        assert_eq!(
            restarted_fixture.calls(),
            0,
            "persistent cache reuses the exact photograph after restart"
        );
        restarted.finish_canonical(work, Ok(key.clone()));
        assert!(
            matches!(
                restarted.imagery.image(key.as_str()).await,
                Lookup::Image { .. }
            ),
            "the restarted server serves the cached mural bytes"
        );
        {
            let mut state = locked(&first.state);
            state.canonical.fixture_hero(3, "Mira", reference.clone());
            state.canonical.mural_expected_count = 3;
            state.canonical.admit_mural();
            assert_eq!(state.canonical.mural_view()["state"], "pending");
        }
        let work = take_mural(&first);
        let changed = first.render_canonical(&work).await.unwrap();
        assert_ne!(changed, key);
        first.finish_canonical(work, Ok(changed.clone()));
        assert_eq!(fixture.calls(), 2);
        assert!(fixture.prompt().contains("Exactly 3 heroes"));
        {
            let mut state = locked(&first.state);
            state.canonical.fixture_hero(1, "Brakka revised", reference);
            state.canonical.admit_mural();
        }
        let work = take_mural(&first);
        assert_ne!(first.render_canonical(&work).await.unwrap(), changed);
        assert_eq!(
            fixture.calls(),
            3,
            "a depiction revision creates one new mural"
        );
    }

    #[test]
    fn party_mural_waits_for_every_hero_and_rejects_old_roster_completions() {
        let inner = inner(Imagery::disabled(
            crate::gameplay::startup_configuration::ImageryOff::Qualification,
        ));
        let key = ImageKey::parse(&"a".repeat(64)).unwrap();
        {
            let mut state = locked(&inner.state);
            state.canonical.fixture_hero(1, "Brakka", key.clone());
            state.canonical.mural_expected_count = 2;
            state.canonical.admit_mural();
            assert!(
                state.canonical.take_jobs().is_empty(),
                "an incomplete roster cannot produce a partial mural"
            );
            state.canonical.fixture_hero(2, "Sigrun", key.clone());
        }
        let old = take_mural(&inner);
        {
            let mut state = locked(&inner.state);
            state.canonical.seats.remove(&2);
            state.canonical.mural_expected_count = 1;
            state.canonical.admit_mural();
        }
        inner.finish_canonical(old, Ok(key));
        assert_eq!(
            locked(&inner.state).canonical.mural_view()["state"],
            "pending"
        );
        let current = take_mural(&inner);
        inner.finish_canonical(current, Err(ImageFailure::Provider));
        let mut state = locked(&inner.state);
        state.canonical.admit_mural();
        assert!(
            state.canonical.take_jobs().is_empty(),
            "failure is terminal for this roster binding"
        );
        assert_eq!(state.canonical.mural_view()["state"], "failed");
    }
}
