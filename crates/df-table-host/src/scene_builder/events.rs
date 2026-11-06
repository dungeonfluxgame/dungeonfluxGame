//! What the scene builder records: one typed event for each change of a scene's build and of its
//! plates, in one place.
//!
//! Stage outputs are stored and audited, so every request of a stage, every stored output, every
//! failure, every Locate round, the card falling due and every job's end is a `SceneBuilderEvent`
//! under the area `SCENE`; the clock's ticks and skips, the playout buffer's deliveries,
//! rejections, swaps and lateness are under `PLATE` (`plate.swapped`, `plate.rejected_epoch` and
//! `plate.dropped_stale` of RES41 and D.2). Each renders as one `AREA verb key=value` line (coding
//! style section 8, pattern 7). An event holds ids, counts, closed keys and milliseconds only:
//! never a description, a phrase, a place name or any player text. A result set aside as stale
//! is the pure crate's own `TableEvent` (`RESULT rejected`), carried here so there is one sink.
//!
//! These are host events because `TableEvent` is a closed enum of the pure crate (the broker does
//! the same). A failure is recorded once, at the boundary that handles it.

use std::fmt::{self, Write};

use df_table::event::{FailureCause, TableEvent};
use df_table::identity::SceneId;
use df_table::lookahead::{CancelReason, Expectation};
use df_table::plate::buffer::{PlateRejectError, Swap, WaitReason};
use df_table::plate::job::{JobReason, PlateJobId};
use df_table::plate::scheduler::{JobEnd, SkipReason};
use df_table::plate::timing::SceneBuildKind;
use df_table::ports::PurposeName;
use df_table::scene_plan::SceneStage;
use df_table::table_state::SceneGeneration;

use super::purposes::BuildPriority;
use super::request::BuildEnd;
use crate::broker::purpose::Fallback;

/// Where a stage's stored output came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageSource {
    /// A broker request made it.
    Requested,
    /// The scene record already held it, so nothing was asked (an authored description).
    SceneRecord,
    /// Nothing was left to ask for (a plate with nobody and nothing to find).
    NothingToAsk,
    /// The look-ahead worker built it before the table entered the scene (SB-LA).
    Lookahead,
}

impl StageSource {
    pub fn key(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::SceneRecord => "scene_record",
            Self::NothingToAsk => "nothing_to_ask",
            Self::Lookahead => "lookahead",
        }
    }
}

/// What Locate decided for one plate, as counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocateCounts {
    pub located_figures: usize,
    pub unlocated_figures: usize,
    pub located_items: usize,
    pub own_picture_items: usize,
    pub unlocated_items: usize,
}

