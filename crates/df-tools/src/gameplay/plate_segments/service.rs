//! The hub adapter of plate segmentation: the two resources a plate's people are made of.
//!
//! * `PlateSegments` ([`ResourceKind::PlateSegments`]): the hosted SAM 3.1 answer for the noun
//!   phrases `person` and `head`, one provider call each, kept as a note beside the cached plate.
//!   Keyed by the plate's image key, the endpoint, the model and the prompt set.
//! * `PlateStaging` ([`ResourceKind::PlateStaging`]): who each SAM person is and which way they
//!   face. Located heads are matched by overlap; the vision judge answers the rest with closed
//!   choices; one batched facing call labels everyone. Keyed by the segments' key, the pinned
//!   reference portraits and the located boxes it was made from.
//!
//! Both go through [`Kit::produce`], so the hub owns the registry, the single flight, the failure
//! memory, the timeout, the `RESOURCE` audit lines and the spend ledger. Generation has no spend
//! cap: every sent call is recorded ([`Unit::SegmentCall`], [`Unit::TextToken`]) and never
//! refused. A failure leaves the caller on the head locator's layout.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use df_providers::{
    FalSegmentError, JudgePicture, MaskRows, MaskRun, NativeFalSegmenter, NativePlateJudge,
    PlateJudgeError, SegmentInstance, SegmentModel, SegmentRequest,
};
use df_render::art_contract::Neutral;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{
    candidate_label,
    crops::{candidate_crops, prepare_plate},
    facing::{Facing, facing_request, read_facing},
    geometry::{SegmentedPerson, people_from_segments},
    identity::{LocatedPerson, identity_request, match_locator, read_identity},
    reference_label,
};
use crate::gameplay::{
    actor::record_audit_line,
    cost,
    resource_adapters::{Eraser, Flights, Kit, Outcome, Produced, hex, slug, write_atomic},
    resources::{
        Chrome, Generated, Invalidate, Limits, ResourceFailure, ResourceKey, ResourceKind,
        ResourceRow, ResourceSpec, Unit,
    },
};

const PROMPTS: [&str; 2] = ["person", "head"];
const MAXIMUM_MASKS: u8 = 16;
const SEGMENT_DEADLINE: Duration = Duration::from_secs(60);
const JUDGE_DEADLINE: Duration = Duration::from_secs(90);
/// What a vision-judge call is counted as before its provider usage is known.
const JUDGE_MODEL: &str = "gpt-5.5";

/// The `PlateSegments` spec: what was asked of the hosted model about which plate.
#[derive(Clone, Debug)]
struct PlateSegmentsSpec {
    image_key: String,
    model: SegmentModel,
}

impl ResourceSpec for PlateSegmentsSpec {
    const KIND: ResourceKind = ResourceKind::PlateSegments;
    const CONTRACT_VERSION: u32 = 1;

    fn canonical(&self) -> String {
        json!({
            "image": self.image_key,
            "endpoint": self.model.endpoint(),
            "model": self.model.label(),
            "prompts": PROMPTS,
            "maximum_masks": MAXIMUM_MASKS,
            "output": "rle",
        })
        .to_string()
    }

    fn subject(&self) -> String {
        format!("plate:{}", slug(&self.image_key, 64))
    }

    fn chrome(&self) -> Chrome {
        Neutral::Nothing
    }
}

/// The `PlateStaging` spec: the segments it reads and everything the judges are shown.
#[derive(Clone, Debug)]
struct PlateStagingSpec {
    image_key: String,
    segments_key: String,
    /// The reference portraits, by id and the SHA-256 of their JPEG.
    references: Vec<(String, String)>,
    /// The located boxes, by id, to the thousandth.
    located: Vec<(String, [u32; 4])>,
}

impl ResourceSpec for PlateStagingSpec {
    const KIND: ResourceKind = ResourceKind::PlateStaging;
    const CONTRACT_VERSION: u32 = 1;

    fn canonical(&self) -> String {
        json!({
            "image": self.image_key,
            "segments": self.segments_key,
            "references": self.references,
            "located": self.located,
            "judge": JUDGE_MODEL,
        })
        .to_string()
    }

    fn subject(&self) -> String {
        format!("plate:{}", slug(&self.image_key, 64))
    }

