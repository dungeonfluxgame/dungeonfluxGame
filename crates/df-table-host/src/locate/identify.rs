//! Character identification: which region of a stored layout is which character, asked of the
//! text service with a question the imagery API prepares and checks.

use std::path::Path;

use df_scene_layout::{RegionRef, SceneLayout};

use super::request::{
    IdentifyRequest, IdentityOutcome, IdentityRow, LocateError, QuestionCause, TextServiceCause,
};
use super::scan::relabel;
use super::storage::{current_layout, lock_layout_writes, read_picture, write_layout};
use super::validate::{check_label, check_picture_name};

/// How sure the model must be, in thousandths, for its match to name a region.
pub const DEFAULT_MIN_CONFIDENCE_PERMILLE: u16 = 600;
/// The most a confidence can be, in thousandths: certain.
const FULL_CONFIDENCE_PERMILLE: u16 = 1000;
/// The text-service scope of every identity question this feature asks.
pub const IDENTIFY_SCOPE: &str = "locate";

/// Applies a checked answer to `layout`: a character placed with at least `min_confidence` and no
/// conflict names its region (moving the name off any other region); every other character changes
/// nothing, so a name given by hand stays.
pub fn apply_identification(
    layout: &mut SceneLayout,
    answers: &[df_imagery::identity_question::Identified],
    min_confidence: u16,
) -> Result<Vec<IdentityRow>, LocateError> {
    let mut rows = Vec::new();
    for answer in answers {
        let region = answer.candidate.map(|number| number - 1);
        let outcome = if answer.conflict {
            IdentityOutcome::Conflict
        } else if region.is_none() {
            IdentityOutcome::NotFound
        } else if answer.confidence_permille < min_confidence {
            IdentityOutcome::Unsure
        } else {
            IdentityOutcome::Named
        };
        if let (Some(index), IdentityOutcome::Named) = (region, outcome) {
            relabel(layout, RegionRef::Number(index), Some(&answer.name))?;
        }
        rows.push(IdentityRow {
            name: answer.name.clone(),
            region,
            confidence_permille: answer.confidence_permille,
            outcome,
        });
    }
    Ok(rows)
}

/// Applies a checked answer to the layout of `picture` as it is stored now, under the layout write
/// lock, and stores it: a label stored while the question was asked is kept, and an answer about
/// regions that are no longer there (a new scan) is refused as stale. Blocks on files: call it from
/// a blocking task.
pub fn store_identification(
    media: &Path,
    layouts: &Path,
    picture: &str,
    asked: &SceneLayout,
    answers: &[df_imagery::identity_question::Identified],
    min_confidence: u16,
) -> Result<(SceneLayout, Vec<IdentityRow>), LocateError> {
    let _writing = lock_layout_writes();
    let changed = || LocateError::LayoutChanged {
        picture: picture.to_owned(),
    };
    let mut layout = current_layout(media, layouts, picture)?.ok_or_else(changed)?;
    let same_regions = layout
        .regions
        .iter()
        .map(|region| &region.id)
        .eq(asked.regions.iter().map(|region| &region.id));
    if layout.picture.sha256 != asked.picture.sha256 || !same_regions {
        return Err(changed());
    }
    let rows = apply_identification(&mut layout, answers, min_confidence)?;
    write_layout(layouts, picture, &layout)?;
    Ok((layout, rows))
}

/// The question an identification asks, prepared by the imagery API from the stored layout and
/// the library pictures, with the layout it is about.
pub struct PreparedIdentity {
    pub layout: SceneLayout,
    pub question: df_imagery::identity_question::IdentityQuestion,
    pub min_confidence: u16,
}

/// Reads the layout and pictures and has the imagery API prepare the question. Blocks on files
/// and picture work: call it from a blocking task.
pub fn prepare_identity(
    media: &Path,
    layouts: &Path,
    request: &IdentifyRequest,
) -> Result<PreparedIdentity, LocateError> {
    use df_imagery::identity_question::{self, CandidateBox, Character};
    check_picture_name(&request.picture)?;
    let min_confidence = request
        .min_confidence_permille
        .unwrap_or(DEFAULT_MIN_CONFIDENCE_PERMILLE);
    if min_confidence > FULL_CONFIDENCE_PERMILLE {
        return Err(LocateError::InvalidMinimumConfidence {
            min_confidence_permille: min_confidence,
        });
    }
    if request.characters.is_empty() {
        return Err(LocateError::NoCharacters);
    }
    for character in &request.characters {
        check_label(&character.name)?;
    }
    let layout =
        current_layout(media, layouts, &request.picture)?.ok_or_else(|| LocateError::NoLayout {
            picture: request.picture.clone(),
        })?;
    let read_media = |name: &str| {
        check_picture_name(name)?;
        read_picture(media, name)
    };
    let mut characters = Vec::new();
    for character in &request.characters {
        characters.push(Character {
            name: character.name.clone(),
            description: character.description.clone(),
            portrait: character.portrait.as_deref().map(read_media).transpose()?,
        });
    }
    let question_request = identity_question::IdentifyRequest {
        picture: read_media(&request.picture)?,
        candidates: layout
            .regions
            .iter()
            .map(|region| CandidateBox {
                left_permille: region.left_permille,
                top_permille: region.top_permille,
                right_permille: region.right_permille.max(region.left_permille + 1),
                bottom_permille: region.bottom_permille.max(region.top_permille + 1),
            })
            .collect(),
        characters,
    };
    let question = identity_question::ask(&question_request).map_err(|error| {
        LocateError::QuestionInvalid {
            cause: QuestionCause::from(error),
        }
    })?;
    Ok(PreparedIdentity {
        layout,
        question,
        min_confidence,
    })
}

