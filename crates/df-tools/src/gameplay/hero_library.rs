//! Phone-key library RPCs. Save snapshots only the authenticated committed seat.
use df_persistence::hero_library::{HeroLibrary, HeroRevision, IdentityRecord, PhoneKey};
use df_protocol::common as rpc;
use prost::Message;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::Mutex, time::Duration};
use tonic::{Request, Response, Status};

static DATABASE: Mutex<Option<tokio_postgres::Config>> = Mutex::new(None);
// Optional attachment cache for the existing synchronous creation hook. It holds no
// library hero/revision origin; saves recover that independently from the phone hint.
static PORTRAIT_IMPORTS: Mutex<BTreeMap<[u8; 32], PortraitImport>> = Mutex::new(BTreeMap::new());
struct PortraitImport {
    identity: rpc::LibraryHeroIdentity,
    attachment: super::creator_portraits::Attachment,
}
const RECORD_PREFIX: &str = "identity-portrait:creator-v1:";
pub(super) fn configure(database: tokio_postgres::Config) {
    if let Ok(mut configured) = DATABASE.lock() {
        *configured = Some(database);
    }
    if let Ok(mut imports) = PORTRAIT_IMPORTS.lock() {
        imports.clear();
    }
}
pub(super) fn config() -> Result<tokio_postgres::Config, Status> {
    DATABASE
        .lock()
        .map_err(|_| unavailable())?
        .clone()
        .ok_or_else(unavailable)
}
fn unavailable() -> Status {
    Status::unavailable("Your hero is safe in this game. Not yet saved to your heroes.")
}
fn failure(error: df_persistence::hero_library::LibraryError) -> Status {
    super::actor::record_audit_line(
        "hero_library.failure",
        format!(
            "HERO_LIBRARY failure class={}",
            match error {
                df_persistence::hero_library::LibraryError::Deadline => "deadline",
                df_persistence::hero_library::LibraryError::NotFound => "not_found",
                df_persistence::hero_library::LibraryError::InvalidRecord => "invalid_record",
                df_persistence::hero_library::LibraryError::Database(_) => "database",
            }
        ),
    );
    unavailable()
}
pub(super) async fn open() -> Result<HeroLibrary, Status> {
    HeroLibrary::open(&config()?).await.map_err(failure)
}
/// Full approved creator data shares the existing non-rules identity attachment namespace.
/// Each checkpoint label is <=128 bytes; the record is bounded before chunking.
pub(super) fn identity_choices(
    selection: Option<&rpc::CharacterSelection>,
) -> Result<Vec<(df_types::RevisionLabel, df_types::RevisionLabel)>, Status> {
    let Some(selection) = selection else {
        return Ok(Vec::new());
    };
    let mut record = rpc::LibraryHeroIdentity {
        name: selection.name.clone(),
        design: selection
            .choices
            .iter()
            .filter(|choice| matches!(choice.group_id.as_str(), "species" | "class" | "background"))
            .cloned()
            .collect(),
        identity: selection.identity.clone(),
        portrait_scenery: selection.portrait_scenery.clone(),
        private_backstory: String::new(),
        sealed_backstory: String::new(),
    };
    record
        .design
        .sort_by(|left, right| left.group_id.cmp(&right.group_id));
    // Public look already has its own lossless canonical checkpoint chunks. Avoid duplicating
    // it in both attachment and appearance records under the checkpoint's record budget.
    if let Some(appearance) = record
        .identity
        .as_mut()
        .and_then(|identity| identity.appearance.as_mut())
    {
        appearance.public_description.clear();
    }
    let bytes = record.encode_to_vec();
    if bytes.len() > df_persistence::hero_library::MAX_IDENTITY_BYTES {
        return Err(Status::invalid_argument("Hero identity is too large"));
    }
    let mut choices = Vec::new();
    for (index, chunk) in bytes.chunks(96).enumerate() {
        choices.push((
            super::model::label(&format!("{RECORD_PREFIX}{index:04}"))
                .map_err(super::actor::unavailable)?,
            super::model::label(&encode_record_chunk(chunk)).map_err(super::actor::unavailable)?,
        ));
    }
    Ok(choices)
}
pub(super) struct SeatSnapshot {
    pub(super) seat: Vec<u8>,
    pub(super) hero: Option<rpc::LibraryHeroIdentity>,
    pub(super) portrait: Option<super::creator_portraits::Attachment>,
}
pub(super) fn snapshot(
    checkpoint: &df_model::checkpoint::Checkpoint,
    member: df_types::MemberId,
) -> Result<SeatSnapshot, Status> {
    let character = checkpoint
        .state()
        .characters
        .iter()
        .find(|character| character.owner == member);
    let Some(character) = character else {
        return Ok(SeatSnapshot {
            seat: member.as_bytes().to_vec(),
            hero: None,
            portrait: None,
        });
    };
    let field = |key: &str| {
        character
            .choices
            .iter()
            .find(|choice| choice.offer.as_str() == key)
            .map(|choice| choice.selected.as_str())
    };
    let mut bytes = Vec::new();
    // Read both the original 48-byte chunks and the denser 96-byte chunks. Ordering is
    // recorded by the index, independently of chunk width.
    for (expected_index, choice) in character
        .choices
        .iter()
        .filter(|choice| choice.offer.as_str().starts_with(RECORD_PREFIX))
        .enumerate()
    {
        let index = choice
            .offer
            .as_str()
            .strip_prefix(RECORD_PREFIX)
            .and_then(|text| text.parse::<usize>().ok())
            .ok_or_else(unavailable)?;
        if index != expected_index {
            return Err(unavailable());
        }
        bytes.extend(decode_record_chunk(choice.selected.as_str()).ok_or_else(unavailable)?);
        if bytes.len() > df_persistence::hero_library::MAX_IDENTITY_BYTES {
            return Err(unavailable());
        }
    }
    let mut hero = if bytes.is_empty() {
        let choices = character
            .choices
            .iter()
            .map(|choice| {
                (
                    choice.offer.as_str().to_owned(),
                    choice.selected.as_str().to_owned(),
                )
            })
            .collect::<Vec<_>>();
        rpc::LibraryHeroIdentity {
            name: super::journey::name(checkpoint.state(), character.entity)
                .map_err(super::actor::unavailable)?,
            identity: Some(
                super::hero_identity::ValidatedIdentity::from_canonical_appearance_choices(
                    &choices,
                )
                .map_err(|_| unavailable())?
                .to_rpc(),
            ),
            design: choices
                .into_iter()
                .filter(|(group, _)| matches!(group.as_str(), "species" | "class" | "background"))
                .map(|(group_id, option_id)| rpc::JourneyChoice {
                    group_id,
                    option_id,
                })
                .collect(),
            ..Default::default()
        }
    } else {
        rpc::LibraryHeroIdentity::decode(bytes.as_slice()).map_err(|_| unavailable())?
    };
    if character
        .choices
        .iter()
        .any(|choice| choice.offer.as_str().starts_with("identity-appearance:"))
    {
        let canonical = character
            .choices
            .iter()
            .map(|choice| {
                (
                    choice.offer.as_str().to_owned(),
                    choice.selected.as_str().to_owned(),
                )
            })
            .collect::<Vec<_>>();
        let look =
            super::hero_identity::ValidatedIdentity::from_canonical_appearance_choices(&canonical)
                .map_err(|_| unavailable())?
                .to_rpc();
        if let (Some(target), Some(source)) = (
            hero.identity
                .as_mut()
                .and_then(|identity| identity.appearance.as_mut()),
            look.appearance,
        ) {
            target.public_description = source.public_description;
        }
    }
    let portrait = if let Some(key) = field("identity-portrait:key") {
        Some(super::creator_portraits::Attachment {
            key: key.to_owned(),
            digest: super::journey::unhex(
                field("identity-portrait:digest").ok_or_else(unavailable)?,
            )
            .map_err(super::actor::unavailable)?
            .try_into()
            .map_err(|_| unavailable())?,
            bytes: field("identity-portrait:bytes")
                .and_then(|value| value.parse().ok())
                .ok_or_else(unavailable)?,
            revision: field("identity-portrait:revision")
                .and_then(|value| value.parse().ok())
                .ok_or_else(unavailable)?,
        })
    } else {
        None
    };
    Ok(SeatSnapshot {
        seat: member.as_bytes().to_vec(),
        hero: Some(hero),
        portrait,
    })
}
async fn seat(
    service: &super::Service,
    credential: [u8; 32],
    scope: Option<rpc::WatchViewRequest>,
) -> Result<SeatSnapshot, Status> {
    let (reply, wait) = tokio::sync::oneshot::channel();
    service
        .actor
        .try_submit(super::actor::Call::LibraryHero {
            credential,
            request: scope.ok_or_else(|| Status::invalid_argument("Seat scope required"))?,
            reply,
        })
        .map_err(|_| Status::resource_exhausted("Seat is busy; retry after reconnect"))?;
    tokio::time::timeout(super::timing::active().reply, wait)
        .await
        .map_err(|_| unavailable())?
        .map_err(|_| unavailable())?
}
fn cache_root() -> Result<std::path::PathBuf, Status> {
    Ok(std::env::var("DF_GAMEPLAY_IMAGERY_CACHE")
        .ok()
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or(
            std::env::current_dir()
                .map_err(|_| unavailable())?
                .join("artifacts/tmp/play/imagery-cache"),
        ))
}
async fn portrait_bytes(
    attachment: Option<super::creator_portraits::Attachment>,
) -> Result<Vec<u8>, Status> {
    let Some(attachment) = attachment else {
        return Ok(Vec::new());
    };
    if attachment.key.len() != 64
        || !attachment.key.bytes().all(|byte| byte.is_ascii_hexdigit())
        || attachment.bytes > df_persistence::hero_library::MAX_PORTRAIT_BYTES as u64
    {
        return Err(unavailable());
    }
    let path = cache_root()?.join(format!("{}.jpg", attachment.key));
    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let file = std::fs::File::open(path).map_err(|_| unavailable())?;
        if file.metadata().map_err(|_| unavailable())?.len() != attachment.bytes {
            return Err(unavailable());
        }
        let mut bytes = Vec::new();
        file.take(df_persistence::hero_library::MAX_PORTRAIT_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| unavailable())?;
        if bytes.len() as u64 != attachment.bytes
            || Sha256::digest(&bytes).as_slice() != attachment.digest
        {
            return Err(unavailable());
        }
        Ok(bytes)
    })
    .await
    .map_err(|_| unavailable())?
}
fn summary(row: &HeroRevision) -> rpc::LibraryHeroSummary {
    rpc::LibraryHeroSummary {
        hero_id: row.reference.hero_id,
        revision: row.reference.revision as u64,
        name: row.record.name.clone(),
        species: row.record.species.clone(),
        character_class: row.record.character_class.clone(),
        portrait: row.record.portrait_thumbnail.clone(),
        portrait_key: if row.record.portrait.is_empty() {
            String::new()
        } else {
            super::hex(&row.portrait_sha256)
        },
        portrait_media_type: row.record.portrait_media_type.clone(),
    }
}
pub(super) fn loaded_portrait(
    credential: [u8; 32],
    selection: Option<&rpc::CharacterSelection>,
) -> Option<super::creator_portraits::Attachment> {
    let selection = selection?;
    let imports = PORTRAIT_IMPORTS.lock().ok()?;
    let imported = imports.get(&credential)?;
    let appearance = selection.identity.as_ref()?.appearance.as_ref()?;
    let previous = imported.identity.identity.as_ref()?.appearance.as_ref()?;
    (appearance == previous
        && selection.portrait_scenery == imported.identity.portrait_scenery
        && imported
            .identity
            .design
            .iter()
            .all(|choice| selection.choices.contains(choice)))
    .then(|| imported.attachment.clone())
}
fn loaded_reference(
    reference: Option<rpc::LibraryHeroReference>,
) -> Option<df_persistence::hero_library::RevisionRef> {
    let reference = reference?;
    let revision = i64::try_from(reference.revision)
        .ok()
        .filter(|revision| *revision > 0)?;
    (reference.hero_id > 0).then_some(df_persistence::hero_library::RevisionRef {
        hero_id: reference.hero_id,
        revision,
    })
}
fn portrait_matches(hero: &rpc::LibraryHeroIdentity, previous: &rpc::LibraryHeroIdentity) -> bool {
    hero.identity
        .as_ref()
        .and_then(|identity| identity.appearance.as_ref())
        == previous
            .identity
            .as_ref()
            .and_then(|identity| identity.appearance.as_ref())
        && hero.portrait_scenery == previous.portrait_scenery
        && hero.design == previous.design
}
#[tonic::async_trait]
impl rpc::hero_library_service_server::HeroLibraryService for super::Service {
    async fn list_my_heroes(
        &self,
        request: Request<rpc::ListMyHeroesRequest>,
    ) -> Result<Response<rpc::ListMyHeroesResponse>, Status> {
        let body = request.into_inner();
        let key = PhoneKey::from_bytes(&body.key)
            .map_err(|_| Status::invalid_argument("Library key invalid"))?;
        if body.after_hero_id < 0 {
            return Err(Status::invalid_argument("Library cursor invalid"));
        }
        let rows = tokio::time::timeout(Duration::from_secs(15), async {
            open()
                .await?
                .list_after(&key, body.after_hero_id)
                .await
                .map_err(failure)
        })
        .await
        .map_err(|_| unavailable())??;
        let response = summaries_page(&rows);
        Ok(Response::new(response))
    }
    async fn load_hero(
        &self,
        request: Request<rpc::LoadHeroRequest>,
    ) -> Result<Response<rpc::LoadHeroResponse>, Status> {
        let credential = super::credential(&request)?;
        let body = request.into_inner();
        let key = PhoneKey::from_bytes(&body.key)
            .map_err(|_| Status::invalid_argument("Library key invalid"))?;
        if body.new_hero {
            if body.hero_id != 0 {
                return Err(Status::invalid_argument(
                    "New hero cannot select a saved id",
                ));
            }
            PORTRAIT_IMPORTS
                .lock()
                .map_err(|_| unavailable())?
                .remove(&credential);
            return Ok(Response::new(rpc::LoadHeroResponse {
                hero: None,
                summary: None,
            }));
        }
        let row = tokio::time::timeout(Duration::from_secs(15), async {
            open()
                .await?
                .load(&key, body.hero_id)
                .await
                .map_err(|_| Status::not_found("Hero unavailable"))
        })
        .await
        .map_err(|_| unavailable())??;
        let hero = rpc::LibraryHeroIdentity::decode(row.record.encoded.as_slice())
            .map_err(|_| unavailable())?;
        if !row.record.portrait.is_empty() {
            let digest: [u8; 32] = Sha256::digest(&row.record.portrait).into();
            if digest.as_slice() != row.portrait_sha256
                || row.record.portrait_media_type != "image/jpeg"
            {
                return Err(unavailable());
            }
            let key = super::hex(&digest);
            let root = cache_root()?;
            let path = root.join(format!("{key}.jpg"));
            let bytes = row.record.portrait.clone();
            tokio::task::spawn_blocking(move || {
                std::fs::create_dir_all(root)?;
                std::fs::write(path, bytes)
            })
            .await
            .map_err(|_| unavailable())?
            .map_err(|_| unavailable())?;
            let mut imports = PORTRAIT_IMPORTS.lock().map_err(|_| unavailable())?;
            if imports.len() >= 16 && !imports.contains_key(&credential) {
                // This optional cache must not prevent loading or durable library saves.
                imports.pop_first();
            }
            imports.insert(
                credential,
                PortraitImport {
                    identity: hero.clone(),
                    attachment: super::creator_portraits::Attachment {
                        key,
                        digest,
                        bytes: row.record.portrait.len() as u64,
                        revision: row.reference.revision as u64,
                    },
                },
            );
        } else {
            PORTRAIT_IMPORTS
                .lock()
                .map_err(|_| unavailable())?
                .remove(&credential);
        }
        Ok(Response::new(rpc::LoadHeroResponse {
            hero: Some(hero),
            summary: Some(summary(&row)),
        }))
    }
    async fn save_my_seat_hero(
        &self,
        request: Request<rpc::SaveMySeatHeroRequest>,
    ) -> Result<Response<rpc::SaveMySeatHeroResponse>, Status> {
        let credential = super::credential(&request)?;
        let body = request.into_inner();
        let key = PhoneKey::from_bytes(&body.key)
            .map_err(|_| Status::invalid_argument("Library key invalid"))?;
        let snapshot = seat(self, credential, body.scope).await?;
        let hero = snapshot
            .hero
            .ok_or_else(|| Status::failed_precondition("Accept a hero first"))?;
        let requested_origin = loaded_reference(body.loaded_hero);
        let mut library = open().await?;
        let source = if let Some(reference) = requested_origin.as_ref() {
            match library.load_revision(&key, reference).await {
                Ok(row) => Some(row),
                Err(df_persistence::hero_library::LibraryError::NotFound) => {
                    super::actor::record_audit_line(
                        "hero_library.origin_ignored",
                        "HERO_LIBRARY origin_ignored".into(),
                    );
                    None
                }
                Err(error) => return Err(failure(error)),
            }
        } else {
            None
        };
        // Loaded portraits remain approved durable bytes even after the server/cache restarts.
        let portrait = if snapshot.portrait.is_some() {
            portrait_bytes(snapshot.portrait).await?
        } else if let Some(source) = source.as_ref() {
            let previous = rpc::LibraryHeroIdentity::decode(source.record.encoded.as_slice())
                .map_err(|_| unavailable())?;
            if portrait_matches(&hero, &previous) {
                source.record.portrait.clone()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        let choice = |group: &str| {
            hero.design
                .iter()
                .find(|choice| choice.group_id == group)
                .map(|choice| choice.option_id.clone())
                .unwrap_or_default()
        };
        let thumbnail = thumbnail(&portrait)?;
        let record = IdentityRecord {
            name: hero.name.clone(),
            species: choice("species"),
            character_class: choice("class"),
            encoded: hero.encode_to_vec(),
            portrait_media_type: if portrait.is_empty() {
                String::new()
            } else {
                "image/jpeg".into()
            },
            portrait,
            portrait_thumbnail: thumbnail,
        };
        let origin = source.as_ref().map(|source| &source.reference);
        let config = config()?;
        let campaign = config.get_dbname().ok_or_else(unavailable)?.to_owned();
        let reference = tokio::time::timeout(Duration::from_secs(20), async {
            let reference = library
                .save_from(
                    &key,
                    &campaign,
                    &snapshot.seat,
                    &record,
                    requested_origin.as_ref(),
                )
                .await
                .map_err(failure)?;
            HeroLibrary::record_campaign_origin(
                &config,
                &snapshot.seat,
                origin.unwrap_or(&reference),
            )
            .await
            .map_err(failure)?;
            HeroLibrary::record_campaign_link(&config, &snapshot.seat, &reference)
                .await
                .map_err(failure)?;
            Ok::<_, Status>(reference)
        })
        .await
        .map_err(|_| unavailable())??;
        super::actor::record_audit_line(
            "hero_library.saved",
            format!(
                "HERO_LIBRARY saved hero={} revision={}",
                reference.hero_id, reference.revision
            ),
        );
        Ok(Response::new(rpc::SaveMySeatHeroResponse {
            hero_id: reference.hero_id,
            revision: reference.revision as u64,
        }))
    }
}
#[cfg(test)]
#[path = "../gameplay_browser/hero_draft_storage.rs"]
mod draft_storage_tests;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loaded_origin_wire_reference_and_portrait_survive_without_process_state() {
        let request = rpc::SaveMySeatHeroRequest {
            key: vec![1; 32],
            scope: None,
            loaded_hero: Some(rpc::LibraryHeroReference {
                hero_id: 19,
                revision: 2,
            }),
        };
        let restarted =
            rpc::SaveMySeatHeroRequest::decode(request.encode_to_vec().as_slice()).unwrap();
        assert_eq!(
            loaded_reference(restarted.loaded_hero).unwrap(),
            df_persistence::hero_library::RevisionRef {
                hero_id: 19,
                revision: 2
            }
        );
        assert!(
            loaded_reference(Some(rpc::LibraryHeroReference {
                hero_id: 19,
                revision: u64::MAX
            }))
            .is_none()
        );
        let original = rpc::LibraryHeroIdentity {
            name: "Hero".into(),
            identity: Some(rpc::HeroIdentity::default()),
            ..Default::default()
        };
        let mut edited = original.clone();
        edited.name = "New name".into();
        edited.identity.as_mut().unwrap().backstory = "New story".into();
        assert!(portrait_matches(&edited, &original));
        edited.portrait_scenery = "Changed scenery".into();
        assert!(!portrait_matches(&edited, &original));
    }
    #[test]
    fn library_snapshot_reads_the_committed_seat_and_preserves_identity_only() {
        let initial = super::super::model::initial().unwrap();
        let mut state = initial.state().clone();
        let member = state.characters[0].owner;
        let mut identity = super::super::hero_identity::Identity::from_menus(&[])
            .unwrap()
            .to_rpc();
        identity.backstory = "Exact story  \r\n\u{96ea} \u{1f409}".into();
        let selection = rpc::CharacterSelection {
            name: "CommittedHero".into(),
            identity: Some(identity.clone()),
            portrait_scenery: "Forest\nwith mist".into(),
            choices: vec![
                rpc::JourneyChoice {
                    group_id: "species".into(),
                    option_id: "dwarf".into(),
                },
                rpc::JourneyChoice {
                    group_id: "class".into(),
                    option_id: "fighter-1".into(),
                },
                rpc::JourneyChoice {
                    group_id: "background".into(),
                    option_id: "soldier".into(),
                },
                rpc::JourneyChoice {
                    group_id: "equipment".into(),
                    option_id: "campaign-only".into(),
                },
                rpc::JourneyChoice {
                    group_id: "array".into(),
                    option_id: "campaign-only".into(),
                },
            ],
        };
        state.characters[0].choices = identity_choices(Some(&selection))
            .unwrap()
            .into_iter()
            .map(|(offer, selected)| df_model::checkpoint::AcceptedChoice {
                participant: member,
                offer,
                selected,
                source: super::super::model::rule().unwrap(),
            })
            .collect();
        let committed = super::super::model::checkpoint(initial.basis(), state).unwrap();
        let saved = snapshot(&committed, member).unwrap().hero.unwrap();
        assert_eq!(saved.name, "CommittedHero");
        assert_eq!(saved.identity, Some(identity));
        assert_eq!(saved.portrait_scenery, selection.portrait_scenery);
        assert_eq!(saved.design.len(), 3);
        assert!(
            !saved
                .design
                .iter()
                .any(|choice| matches!(choice.group_id.as_str(), "equipment" | "array"))
        );
        let other = df_types::MemberId::from_bytes(&[0x79; 16]).unwrap();
        assert!(snapshot(&committed, other).unwrap().hero.is_none());
    }

    #[test]
    fn creator_record_chunks_fit_checkpoint_labels_and_preserve_all_bytes() {
        let bytes = (0..=255).collect::<Vec<u8>>();
        for length in 1..=96 {
            for chunk in bytes.chunks(length) {
                let encoded = encode_record_chunk(chunk);
                assert!(super::super::model::label(&encoded).is_ok());
                assert_eq!(decode_record_chunk(&encoded).unwrap(), chunk);
            }
        }
        assert!(decode_record_chunk("!!!!").is_none());
    }

    #[test]
    fn library_pages_fit_the_existing_rpc_message_bound() {
        let rows = (1..=3)
            .map(|id| HeroRevision {
                reference: df_persistence::hero_library::RevisionRef {
                    hero_id: id,
                    revision: 1,
                },
                record: IdentityRecord {
                    name: "Hero".into(),
                    species: "dwarf".into(),
                    character_class: "fighter-1".into(),
                    encoded: vec![1],
                    portrait: vec![1],
                    portrait_thumbnail: vec![3; 32768],
                    portrait_media_type: "image/jpeg".into(),
                },
                portrait_sha256: vec![4; 32],
            })
            .collect::<Vec<_>>();
        let page = summaries_page(&rows);
        assert_eq!(page.heroes.len(), 1);
        assert_eq!(page.next_cursor, 1);
        assert!(page.encoded_len() < df_rpc_bridge::RPC_MESSAGE_BYTES);
    }

    #[test]
    fn library_save_contract_has_no_client_identity() {
        let request = rpc::SaveMySeatHeroRequest {
            key: vec![7; 32],
            scope: None,
            loaded_hero: None,
        };
        let encoded = request.encode_to_vec();
        // An unknown wire field carrying a fabricated identity is skipped by generated decoding.
        let mut forged = encoded.clone();
        forged.extend([0x7a, 4, b'E', b'v', b'i', b'l']);
        assert_eq!(
            rpc::SaveMySeatHeroRequest::decode(forged.as_slice()).unwrap(),
            request
        );
        assert_eq!(
            rpc::SaveMySeatHeroRequest::decode(encoded.as_slice())
                .unwrap()
                .key,
            vec![7; 32]
        );
    }
}

pub(super) fn router() -> axum::Router {
    axum::Router::new().route(
        "/gameplay/library-portrait/{key}",
        axum::routing::get(serve_portrait),
    )
}
async fn serve_portrait(
    axum::extract::Path(key): axum::extract::Path<String>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    }
    let result = async {
        let digest: [u8; 32] = super::journey::unhex(&key)
            .map_err(super::actor::unavailable)?
            .try_into()
            .map_err(|_| unavailable())?;
        let bytes = open()
            .await?
            .portrait_by_digest(&digest)
            .await
            .map_err(failure)?
            .ok_or_else(unavailable)?;
        if Sha256::digest(&bytes).as_slice() != digest {
            return Err(unavailable());
        }
        Ok::<_, Status>(bytes)
    };
    match tokio::time::timeout(Duration::from_secs(15), result).await {
        Ok(Ok(bytes)) => (
            [
                (axum::http::header::CONTENT_TYPE, "image/jpeg"),
                (
                    axum::http::header::CACHE_CONTROL,
                    "private, max-age=86400, immutable",
                ),
            ],
            bytes,
        )
            .into_response(),
        _ => axum::http::StatusCode::NOT_FOUND.into_response(),
    }
}

