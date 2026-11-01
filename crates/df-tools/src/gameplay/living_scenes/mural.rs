//! Session mural admission shares canonical work fencing and the durable image service.
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
            // A temporarily missing approved reference can become available again.
            let reference = self
                .imagery
                .reference(&hero.image)
                .await
                .map_err(|_| ImageFailure::Interrupted)?;
            references.push(reference);
        }
        if !locked(&self.state)
            .canonical
            .entries
            .get(&Target::Mural)
            .is_some_and(|entry| entry.work.binding == work.binding)
        {
            return Err(ImageFailure::Interrupted);
        }
        let sheet = party_sheet(&self.imagery, &references)
            .await
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
        self.imagery
            .generate_for_purpose(
                "party-mural",
                request,
                df_imagery::Purpose::PartyMural,
                crate::gameplay::imagery::ImagePriority::Foreground,
                df_imagery::Freshness::ReuseCompatible,
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay::imagery::{Imagery, tests::DirectionFixture};
    use std::path::PathBuf;

    fn inner(imagery: Imagery) -> std::sync::Arc<Inner> {
        let timing = Timing::from_interval(Duration::from_secs(30));
        std::sync::Arc::new(Inner {
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
        })
    }

    async fn ready_party(inner: &Inner, fixture: &DirectionFixture) -> ImageKey {
        let (key, _) = fixture
            .import_bytes(df_providers::ImageFormat::Jpeg, canonical_photo().into())
            .await;
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
        let Some(fixture) = DirectionFixture::photographic(&cache).await else {
            return;
        };
        let inner = inner(fixture.imagery.clone());
        ready_party(&inner, &fixture).await;
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
        // A reference miss precedes admission. Cooldown re-observes the same intent;
        // it neither invalidates paid state nor manufactures a regeneration nonce.
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
        assert_eq!(fixture.calls(), 0);
        let first = inner.render_canonical(&retry).await.unwrap();
        assert_eq!(fixture.calls(), 1);
        assert_eq!(inner.render_canonical(&retry).await.unwrap(), first);
        assert_eq!(
            fixture.calls(),
            1,
            "same accepted intent reuses the actual result"
        );
        drop(inner);
        fixture.close().await;
    }

    #[tokio::test]
    async fn mural_unknown_cooldown_and_restart_reobserve_without_another_supplier_send() {
        let cache = std::env::temp_dir().join(format!("mural-unknown-{}", std::process::id()));
        let Some(mut fixture) = DirectionFixture::unknown(&cache).await else {
            return;
        };
        let first = inner(fixture.imagery.clone());
        ready_party(&first, &fixture).await;
        let work = take_mural(&first);
        assert_eq!(
            first.render_canonical(&work).await,
            Err(ImageFailure::Interrupted)
        );
        assert_eq!(
            fixture.calls(),
            1,
            "the actual endpoint consumed the request before closing"
        );
        first.finish_canonical(work.clone(), Err(ImageFailure::Interrupted));
        {
            let mut state = locked(&first.state);
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
        let retry = take_mural(&first);
        assert_eq!(retry.binding, work.binding);
        assert_eq!(
            first.render_canonical(&retry).await,
            Err(ImageFailure::Interrupted)
        );
        assert_eq!(
            fixture.calls(),
            1,
            "cooldown cannot turn Unknown into a second send"
        );
        drop(first);
        fixture.restart().await;
        let restarted = inner(fixture.imagery.clone());
        ready_party(&restarted, &fixture).await;
        let replay = take_mural(&restarted);
        assert_eq!(replay.binding, work.binding);
        assert_eq!(
            restarted.render_canonical(&replay).await,
            Err(ImageFailure::Interrupted)
        );
        assert_eq!(
            fixture.calls(),
            1,
            "the durable Unknown receipt survives host restart"
        );
        drop(restarted);
        fixture.close().await;
    }

    #[tokio::test]
    async fn party_mural_requests_once_reuses_after_reconnect_and_restart_and_changes_with_roster()
    {
        let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../artifacts/tmp/lanes/HS49/mural-cache-{}",
            std::process::id()
        ));
        let Some(mut fixture) = DirectionFixture::photographic(&cache).await else {
            return;
        };
        let first = inner(fixture.imagery.clone());
        let reference = ready_party(&first, &fixture).await;
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
        let calls_before_restart = fixture.calls();
        drop(first);
        fixture.restart().await;
        let restarted = inner(fixture.imagery.clone());
        ready_party(&restarted, &fixture).await;
        let work = take_mural(&restarted);
        assert_eq!(restarted.render_canonical(&work).await.unwrap(), key);
        assert_eq!(
            fixture.calls() - calls_before_restart,
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
            let mut state = locked(&restarted.state);
            state.canonical.fixture_hero(3, "Mira", reference.clone());
            state.canonical.mural_expected_count = 3;
            state.canonical.admit_mural();
            assert_eq!(state.canonical.mural_view()["state"], "pending");
        }
        let work = take_mural(&restarted);
        let changed = restarted.render_canonical(&work).await.unwrap();
        assert_ne!(changed, key);
        restarted.finish_canonical(work, Ok(changed.clone()));
        assert_eq!(fixture.calls(), 2);
        assert!(fixture.prompt().contains("Exactly 3 heroes"));
        {
            let mut state = locked(&restarted.state);
            state.canonical.fixture_hero(1, "Brakka revised", reference);
            state.canonical.admit_mural();
        }
        let work = take_mural(&restarted);
        assert_ne!(restarted.render_canonical(&work).await.unwrap(), changed);
        assert_eq!(
            fixture.calls(),
            3,
            "a depiction revision creates one new mural"
        );
        drop(restarted);
        fixture.close().await;
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