/// The text request of a prepared question, for the text service.
pub fn identity_request(
    question: &df_imagery::identity_question::IdentityQuestion,
    profile: &df_ai::text::TextProfile,
) -> df_ai::text::TextRequest {
    use df_ai::text::{TextImage, TextImageType, TextPurpose, TextRequest};
    TextRequest {
        purpose: TextPurpose::CharacterIdentity,
        profile: profile.clone(),
        instructions: question.instructions.clone(),
        facts: question.facts.clone(),
        output_name: df_imagery::identity_question::OUTPUT_NAME.to_owned(),
        output_schema: question.output_schema.clone(),
        deadline: crate::text::IDENTITY_DEADLINE,
        images: question
            .pictures
            .iter()
            .map(|picture| TextImage {
                label: picture.label.clone(),
                media_type: TextImageType::Jpeg,
                bytes: picture.jpeg.clone(),
            })
            .collect(),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// What the text service gave for an identity question: the checked answer and whether it was a
/// replay of an answer the service had already qualified.
pub struct IdentityAnswer {
    pub found: Vec<df_imagery::identity_question::Identified>,
    pub replayed: bool,
}

/// The text-service failure `cause`, with the service's own account of it when it gave one.
fn text_service(cause: TextServiceCause, detail: Option<String>) -> LocateError {
    LocateError::TextService { cause, detail }
}

/// Asks the text service the prepared question and checks the answer with the imagery API: a
/// candidate that passes is qualified, one that fails is rejected (and never asked again under the
/// same question); an answer the service already qualified replays without a call.
pub async fn ask_identity(
    text: &crate::text::TextRuntime,
    prepared: &PreparedIdentity,
) -> Result<IdentityAnswer, LocateError> {
    use df_ai::text::TextOutcome;
    use df_ai::text_attempts::{Submission, fingerprint};
    use df_imagery::identity_question::read_answer;
    if let Some(missing) = text.missing() {
        return Err(LocateError::TextServiceMissing {
            missing: missing.to_owned(),
        });
    }
    let request = identity_request(&prepared.question, text.identity_profile());
    // The same question about the same pictures is the same operation: its answer replays.
    let operation = format!("identify:{}", &hex(&fingerprint(&request))[..32]);
    let key = crate::text::TextRuntime::key(IDENTIFY_SCOPE, &operation)
        .ok_or(LocateError::InvalidOperationKey)?;
    let submitted = text
        .service()
        .submit(&key, &request)
        .await
        .map_err(|error| text_service(TextServiceCause::Refused, Some(format!("{error:?}"))))?;
    match submitted {
        Submission::Candidate { answer, .. } => {
            qualify_candidate(text, &key, &request, &answer, prepared).await
        }
        Submission::Settled {
            outcome: TextOutcome::Answered(answer),
            replayed,
            ..
        } => read_answer(&answer, &prepared.question)
            .map(|found| IdentityAnswer { found, replayed })
            .map_err(|error| {
                text_service(TextServiceCause::ReplayRefused, Some(error.to_string()))
            }),
        Submission::Settled { outcome, .. } => Err(text_service(
            TextServiceCause::NoAnswer,
            Some(format!("{outcome:?}")),
        )),
        Submission::Rejected { .. } => Err(text_service(TextServiceCause::RejectedBefore, None)),
        Submission::InFlight => Err(text_service(TextServiceCause::InFlight, None)),
        Submission::Unknown => Err(text_service(TextServiceCause::Lost, None)),
        Submission::Cancelled => Err(text_service(TextServiceCause::Cancelled, None)),
    }
}

/// Checks a fresh answer with the imagery API and tells the text service whether to keep it.
async fn qualify_candidate(
    text: &crate::text::TextRuntime,
    key: &df_ai::text_attempts::OperationKey,
    request: &df_ai::text::TextRequest,
    answer: &serde_json::Value,
    prepared: &PreparedIdentity,
) -> Result<IdentityAnswer, LocateError> {
    use df_ai::text_attempts::{Qualification, Verdict};
    use df_imagery::identity_question::read_answer;
    match read_answer(answer, &prepared.question) {
        Ok(found) => match text
            .service()
            .qualify(key, request, Verdict::Accepted)
            .await
        {
            Ok(Qualification::Qualified) => Ok(IdentityAnswer {
                found,
                replayed: false,
            }),
            other => Err(text_service(
                TextServiceCause::NotKept,
                Some(format!("{other:?}")),
            )),
        },
        Err(error) => {
            // The rejection only stops this answer from being reused; the error below reports
            // the failure either way, and the service records the verdict event.
            let _ = text
                .service()
                .qualify(key, request, Verdict::Rejected)
                .await;
            Err(text_service(
                TextServiceCause::AnswerRefused,
                Some(error.to_string()),
            ))
        }
    }
}