/// One change of a scene's build or of its plates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneBuilderEvent {
    Opened {
        scene: SceneId,
        generation: SceneGeneration,
        kind: SceneBuildKind,
        priority: BuildPriority,
    },
    StageRequested {
        scene: SceneId,
        generation: SceneGeneration,
        job: PlateJobId,
        stage: SceneStage,
        purpose: PurposeName,
        requests: u32,
    },
    StageStored {
        scene: SceneId,
        generation: SceneGeneration,
        job: PlateJobId,
        stage: SceneStage,
        source: StageSource,
        ms: u64,
    },
    StageFailed {
        scene: SceneId,
        generation: SceneGeneration,
        job: PlateJobId,
        stage: SceneStage,
        cause: FailureCause,
        fallback: Fallback,
    },
    LocateRound {
        scene: SceneId,
        generation: SceneGeneration,
        job: PlateJobId,
        round: u32,
        prompts: u32,
    },
    Located {
        scene: SceneId,
        generation: SceneGeneration,
        job: PlateJobId,
        counts: LocateCounts,
    },
    /// The build passed its budget with no plate on screen: play continues on the card (K.2).
    CardDue {
        scene: SceneId,
        generation: SceneGeneration,
        after_ms: u64,
    },
    JobReady {
        scene: SceneId,
        generation: SceneGeneration,
        job: PlateJobId,
        ms: u64,
    },
    JobEnded {
        scene: SceneId,
        generation: SceneGeneration,
        job: PlateJobId,
        end: JobEnd,
    },
    /// A result set aside as stale (D.2): the pure crate's own typed event.
    Table(TableEvent),
    TickQueued {
        scene: SceneId,
        generation: SceneGeneration,
        job: PlateJobId,
        cause: JobReason,
    },
    TickSkipped {
        scene: SceneId,
        reason: SkipReason,
    },
    /// Queued jobs of an earlier generation were thrown away (D.2).
    TickDiscarded {
        scene: SceneId,
        jobs: usize,
    },
    /// The plate queue was full, so its oldest job was dropped (H.7, `plate.dropped_stale`).
    DroppedStale {
        scene: SceneId,
        job: PlateJobId,
    },
    Delivered {
        scene: SceneId,
        generation: SceneGeneration,
        job: PlateJobId,
    },
    /// A plate the buffer would not take (D.2, `plate.rejected_epoch`).
    Rejected {
        scene: SceneId,
        generation: SceneGeneration,
        job: PlateJobId,
        reason: PlateRejectError,
    },
    Swapped {
        scene: SceneId,
        swap: Swap,
    },
    /// A ready plate has not swapped in, and why.
    Waiting {
        scene: SceneId,
        job: Option<u64>,
        reason: WaitReason,
        waited_ms: u64,
    },
    SwapOverdue {
        scene: SceneId,
        waited_ms: u64,
    },
    /// The next plate is past its slot (once for each slot).
    Late {
        scene: SceneId,
        slot_late_ms: u64,
        oldest_outstanding_age_ms: u64,
    },
    /// The "picture catching up" mark started or ended (RES10).
    CatchingUp {
        scene: SceneId,
        on: bool,
        age_ms: u64,
    },
    /// A look-ahead build of the next scene started (SB-LA). `epoch` is the broker epoch its
    /// requests are for.
    LookaheadStarted {
        scene: SceneId,
        token: u64,
        expectation: Expectation,
        likelihood_permille: u16,
        epoch: u64,
    },
    /// A look-ahead stage was stored (`ms` of provider time).
    LookaheadStored {
        scene: SceneId,
        token: u64,
        stage: SceneStage,
        source: StageSource,
        ms: u64,
    },
    /// The description and the background of the next scene are stored.
    LookaheadReady {
        scene: SceneId,
        token: u64,
        ms: u64,
    },
    LookaheadFailed {
        scene: SceneId,
        token: u64,
        stage: SceneStage,
        cause: FailureCause,
    },
    LookaheadCancelled {
        scene: SceneId,
        token: u64,
        reason: CancelReason,
    },
    /// A build whose epoch passed was discarded and nothing of it kept (the lease and epoch check).
    LookaheadDiscarded {
        scene: SceneId,
        token: u64,
        stage: SceneStage,
    },
    /// What the transition found: `hit`, `redress`, `stale`, `late`, `failed` or `miss`;
    /// `ready_for_ms` is how long a built scene had waited.
    LookaheadTaken {
        scene: SceneId,
        outcome: &'static str,
        ready_for_ms: Option<u64>,
    },
    /// A built scene nobody took aged out of the book.
    LookaheadExpired {
        scene: SceneId,
    },
    /// The story's expected scenes were stated again.
    LookaheadTargets {
        scenes: usize,
    },
    LookaheadPaused {
        paused: bool,
    },
    /// A target scripted the look-ahead provider of `stage` to take `latency_ms` from now on.
    LookaheadSlowed {
        stage: SceneStage,
        latency_ms: u64,
    },
    /// A target scripted the look-ahead provider of `stage` to refuse its next call.
    LookaheadRefusing {
        stage: SceneStage,
    },
    /// The look-ahead settings changed to these values.
    LookaheadSettings {
        enabled: bool,
        depth: u32,
        start_s: u32,
        deadline_s: u32,
        book_age_s: u32,
    },
}