    fn chrome(&self) -> Chrome {
        Neutral::Nothing
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct StoredInstance {
    index: u32,
    score: f32,
    /// `[start, length]` pairs, 0-based over the row-major picture.
    runs: Vec<[u32; 2]>,
}

impl StoredInstance {
    fn of(found: &SegmentInstance) -> Self {
        Self {
            index: found.index,
            score: found.score,
            runs: found
                .mask
                .runs()
                .iter()
                .map(|run| [run.start, run.length])
                .collect(),
        }
    }

    fn instance(&self, width: u32, height: u32) -> Option<SegmentInstance> {
        let runs = self
            .runs
            .iter()
            .map(|[start, length]| MaskRun {
                start: *start,
                length: *length,
            })
            .collect();
        Some(SegmentInstance {
            index: self.index,
            score: self.score,
            mask: MaskRows::from_runs(width, height, runs).ok()?,
        })
    }
}

/// What the hosted model answered, as kept beside the cached plate.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct SegmentsNote {
    picture: [u32; 2],
    model: String,
    #[serde(default)]
    micro_dollars: Option<u64>,
    person_ms: u64,
    head_ms: u64,
    person: Vec<StoredInstance>,
    head: Vec<StoredInstance>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct StoredWho {
    candidate: usize,
    /// An actor id or `hero-<art>`; `None` for a person nobody was matched to.
    who: Option<String>,
    /// `locator` or `judge`.
    source: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct StoredFacing {
    candidate: usize,
    facing: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct StagingNote {
    assignments: Vec<StoredWho>,
    facings: Vec<StoredFacing>,
    identity_ms: u64,
    facing_ms: u64,
    /// Provider-reported input and output tokens of the judge calls.
    tokens: [u64; 2],
    identity_error: Option<String>,
    facing_error: Option<String>,
}

/// One person of a plate: the SAM geometry, who they were matched to and which way they face.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::gameplay) struct AnalysedPerson {
    /// An actor id or `hero-<art>`; `None` when unidentified.
    pub(in crate::gameplay) who: Option<String>,
    pub(in crate::gameplay) person: SegmentedPerson,
    pub(in crate::gameplay) facing: Option<Facing>,
}

/// Everyone SAM found on one plate, identified and facing.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::gameplay) struct Analysis {
    pub(in crate::gameplay) image_key: String,
    pub(in crate::gameplay) picture: (u32, u32),
    pub(in crate::gameplay) people: Vec<AnalysedPerson>,
    pub(in crate::gameplay) sam_micro_dollars: Option<u64>,
}

/// What one analysis is made from. The plate is the cached image (any format); the references
/// are the pinned portraits as JPEGs with their SHA-256; located boxes are the head locator's,
/// by the same ids as the references.
pub(in crate::gameplay) struct AnalysisInput {
    pub(in crate::gameplay) image_key: String,
    pub(in crate::gameplay) plate: Arc<Vec<u8>>,
    pub(in crate::gameplay) references: Vec<(String, Arc<Vec<u8>>, String)>,
    pub(in crate::gameplay) located: Vec<LocatedPerson>,
}

/// Why no analysis came back; the caller keeps the head locator's layout.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct AnalysisFailure(pub(in crate::gameplay) &'static str);

struct Inner {
    kit: Kit,
    segmenter: NativeFalSegmenter,
    judge: Option<NativePlateJudge>,
    directory: PathBuf,
    segment_flights: Flights<Outcome<Arc<SegmentsNote>>>,
    staging_flights: Flights<Outcome<Arc<StagingNote>>>,
}

/// The plate segmentation service: a cheap handle over the generation hub.
#[derive(Clone)]
pub(in crate::gameplay) struct Segmentation {
    inner: Arc<Inner>,
}

fn note_name(prefix: &str, key: &ResourceKey) -> String {
    format!("{prefix}-{}.json", key.as_str())
}

fn read_note<T: for<'de> Deserialize<'de>>(directory: &Path, name: &str) -> Option<T> {
    let bytes = std::fs::read(directory.join(name)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn sam_failure(failure: FalSegmentError) -> ResourceFailure {
    if failure.is_transient() {
        ResourceFailure::Transient(failure.label())
    } else {
        ResourceFailure::Permanent(failure.label())
    }
}

fn judge_failure(failure: PlateJudgeError) -> &'static str {
    failure.label()
}

impl Segmentation {
    /// A service over `kit`, writing its notes to `directory` (the imagery cache). `judge` may be
    /// absent: nobody is identified or labelled then, and the layout stays the locator's.
    pub(in crate::gameplay) fn new(
        kit: Kit,
        segmenter: NativeFalSegmenter,
        judge: Option<NativePlateJudge>,
        directory: PathBuf,
    ) -> Self {
        let remove = |directory: PathBuf| -> Eraser {
            Arc::new(move |generated: &Generated| {
                // Best effort: a note that cannot be removed is asked for again anyway.
                let _removed = std::fs::remove_file(directory.join(&generated.handle));
            })
        };
        let exists = |directory: PathBuf| -> Arc<dyn Fn(&Generated) -> bool + Send + Sync> {
            Arc::new(move |generated: &Generated| directory.join(&generated.handle).is_file())
        };
        kit.tune(
            ResourceKind::PlateSegments,
            Limits::for_kind(ResourceKind::PlateSegments),
            exists(directory.clone()),
            remove(directory.clone()),
        );
        kit.tune(
            ResourceKind::PlateStaging,
            Limits::for_kind(ResourceKind::PlateStaging),
            exists(directory.clone()),
            remove(directory.clone()),
        );
        Self {
            inner: Arc::new(Inner {
                kit,
                segmenter,
                judge,
                directory,
                segment_flights: Flights::default(),
                staging_flights: Flights::default(),
            }),
        }
    }

    /// The model this service calls.
    pub(in crate::gameplay) fn model(&self) -> SegmentModel {
        self.inner.segmenter.model()
    }

    /// Segments the plate, identifies and labels everyone on it, and returns the result. A plate
    /// already analysed is read from its notes without a provider call.
    pub(in crate::gameplay) async fn analyse(
        &self,
        input: AnalysisInput,
    ) -> Result<Analysis, AnalysisFailure> {
        let started = Instant::now();
        let model = self.model();
        let segments_spec = PlateSegmentsSpec {
            image_key: input.image_key.clone(),
            model,
        };
        let segments_key = ResourceKey::of(&segments_spec);
        let (segments, _) = self
            .inner
            .kit
            .produce(
                &self.inner.segment_flights,
                segments_spec,
                self.clone()
                    .segments_work(input.plate.clone(), segments_key.clone()),
            )
            .await;
        let segments = self
            .note_of::<SegmentsNote>("seg", &segments_key, segments)
            .map_err(|failure| AnalysisFailure(failure.label()))?;
        let (width, height) = (segments.picture[0], segments.picture[1]);
        let to_instances = |stored: &[StoredInstance]| -> Option<Vec<SegmentInstance>> {
            stored
                .iter()
                .map(|found| found.instance(width, height))
                .collect()
        };
        let (Some(person_instances), Some(head_instances)) =
            (to_instances(&segments.person), to_instances(&segments.head))
        else {
            return Err(AnalysisFailure("bad_note"));
        };
        let people = people_from_segments(&person_instances, &head_instances);

        let staging_spec = PlateStagingSpec {
            image_key: input.image_key.clone(),
            segments_key: segments_key.as_str().to_owned(),
            references: input
                .references
                .iter()
                .map(|(id, _, sha)| (id.clone(), sha.clone()))
                .collect(),
            located: input
                .located
                .iter()
                .map(|found| {
                    let thousandth = |value: f32| (value.clamp(0.0, 1.0) * 1000.0).round() as u32;
                    (
                        found.id.clone(),
                        [
                            thousandth(found.head.left),
                            thousandth(found.head.top),
                            thousandth(found.head.right),
                            thousandth(found.head.bottom),
                        ],
                    )
                })
                .collect(),
        };
        let staging_key = ResourceKey::of(&staging_spec);
        let (staging, _) = self
            .inner
            .kit
            .produce(
                &self.inner.staging_flights,
                staging_spec,
                self.clone().staging_work(
                    staging_key.clone(),
                    people.clone(),
                    input.plate.clone(),
                    input.references.clone(),
                    input.located.clone(),
                ),
            )
            .await;
        let staging = self
            .note_of::<StagingNote>("stg", &staging_key, staging)
            .map_err(|failure| AnalysisFailure(failure.label()))?;

        let analysed: Vec<AnalysedPerson> = people
            .into_iter()
            .map(|person| {
                let who = staging
                    .assignments
                    .iter()
                    .find(|stored| stored.candidate == person.candidate)
                    .and_then(|stored| stored.who.clone());
                let facing = staging
                    .facings
                    .iter()
                    .find(|stored| stored.candidate == person.candidate)
                    .and_then(|stored| stored.facing.as_deref())
                    .and_then(|label| {
                        [
                            Facing::Left,
                            Facing::Right,
                            Facing::TowardCamera,
                            Facing::AwayFromCamera,
                        ]
                        .into_iter()
                        .find(|facing| facing.label() == label)
                    });
                AnalysedPerson {
                    who,
                    person,
                    facing,
                }
            })
            .collect();
        let analysis = Analysis {
            image_key: input.image_key,
            picture: (width, height),
            people: analysed,
            sam_micro_dollars: segments.micro_dollars,
        };
        record_staging_line(&analysis, &segments, &staging, started.elapsed());
        Ok(analysis)
    }

    /// The note of a produced resource: the fresh payload, or the stored note its handle names.
    fn note_of<T: for<'de> Deserialize<'de> + Clone>(
        &self,
        prefix: &str,
        key: &ResourceKey,
        outcome: Outcome<Arc<T>>,
    ) -> Result<Arc<T>, ResourceFailure> {
        let produced = outcome?;
        match produced.payload {
            Some(note) => Ok(note),
            None => read_note::<T>(&self.inner.directory, &note_name(prefix, key))
                .map(Arc::new)
                .ok_or(ResourceFailure::Permanent("note_unreadable")),
        }
    }

    /// The two hosted calls for one plate, unless its note is already stored.
    async fn segments_work(
        self,
        plate: Arc<Vec<u8>>,
        key: ResourceKey,
    ) -> Outcome<Arc<SegmentsNote>> {
        let name = note_name("seg", &key);
        if let Some(note) = read_note::<SegmentsNote>(&self.inner.directory, &name) {
            return Ok(Produced {
                generated: Generated::new(name),
                payload: Some(Arc::new(note)),
                from_store: true,
            });
        }
        let prepared = {
            let plate = plate.clone();
            tokio::task::spawn_blocking(move || prepare_plate(&plate))
                .await
                .map_err(|_| ResourceFailure::Interrupted)?
                .map_err(|_| ResourceFailure::Permanent("plate_unreadable"))?
        };
        let model = self.model();
        let request = |prompt: &'static str| SegmentRequest {
            jpeg: &prepared.jpeg,
            width: prepared.width,
            height: prepared.height,
            prompt,
            max_masks: MAXIMUM_MASKS,
        };
        let (person_request, head_request) = (request(PROMPTS[0]), request(PROMPTS[1]));
        // Both calls are recorded as spend before they are sent and kept whatever the outcome;
        // each is metered in dollars on the cost book and settled by its own result.
        self.inner.kit.charge(Unit::SegmentCall, 2);
        let ledger = self.inner.kit.hub().ledger();
        let person_billed =
            ledger.begin_call(cost::segment_call(model.endpoint(), PROMPTS[0]), &[]);
        let head_billed = ledger.begin_call(cost::segment_call(model.endpoint(), PROMPTS[1]), &[]);
        let (person, head) = tokio::join!(
            self.inner
                .segmenter
                .segment(&person_request, SEGMENT_DEADLINE),
            self.inner
                .segmenter
                .segment(&head_request, SEGMENT_DEADLINE)
        );
        for (billed, outcome) in [(person_billed, &person), (head_billed, &head)] {
            match outcome {
                Ok(_) => cost::settle_counted(billed, vec![(cost::Category::Calls, 1)]),
                Err(error) => {
                    cost::settle_failure(billed, cost::segment_error_may_be_billed(*error));
                }
            }
        }
        let mut sent = 2i64;
        for outcome in [&person, &head] {
            if outcome.as_ref().is_err_and(|failure| !failure.was_sent()) {
                sent -= 1;
            }
        }
        if sent < 2 {
            self.inner.kit.adjust(Unit::SegmentCall, sent - 2);
        }
        let person = person.map_err(sam_failure)?;
        let head = head.map_err(sam_failure)?;
        let note = SegmentsNote {
            picture: [prepared.width, prepared.height],
            model: model.label().to_owned(),
            micro_dollars: ledger.costs().quote(
                cost::Provider::Fal,
                model.endpoint(),
                &[(cost::Category::Calls, 2)],
            ),
            person_ms: person.elapsed.as_millis() as u64,
            head_ms: head.elapsed.as_millis() as u64,
            person: person.instances.iter().map(StoredInstance::of).collect(),
            head: head.instances.iter().map(StoredInstance::of).collect(),
        };
        self.store(&name, &note).await?;
        Ok(Produced {
            generated: Generated {
                handle: name,
                note: format!(
                    "people={} heads={} usd_micro={}",
                    note.person.len(),
                    note.head.len(),
                    note.micro_dollars
                        .map_or_else(|| "unknown".to_owned(), |price| price.to_string())
                ),
            },
            payload: Some(Arc::new(note)),
            from_store: false,
        })
    }

    async fn store<T: Serialize>(&self, name: &str, note: &T) -> Result<(), ResourceFailure> {
        let bytes = serde_json::to_vec(note).map_err(|_| ResourceFailure::Permanent("encode"))?;
        let directory = self.inner.directory.clone();
        let name = name.to_owned();
        tokio::task::spawn_blocking(move || write_atomic(&directory, &name, &bytes))
            .await
            .map_err(|_| ResourceFailure::Interrupted)?
            .map_err(|_| ResourceFailure::Transient("note_write"))
    }

    /// Identity and facing for the people SAM found.
    async fn staging_work(
        self,
        key: ResourceKey,
        people: Vec<SegmentedPerson>,
        plate: Arc<Vec<u8>>,
        references: Vec<(String, Arc<Vec<u8>>, String)>,
        located: Vec<LocatedPerson>,
    ) -> Outcome<Arc<StagingNote>> {
        let name = note_name("stg", &key);
        if let Some(note) = read_note::<StagingNote>(&self.inner.directory, &name) {
            return Ok(Produced {
                generated: Generated::new(name),
                payload: Some(Arc::new(note)),
                from_store: true,
            });
        }
        let mut assignments: Vec<StoredWho> = Vec::new();
        for found in match_locator(&people, &located) {
            assignments.push(StoredWho {
                candidate: found.candidate,
                who: Some(found.id),
                source: "locator".to_owned(),
            });
        }
        let crops = {
            let plate = plate.clone();
            let shown = people.clone();
            tokio::task::spawn_blocking(move || {
                let refs: Vec<&SegmentedPerson> = shown.iter().collect();
                candidate_crops(&plate, &refs)
            })
            .await
            .map_err(|_| ResourceFailure::Interrupted)?
            .map_err(|_| ResourceFailure::Permanent("plate_unreadable"))?
        };

        // Identity: only the references the locator did not place, over the people it did not.
        let open_references: Vec<&(String, Arc<Vec<u8>>, String)> = references
            .iter()
            .filter(|(id, _, _)| {
                !assignments
                    .iter()
                    .any(|stored| stored.who.as_deref() == Some(id.as_str()))
            })
            .collect();
        let open_candidates: Vec<usize> = people
            .iter()
            .map(|person| person.candidate)
            .filter(|candidate| {
                !assignments
                    .iter()
                    .any(|stored| stored.candidate == *candidate)
            })
            .collect();
        let mut tokens = [0u64; 2];
        let mut identity_ms = 0;
        let mut identity_error = None;
        if let (Some(judge), false, false) = (
            self.inner.judge.as_ref(),
            open_references.is_empty(),
            open_candidates.is_empty(),
        ) && let Some(request) = identity_request(open_references.len(), open_candidates.len())
        {
            // The candidates are renumbered 1..n for this question only.
            let reference_pictures: Vec<(String, &[u8])> = open_references
                .iter()
                .enumerate()
                .map(|(index, (_, jpeg, _))| (reference_label(index + 1), jpeg.as_slice()))
                .collect();
            let candidate_pictures: Vec<(String, &[u8])> = open_candidates
                .iter()
                .enumerate()
                .filter_map(|(index, candidate)| {
                    let (_, jpeg) = crops
                        .iter()
                        .find(|(label, _)| *label == candidate_label(*candidate))?;
                    Some((candidate_label(index + 1), jpeg.as_slice()))
                })
                .collect();
            let pictures: Vec<JudgePicture<'_>> = reference_pictures
                .iter()
                .chain(candidate_pictures.iter())
                .map(|(label, jpeg)| JudgePicture { label, jpeg })
                .collect();
            let asked = Instant::now();
            match judge
                .judge(
                    &request.instruction,
                    &pictures,
                    request.schema_name,
                    &request.schema,
                    JUDGE_DEADLINE,
                )
                .await
            {
                Ok(answer) => {
                    identity_ms = asked.elapsed().as_millis() as u64;
                    self.charge_tokens(answer.input_tokens, answer.output_tokens, &mut tokens);
                    match read_identity(&answer.value, open_references.len(), open_candidates.len())
                    {
                        Ok(read) => {
                            for (reference, pick) in read.assignments.iter().enumerate() {
                                if let Some(number) = pick {
                                    assignments.push(StoredWho {
                                        candidate: open_candidates[number - 1],
                                        who: Some(open_references[reference].0.clone()),
                                        source: "judge".to_owned(),
                                    });
                                }
                            }
                        }
                        Err(_) => identity_error = Some("bad_answer".to_owned()),
                    }
                }
                Err(failure) => {
                    identity_error = Some(judge_failure(failure).to_owned());
                    if matches!(
                        failure,
                        PlateJudgeError::Transport(_) | PlateJudgeError::Provider(_)
                    ) {
                        // A transient fault: ask again later rather than keep a plate whose
                        // people were never identified.
                        return Err(ResourceFailure::Transient(judge_failure(failure)));
                    }
                }
            }
        }
        for person in &people {
            if !assignments
                .iter()
                .any(|stored| stored.candidate == person.candidate)
            {
                assignments.push(StoredWho {
                    candidate: person.candidate,
                    who: None,
                    source: "none".to_owned(),
                });
            }
        }
        assignments.sort_by_key(|stored| stored.candidate);

        // Facing: one batched call over everyone SAM found.
        let numbers: Vec<usize> = people.iter().map(|person| person.candidate).collect();
        let mut facings: Vec<StoredFacing> = Vec::new();
        let mut facing_ms = 0;
        let mut facing_error = None;
        if let (Some(judge), Some(request)) = (self.inner.judge.as_ref(), facing_request(&numbers))
        {
            let pictures: Vec<JudgePicture<'_>> = crops
                .iter()
                .map(|(label, jpeg)| JudgePicture { label, jpeg })
                .collect();
            let asked = Instant::now();
            match judge
                .judge(
                    &request.instruction,
                    &pictures,
                    request.schema_name,
                    &request.schema,
                    JUDGE_DEADLINE,
                )
                .await
            {
                Ok(answer) => {
                    facing_ms = asked.elapsed().as_millis() as u64;
                    self.charge_tokens(answer.input_tokens, answer.output_tokens, &mut tokens);
                    match read_facing(&answer.value, &numbers) {
                        Ok(read) => {
                            facings = numbers
                                .iter()
                                .zip(read)
                                .map(|(candidate, facing)| StoredFacing {
                                    candidate: *candidate,
                                    facing: facing.map(|facing| facing.label().to_owned()),
                                })
                                .collect();
                        }
                        Err(_) => facing_error = Some("bad_answer".to_owned()),
                    }
                }
                Err(failure) => {
                    facing_error = Some(judge_failure(failure).to_owned());
                    if matches!(
                        failure,
                        PlateJudgeError::Transport(_) | PlateJudgeError::Provider(_)
                    ) {
                        return Err(ResourceFailure::Transient(judge_failure(failure)));
                    }
                }
            }
        }
        let note = StagingNote {
            assignments,
            facings,
            identity_ms,
            facing_ms,
            tokens,
            identity_error,
            facing_error,
        };
        self.store(&name, &note).await?;
        Ok(Produced {
            generated: Generated {
                handle: name,
                note: format!(
                    "identified={} tokens={}+{}",
                    note.assignments
                        .iter()
                        .filter(|stored| stored.who.is_some())
                        .count(),
                    tokens[0],
                    tokens[1]
                ),
            },
            payload: Some(Arc::new(note)),
            from_store: false,
        })
    }

    /// Records a judge call's provider-reported tokens as spend.
    fn charge_tokens(&self, input: Option<u64>, output: Option<u64>, total: &mut [u64; 2]) {
        let (input, output) = (input.unwrap_or(0), output.unwrap_or(0));
        total[0] += input;
        total[1] += output;
        self.inner.kit.charge(Unit::TextToken, input + output);
    }

    /// The hub rows of this service's kinds, for the dev tools.
    pub(in crate::gameplay) fn rows(&self) -> Vec<ResourceRow> {
        let hub = self.inner.kit.hub();
        let mut rows = hub.list(Some(ResourceKind::PlateSegments));
        rows.extend(hub.list(Some(ResourceKind::PlateStaging)));
        rows
    }

    /// The spend of this service so far: sent segmentation calls and judge tokens.
    pub(in crate::gameplay) fn spend(&self) -> Value {
        let ledger = self.inner.kit.hub().ledger();
        let calls = ledger.snapshot(Unit::SegmentCall);
        let tokens = ledger.snapshot(Unit::TextToken);
        json!({
            "segment_calls": calls.committed,
            "segment_micro_dollars": ledger
                .costs()
                .quote(
                    cost::Provider::Fal,
                    self.model().endpoint(),
                    &[(cost::Category::Calls, calls.committed)],
                ),
            "judge_tokens": tokens.committed,
            "model": self.model().label(),
        })
    }

    /// Drops this service's entries for the plate `image_key` and erases their notes, so the next
    /// need asks again. Returns how many entries were dropped.
    pub(in crate::gameplay) fn invalidate_plate(&self, image_key: &str) -> usize {
        self.inner.kit.invalidate(Invalidate::Subject(&format!(
            "plate:{}",
            slug(image_key, 64)
        )))
    }

    /// The stored note of a plate's segments, for the dev tools: the SAM answer as it was kept.
    pub(in crate::gameplay) fn segments_report(
        &self,
        image_key: &str,
        model: SegmentModel,
    ) -> Value {
        let key = ResourceKey::of(&PlateSegmentsSpec {
            image_key: image_key.to_owned(),
            model,
        });
        match read_note::<SegmentsNote>(&self.inner.directory, &note_name("seg", &key)) {
            Some(note) => json!({
                "key": key.as_str(),
                "model": note.model,
                "picture": note.picture,
                "micro_dollars": note.micro_dollars,
                "person_ms": note.person_ms,
                "head_ms": note.head_ms,
                "people": note.person.iter().map(|found| json!({"index": found.index, "score": found.score, "runs": found.runs.len()})).collect::<Vec<_>>(),
                "heads": note.head.iter().map(|found| json!({"index": found.index, "score": found.score, "runs": found.runs.len()})).collect::<Vec<_>>(),
            }),
            None => json!({"key": key.as_str(), "status": "no note"}),
        }
    }
}

/// One `PLATE-STAGING` audit line per analysed plate.
fn record_staging_line(
    analysis: &Analysis,
    segments: &SegmentsNote,
    staging: &StagingNote,
    elapsed: Duration,
) {
    let identified = analysis
        .people
        .iter()
        .filter(|found| found.who.is_some())
        .count();
    let facing = analysis
        .people
        .iter()
        .filter(|found| found.facing.is_some())
        .count();
    record_audit_line(
        "gameplay.plate_staging",
        format!(
            "PLATE-STAGING image={} people={} identified={identified} facing={facing} \
             sam_ms={} identity_ms={} facing_ms={} usd_micro={} judge_tokens={}+{} identity_error={} \
             facing_error={} ms={}",
            analysis.image_key,
            analysis.people.len(),
            segments.person_ms.max(segments.head_ms),
            staging.identity_ms,
            staging.facing_ms,
            segments
                .micro_dollars
                .map_or_else(|| "unknown".to_owned(), |price| price.to_string()),
            staging.tokens[0],
            staging.tokens[1],
            staging.identity_error.as_deref().unwrap_or("none"),
            staging.facing_error.as_deref().unwrap_or("none"),
            elapsed.as_millis()
        ),
    );
}

/// The SHA-256 of `bytes` as lowercase hexadecimal (a reference portrait's identity).
pub(in crate::gameplay) fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_segment_price_is_kept_unknown_in_the_note() {
        let quote = cost::PriceTable::default().quote(
            cost::Provider::Fal,
            "fal-ai/sam-3.1",
            &[(cost::Category::Calls, 2)],
        );
        assert_eq!(quote, None);

        let note = SegmentsNote {
            picture: [1, 1],
            model: "sam".to_owned(),
            micro_dollars: quote,
            person_ms: 0,
            head_ms: 0,
            person: Vec::new(),
            head: Vec::new(),
        };
        assert_eq!(
            serde_json::to_value(note).unwrap()["micro_dollars"],
            Value::Null
        );
    }
}