fn thumbnail(portrait: &[u8]) -> Result<Vec<u8>, Status> {
    if portrait.is_empty() {
        return Ok(Vec::new());
    }
    let image = image::load_from_memory_with_format(portrait, image::ImageFormat::Jpeg)
        .map_err(|_| unavailable())?
        .thumbnail(96, 144);
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 65)
        .encode_image(&image)
        .map_err(|_| unavailable())?;
    if bytes.len() > 32768 {
        return Err(unavailable());
    }
    Ok(bytes)
}

const RECORD_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
pub(super) fn encode_record_chunk(bytes: &[u8]) -> String {
    let mut result = String::new();
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        for index in [first >> 2, ((first & 3) << 4) | (second >> 4)] {
            result.push(char::from(RECORD_ALPHABET[usize::from(index)]));
        }
        result.push(if chunk.len() > 1 {
            char::from(RECORD_ALPHABET[usize::from(((second & 15) << 2) | (third >> 6))])
        } else {
            '.'
        });
        result.push(if chunk.len() > 2 {
            char::from(RECORD_ALPHABET[usize::from(third & 63)])
        } else {
            '.'
        });
    }
    result
}
pub(super) fn decode_record_chunk(text: &str) -> Option<Vec<u8>> {
    if text.is_empty() || text.len() > 128 || !text.len().is_multiple_of(4) {
        return None;
    }
    let digit = |byte: u8| {
        RECORD_ALPHABET
            .iter()
            .position(|candidate| *candidate == byte)
            .and_then(|value| u8::try_from(value).ok())
    };
    let mut bytes = Vec::new();
    for chunk in text.as_bytes().as_chunks::<4>().0 {
        let first = digit(chunk[0])?;
        let second = digit(chunk[1])?;
        bytes.push((first << 2) | (second >> 4));
        if chunk[2] != b'.' {
            let third = digit(chunk[2])?;
            bytes.push((second << 4) | (third >> 2));
            if chunk[3] != b'.' {
                bytes.push((third << 6) | digit(chunk[3])?);
            }
        }
    }
    (encode_record_chunk(&bytes) == text).then_some(bytes)
}

fn summaries_page(rows: &[HeroRevision]) -> rpc::ListMyHeroesResponse {
    let mut response = rpc::ListMyHeroesResponse {
        heroes: Vec::new(),
        next_cursor: 0,
    };
    for row in rows {
        let summary = summary(row);
        if response.encoded_len() + summary.encoded_len() + 16 > 56 * 1024 {
            response.next_cursor = response.heroes.last().map(|hero| hero.hero_id).unwrap_or(0);
            break;
        }
        response.heroes.push(summary);
    }
    if response.next_cursor == 0 && rows.len() == 100 {
        response.next_cursor = response.heroes.last().map(|hero| hero.hero_id).unwrap_or(0);
    }
    response
}