impl SceneBuilderEvent {
    pub fn area(&self) -> &'static str {
        match self {
            Self::Opened { .. }
            | Self::StageRequested { .. }
            | Self::StageStored { .. }
            | Self::StageFailed { .. }
            | Self::LocateRound { .. }
            | Self::Located { .. }
            | Self::CardDue { .. }
            | Self::JobReady { .. }
            | Self::JobEnded { .. } => "SCENE",
            Self::LookaheadStarted { .. }
            | Self::LookaheadStored { .. }
            | Self::LookaheadReady { .. }
            | Self::LookaheadFailed { .. }
            | Self::LookaheadCancelled { .. }
            | Self::LookaheadDiscarded { .. }
            | Self::LookaheadTaken { .. }
            | Self::LookaheadExpired { .. }
            | Self::LookaheadTargets { .. }
            | Self::LookaheadPaused { .. }
            | Self::LookaheadSlowed { .. }
            | Self::LookaheadRefusing { .. }
            | Self::LookaheadSettings { .. } => "LOOKAHEAD",
            Self::Table(event) => event.area(),
            Self::TickQueued { .. }
            | Self::TickSkipped { .. }
            | Self::TickDiscarded { .. }
            | Self::DroppedStale { .. }
            | Self::Delivered { .. }
            | Self::Rejected { .. }
            | Self::Swapped { .. }
            | Self::Waiting { .. }
            | Self::SwapOverdue { .. }
            | Self::Late { .. }
            | Self::CatchingUp { .. } => "PLATE",
        }
    }

    /// The whole line, `AREA verb key=value ...`.
    pub fn render_audit_line(&self) -> String {
        if let Self::Table(event) = self {
            return event.render_audit_line();
        }
        let mut line = String::with_capacity(128);
        // Writing into a String cannot fail, so the result carries no information.
        let _ = self.write_audit_line(&mut line);
        line
    }

    fn write_audit_line(&self, out: &mut String) -> fmt::Result {
        write!(out, "{} ", self.area())?;
        match self {
            Self::Opened {
                scene,
                generation,
                kind,
                priority,
            } => write!(
                out,
                "opened scene={scene} generation={generation} kind={} priority={}",
                kind.key(),
                priority.key()
            ),
            Self::StageRequested {
                scene,
                generation,
                job,
                stage,
                purpose,
                requests,
            } => write!(
                out,
                "stage_requested scene={scene} generation={generation} job={job} stage={} purpose={purpose} requests={requests}",
                stage.key()
            ),
            Self::StageStored {
                scene,
                generation,
                job,
                stage,
                source,
                ms,
            } => write!(
                out,
                "stage_stored scene={scene} generation={generation} job={job} stage={} source={} ms={ms}",
                stage.key(),
                source.key()
            ),
            Self::StageFailed {
                scene,
                generation,
                job,
                stage,
                cause,
                fallback,
            } => write!(
                out,
                "stage_failed scene={scene} generation={generation} job={job} stage={} cause={} fallback={}",
                stage.key(),
                cause.key(),
                fallback.key()
            ),
            Self::LocateRound {
                scene,
                generation,
                job,
                round,
                prompts,
            } => write!(
                out,
                "locate_round scene={scene} generation={generation} job={job} round={round} prompts={prompts}"
            ),
            Self::Located {
                scene,
                generation,
                job,
                counts,
            } => write!(
                out,
                "located scene={scene} generation={generation} job={job} figures={} unlocated_figures={} items={} own_picture_items={} unlocated_items={}",
                counts.located_figures,
                counts.unlocated_figures,
                counts.located_items,
                counts.own_picture_items,
                counts.unlocated_items
            ),
            Self::CardDue {
                scene,
                generation,
                after_ms,
            } => write!(
                out,
                "card_due scene={scene} generation={generation} after_ms={after_ms}"
            ),
            Self::JobReady {
                scene,
                generation,
                job,
                ms,
            } => write!(
                out,
                "job_ready scene={scene} generation={generation} job={job} ms={ms}"
            ),
            Self::JobEnded {
                scene,
                generation,
                job,
                end,
            } => write!(
                out,
                "job_ended scene={scene} generation={generation} job={job} end={}",
                BuildEnd::from(*end).key()
            ),
            Self::Table(_) => Ok(()),
            Self::TickQueued {
                scene,
                generation,
                job,
                cause,
            } => write!(
                out,
                "tick_queued scene={scene} generation={generation} job={job} cause={}",
                cause.key()
            ),
            Self::TickSkipped { scene, reason } => {
                write!(out, "tick_skipped scene={scene} reason={}", reason.key())
            }
            Self::TickDiscarded { scene, jobs } => {
                write!(out, "tick_discarded scene={scene} jobs={jobs}")
            }
            Self::DroppedStale { scene, job } => {
                write!(out, "dropped_stale scene={scene} job={job}")
            }
            Self::Delivered {
                scene,
                generation,
                job,
            } => write!(
                out,
                "delivered scene={scene} generation={generation} job={job}"
            ),
            Self::Rejected {
                scene,
                generation,
                job,
                reason,
            } => write!(
                out,
                "rejected_epoch scene={scene} generation={generation} job={job} reason={}",
                reason.key()
            ),
            Self::Swapped { scene, swap } => {
                write!(
                    out,
                    "swapped scene={scene} from={} to={} dissolve_ms={} slot_late_ms={} content_age_ms={}",
                    OptionalNumber(swap.from),
                    swap.to,
                    swap.dissolve_ms,
                    swap.slot_late_ms,
                    swap.content_age_ms
                )?;
                match swap.prior_on_screen_ms {
                    Some(prior) => write!(out, " prior_on_screen_ms={prior}"),
                    None => Ok(()),
                }
            }
            Self::Waiting {
                scene,
                job,
                reason,
                waited_ms,
            } => write!(
                out,
                "waiting scene={scene} job={} reason={} waited_ms={waited_ms}",
                OptionalNumber(*job),
                reason.key()
            ),
            Self::SwapOverdue { scene, waited_ms } => {
                write!(out, "swap_overdue scene={scene} waited_ms={waited_ms}")
            }
            Self::Late {
                scene,
                slot_late_ms,
                oldest_outstanding_age_ms,
            } => write!(
                out,
                "late scene={scene} slot_late_ms={slot_late_ms} oldest_outstanding_age_ms={oldest_outstanding_age_ms}"
            ),
            Self::CatchingUp { scene, on, age_ms } => write!(
                out,
                "catching_up scene={scene} on={} age_ms={age_ms}",
                if *on { "true" } else { "false" }
            ),
            Self::LookaheadStarted {
                scene,
                token,
                expectation,
                likelihood_permille,
                epoch,
            } => write!(
                out,
                "started scene={scene} token={token} expectation={} likelihood_permille={likelihood_permille} epoch={epoch}",
                expectation.key()
            ),
            Self::LookaheadStored {
                scene,
                token,
                stage,
                source,
                ms,
            } => write!(
                out,
                "stage_stored scene={scene} token={token} stage={} source={} ms={ms}",
                stage.key(),
                source.key()
            ),
            Self::LookaheadReady { scene, token, ms } => {
                write!(out, "ready scene={scene} token={token} ms={ms}")
            }
            Self::LookaheadFailed {
                scene,
                token,
                stage,
                cause,
            } => write!(
                out,
                "failed scene={scene} token={token} stage={} cause={}",
                stage.key(),
                cause.key()
            ),
            Self::LookaheadCancelled {
                scene,
                token,
                reason,
            } => write!(
                out,
                "cancelled scene={scene} token={token} reason={}",
                reason.key()
            ),
            Self::LookaheadDiscarded {
                scene,
                token,
                stage,
            } => write!(
                out,
                "discarded scene={scene} token={token} stage={} reason=epoch_moved",
                stage.key()
            ),
            Self::LookaheadTaken {
                scene,
                outcome,
                ready_for_ms,
            } => write!(
                out,
                "taken scene={scene} outcome={outcome} ready_for_ms={}",
                OptionalNumber(*ready_for_ms)
            ),
            Self::LookaheadExpired { scene } => write!(out, "expired scene={scene}"),
            Self::LookaheadTargets { scenes } => write!(out, "targets scenes={scenes}"),
            Self::LookaheadPaused { paused } => write!(
                out,
                "paused paused={}",
                if *paused { "true" } else { "false" }
            ),
            Self::LookaheadSlowed { stage, latency_ms } => {
                write!(out, "slowed stage={} latency_ms={latency_ms}", stage.key())
            }
            Self::LookaheadRefusing { stage } => write!(out, "refusing stage={}", stage.key()),
            Self::LookaheadSettings {
                enabled,
                depth,
                start_s,
                deadline_s,
                book_age_s,
            } => write!(
                out,
                "settings enabled={enabled} depth={depth} start_s={start_s} deadline_s={deadline_s} book_age_s={book_age_s}"
            ),
        }
    }
}

/// A number or the word `none`, so a line always has its key.
struct OptionalNumber(Option<u64>);

impl fmt::Display for OptionalNumber {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(number) => write!(formatter, "{number}"),
            None => formatter.write_str("none"),
        }
    }
}

/// Where the scene builder's events go.
pub trait SceneBuilderAudit: Send + Sync {
    fn record(&self, event: &SceneBuilderEvent);
}
