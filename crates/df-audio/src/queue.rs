use std::collections::VecDeque;
use std::fmt;
use std::rc::Rc;
use std::time::Duration;

use crate::{PcmSource, PlaybackBasis, PlaybackCue, PlaybackOutput};
use df_assets::AssetManifest;
use df_types::ClientBindingId;

use crate::pcm::MAX_SAMPLE_BYTES;
use crate::{PcmBuffer, PcmFormat};

/// These prerequisites remain absent in the current contracts. This is a queue API,
/// not an AudioPlayer implementation that fabricates playable production speech.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MissingPlaybackPrerequisite {
    ApprovedCueTimelineAndCodec,
    CurrentAuthorizedDecoder,
    OwnedBrowserPlaybackAdapter,
}

pub const REQUIRED_PLAYBACK_PREREQUISITES: &[MissingPlaybackPrerequisite] = &[
    MissingPlaybackPrerequisite::ApprovedCueTimelineAndCodec,
    MissingPlaybackPrerequisite::CurrentAuthorizedDecoder,
    MissingPlaybackPrerequisite::OwnedBrowserPlaybackAdapter,
];

/// Caller-selected finite admission bounds, not measured device capacity promises.
/// Retained buffers/bytes include the one dispatched buffer until end/stop acknowledgement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueueLimits {
    pub retained_buffers: usize,
    pub sample_bytes: usize,
    pub cue_chunks: u64,
    pub cue_frames: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueError {
    InvalidLimits,
    Allocation,
    InvalidLease,
    WrongLease,
    Locked,
    WrongBasis,
    InvalidGeneration,
    StaleGeneration,
    DuplicateCue,
    WrongAsset,
    InvalidFormat,
    InvalidSamples,
    BufferCapacity,
    SampleCapacity,
    CueCapacity,
    WrongSequence,
    WrongOffset,
    FrameOverflow,
    ForeignOwner,
    StaleReceipt,
    StaleCallback,
    WaitingForStop,
    Busy,
    Closed,
    Incomplete,
    StopNotRequested,
    Disposed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueState {
    Idle,
    Accepting,
    Draining,
    Drained,
    Cancelled,
    WaitingForStop,
    Disposed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueueSnapshot {
    pub state: QueueState,
    pub retained_buffers: usize,
    pub sample_bytes: usize,
    pub dispatched: bool,
    pub admission_closed: bool,
}

/// Cloneable callback receipt, privately fenced to one owner and cue instance.
/// Dropping a receipt does not cancel the queue or release an in-flight buffer.
#[derive(Clone, Debug)]
pub struct AudioReceipt {
    owner: Rc<()>,
    instance: Rc<()>,
    identity: PlaybackCue,
}

impl AudioReceipt {
    #[cfg(target_arch = "wasm32")]
    pub fn identity(&self) -> PlaybackCue {
        self.identity
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn identity(&self) -> df_media::speech::SpeechIdentity {
        crate::native::speech_identity(self.identity)
    }
    pub fn presentation_identity(&self) -> PlaybackCue {
        self.identity
    }
}

/// Checked caption association failure, without exposing private approved text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptionError {
    Queue(QueueError),
    InvalidLimit,
    Empty,
    Capacity,
    Allocation,
}

impl From<QueueError> for CaptionError {
    fn from(error: QueueError) -> Self {
        Self::Queue(error)
    }
}

/// One caller-approved text/source association, fenced to its exact admission receipt.
/// This does not verify that encoded or decoded sound speaks the supplied text.
/// The caller owns approval, source authorization and the actual sound/text provenance.
pub struct CaptionBinding {
    receipt: AudioReceipt,
    source: PcmSource,
    text: Box<str>,
}

impl fmt::Debug for CaptionBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CaptionBinding")
            .finish_non_exhaustive()
    }
}

/// Invalid supplied observation. No local clock or authoritative state is consulted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimelineError {
    BeforeAnchor,
    BeforeFirstFrame,
    ArithmeticOverflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DriftDirection {
    Behind,
    Aligned,
    Ahead,
}

/// Exact drift in nanoseconds is numerator / sample_rate, with no rounded budget comparison.
/// This is a caller-observed timing fact, not evidence of audible sound or server authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlaybackDrift {
    direction: DriftDirection,
    nanoseconds_numerator: u128,
    sample_rate: u32,
    within_budget: bool,
}

impl PlaybackDrift {
    pub fn direction(self) -> DriftDirection {
        self.direction
    }
    pub fn nanoseconds_numerator(self) -> u128 {
        self.nanoseconds_numerator
    }
    pub fn sample_rate(self) -> u32 {
        self.sample_rate
    }
    pub fn within_budget(self) -> bool {
        self.within_budget
    }
}

/// Immutable caller-supplied server anchor for one PCM timeline.
/// Clock mapping and actual playback observations belong to the mounted caller.
/// Chunk arrival cannot change this anchor, and no observation mutates game state.
#[derive(Clone, Copy, Debug)]
pub struct ServerPlaybackTimeline {
    first_frame: u64,
    anchor_server_time: Duration,
    format: PcmFormat,
}

impl ServerPlaybackTimeline {
    /// Existing PcmFormat construction guarantees a nonzero sample rate.
    pub fn new(first_frame: u64, anchor_server_time: Duration, format: PcmFormat) -> Self {
        Self {
            first_frame,
            anchor_server_time,
            format,
        }
    }

    /// Compare observed frames against elapsed server time using exact integer units.
    /// The caller supplies its admissible drift budget explicitly, including zero.
    /// Before-anchor time or frames before the first frame are refused, never saturated.
    pub fn observe(
        self,
        server_time: Duration,
        observed_playback_frame: u64,
        maximum_drift: Duration,
    ) -> Result<PlaybackDrift, TimelineError> {
        let elapsed = server_time
            .checked_sub(self.anchor_server_time)
            .ok_or(TimelineError::BeforeAnchor)?;
        let observed_frame_offset = observed_playback_frame
            .checked_sub(self.first_frame)
            .ok_or(TimelineError::BeforeFirstFrame)?;
        let sample_rate_hz = u128::from(self.format.sample_rate());
        let expected = elapsed
            .as_nanos()
            .checked_mul(sample_rate_hz)
            .ok_or(TimelineError::ArithmeticOverflow)?;
        let observed = u128::from(observed_frame_offset)
            .checked_mul(1_000_000_000)
            .ok_or(TimelineError::ArithmeticOverflow)?;
        let budget = maximum_drift
            .as_nanos()
            .checked_mul(sample_rate_hz)
            .ok_or(TimelineError::ArithmeticOverflow)?;
        let direction = match observed.cmp(&expected) {
            std::cmp::Ordering::Less => DriftDirection::Behind,
            std::cmp::Ordering::Equal => DriftDirection::Aligned,
            std::cmp::Ordering::Greater => DriftDirection::Ahead,
        };
        let difference = expected.abs_diff(observed);
        Ok(PlaybackDrift {
            direction,
            nanoseconds_numerator: difference,
            sample_rate: self.format.sample_rate(),
            within_budget: difference <= budget,
        })
    }
}

/// One local dispatch instance. Pointer identity cannot be reused while a late
/// callback exists, including after same-job replacement or owner reconstruction.
#[derive(Clone, Debug)]
pub struct BufferTicket {
    owner: Rc<()>,
    instance: Rc<()>,
    identity: PlaybackCue,
    sequence: u64,
}

impl BufferTicket {
    #[cfg(target_arch = "wasm32")]
    pub fn identity(&self) -> PlaybackCue {
        self.identity
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn identity(&self) -> df_media::speech::SpeechIdentity {
        crate::native::speech_identity(self.identity)
    }
    pub fn presentation_identity(&self) -> PlaybackCue {
        self.identity
    }
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
}

#[derive(Debug)]
pub struct Cancellation {
    pub discarded_buffers: usize,
    pub discarded_sample_bytes: usize,
    pub stop_required: Option<BufferTicket>,
}

#[derive(Debug)]
pub struct Replacement {
    pub receipt: AudioReceipt,
    pub retired: Cancellation,
}

pub struct EnqueueRefusal {
    pub reason: QueueError,
    pub buffer: PcmBuffer,
}

impl fmt::Debug for EnqueueRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EnqueueRefusal")
            .field("reason", &self.reason)
            .field("sample_bytes", &self.buffer.sample_bytes())
            .finish()
    }
}

/// Borrowed samples remain owned and accounted by the queue until exact acknowledgement.
pub struct PresentationPcmView<'a> {
    pub identity: PlaybackCue,
    pub asset: &'a PcmSource,
    pub sequence: u64,
    pub offset_frames: u64,
    pub buffer: &'a PcmBuffer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BufferCompletion {
    Current(QueueState),
    Retired(QueueState),
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum CueState {
    Accepting,
    Draining,
    Cancelled,
}

struct Cue {
    receipt: AudioReceipt,
    asset: PcmSource,
    format: PcmFormat,
    next_sequence: u64,
    next_offset: u64,
    admitted_frames: u64,
    state: CueState,
}

struct QueuedBuffer {
    sequence: u64,
    offset_frames: u64,
    buffer: PcmBuffer,
}
struct Dispatch {
    ticket: BufferTicket,
    asset: PcmSource,
    queued: QueuedBuffer,
    retiring: bool,
}

/// Pure single-owner queue for already-authorized decoded presentation resources.
///
/// At most one dispatched PCM buffer remains retained. Replacement/cancellation clears
/// only queued resources and requests stop of that exact dispatch; new work waits for
/// its actual consumer to acknowledge stop/end. Stop failure/lost callbacks therefore
/// remain an explicit bounded waiting condition, never fabricated silence or completion.
/// Supplied frame offsets are preserved; no wall-clock or arrival-time offset is used.
/// Source manifests are compared, not verified or granted rights by this queue.
pub struct PresentationQueue {
    owner: Rc<()>,
    lease: PlaybackOutput,
    basis: PlaybackBasis,
    limits: QueueLimits,
    cue: Option<Cue>,
    queued: VecDeque<QueuedBuffer>,
    dispatch: Option<Dispatch>,
    sample_bytes: usize,
    disposed: bool,
}

pub(crate) fn validate_limits(limits: QueueLimits) -> Result<(), QueueError> {
    if limits.retained_buffers == 0
        || limits.retained_buffers > 256
        || limits.sample_bytes == 0
        || limits.sample_bytes > MAX_SAMPLE_BYTES
        || limits.cue_chunks == 0
        || limits.cue_chunks > 4096
        || limits.cue_frames == 0
    {
        return Err(QueueError::InvalidLimits);
    }
    Ok(())
}

impl PresentationQueue {
    pub fn new(
        binding: ClientBindingId,
        lease: PlaybackOutput,
        basis: PlaybackBasis,
        limits: QueueLimits,
    ) -> Result<Self, QueueError> {
        validate_limits(limits)?;
        if lease.binding() != binding {
            return Err(QueueError::InvalidLease);
        }
        let mut queued = VecDeque::new();
        queued
            .try_reserve_exact(limits.retained_buffers)
            .map_err(|_| QueueError::Allocation)?;
        Ok(Self {
            owner: Rc::new(()),
            lease,
            basis,
            limits,
            cue: None,
            queued,
            dispatch: None,
            sample_bytes: 0,
            disposed: false,
        })
    }

    /// Current trusted scope is supplied by the mounted shell, never inferred from IDs.
    /// Only that shell may choose a replacement from its current approved speech work;
    /// a late producer callback must use enqueue with its original receipt instead.
    /// Generations are per job, not a fabricated global order across different jobs.
    /// New session/run or output lease ownership requires explicit dispose and a new owner.
    /// Replacement returns an exact old stop request and retains only the newest cue.
    pub fn replace(
        &mut self,
        lease: &PlaybackOutput,
        basis: PlaybackBasis,
        identity: PlaybackCue,
        asset: PcmSource,
        format: PcmFormat,
        first_frame: u64,
    ) -> Result<Replacement, QueueError> {
        self.check_output(lease)?;
        if !lease.device_unlocked() {
            return Err(QueueError::Locked);
        }
        self.check_basis(basis, identity.basis())?;
        if identity.generation() == 0 {
            return Err(QueueError::InvalidGeneration);
        }
        if asset.manifest().byte_len == 0 {
            return Err(QueueError::WrongAsset);
        }
        if let Some(cue) = &self.cue {
            let current = cue.receipt.identity;
            if current == identity {
                return Err(QueueError::DuplicateCue);
            }
            if current.job() == identity.job() && identity.generation() <= current.generation() {
                return Err(QueueError::StaleGeneration);
            }
        }
        let receipt = AudioReceipt {
            owner: Rc::clone(&self.owner),
            instance: Rc::new(()),
            identity,
        };
        let retired = self.retire();
        self.basis = basis;
        self.cue = Some(Cue {
            receipt: receipt.clone(),
            asset,
            format,
            next_sequence: 0,
            next_offset: first_frame,
            admitted_frames: 0,
            state: CueState::Accepting,
        });
        Ok(Replacement { receipt, retired })
    }

    /// Bind caller-approved text to the exact current cue and selected PCM source.
    /// Retained text is bounded before allocation. Approval and sound/text fidelity
    /// are caller responsibilities; source equality grants no access by itself.
    pub fn bind_caption(
        &self,
        receipt: &AudioReceipt,
        lease: &PlaybackOutput,
        selected_source: &PcmSource,
        caller_approved_caption: &str,
        maximum_caption_bytes: usize,
    ) -> Result<CaptionBinding, CaptionError> {
        self.check_caption_source(receipt, lease, selected_source)?;
        if maximum_caption_bytes == 0 || maximum_caption_bytes > 16_384 {
            return Err(CaptionError::InvalidLimit);
        }
        if caller_approved_caption.len() > maximum_caption_bytes {
            return Err(CaptionError::Capacity);
        }
        if caller_approved_caption.trim().is_empty() {
            return Err(CaptionError::Empty);
        }
        let mut text = String::new();
        text.try_reserve_exact(caller_approved_caption.len())
            .map_err(|_| CaptionError::Allocation)?;
        text.push_str(caller_approved_caption);
        Ok(CaptionBinding {
            receipt: receipt.clone(),
            source: selected_source.clone(),
            text: text.into_boxed_str(),
        })
    }

    /// Return text only for its original current admission and exact source.
    /// Even a replacement reusing identical labels must create a fresh association.
    /// A stop/drain/disposal or pending old-source stop cannot authorize new captions.
    pub fn caption<'a>(
        &self,
        binding: &'a CaptionBinding,
        receipt: &AudioReceipt,
        lease: &PlaybackOutput,
        selected_source: &PcmSource,
    ) -> Result<&'a str, CaptionError> {
        self.check_caption_source(receipt, lease, selected_source)?;
        if !Rc::ptr_eq(&binding.receipt.owner, &receipt.owner)
            || !Rc::ptr_eq(&binding.receipt.instance, &receipt.instance)
            || binding.receipt.identity != receipt.identity
        {
            return Err(QueueError::StaleReceipt.into());
        }
        if binding.source != *selected_source {
            return Err(QueueError::WrongAsset.into());
        }
        Ok(&binding.text)
    }

    fn check_caption_source(
        &self,
        receipt: &AudioReceipt,
        lease: &PlaybackOutput,
        selected_source: &PcmSource,
    ) -> Result<(), QueueError> {
        let cue = self.check(receipt, lease)?;
        if cue.asset != *selected_source {
            return Err(QueueError::WrongAsset);
        }
        match self.snapshot().state {
            QueueState::Accepting | QueueState::Draining => Ok(()),
            QueueState::WaitingForStop => Err(QueueError::WaitingForStop),
            _ => Err(QueueError::Closed),
        }
    }

    /// Refusal returns the unchanged decoded buffer, enabling backpressure without loss.
    /// Caller must recheck current source authorization/decode lease before retrying.
    pub fn enqueue(
        &mut self,
        receipt: &AudioReceipt,
        lease: &PlaybackOutput,
        manifest: AssetManifest,
        sequence: u64,
        offset_frames: u64,
        buffer: PcmBuffer,
    ) -> Result<(), EnqueueRefusal> {
        if let Err(reason) =
            self.validate_enqueue(receipt, lease, manifest, sequence, offset_frames, &buffer)
        {
            return Err(EnqueueRefusal { reason, buffer });
        }
        let Some(cue) = self.cue.as_mut() else {
            return Err(EnqueueRefusal {
                reason: QueueError::StaleReceipt,
                buffer,
            });
        };
        cue.next_sequence += 1; // The finite cue_chunks bound was checked before mutation.
        cue.next_offset += buffer.frames();
        cue.admitted_frames += buffer.frames();
        self.sample_bytes += buffer.sample_bytes();
        self.queued.push_back(QueuedBuffer {
            sequence,
            offset_frames,
            buffer,
        });
        Ok(())
    }

    fn validate_enqueue(
        &self,
        receipt: &AudioReceipt,
        lease: &PlaybackOutput,
        manifest: AssetManifest,
        sequence: u64,
        offset_frames: u64,
        buffer: &PcmBuffer,
    ) -> Result<(), QueueError> {
        let cue = self.check(receipt, lease)?;
        if cue.state != CueState::Accepting {
            return Err(QueueError::Closed);
        }
        if self
            .dispatch
            .as_ref()
            .is_some_and(|dispatch| dispatch.retiring)
        {
            return Err(QueueError::WaitingForStop);
        }
        if manifest.byte_len != cue.asset.manifest().byte_len
            || manifest.sha256 != cue.asset.manifest().sha256
        {
            return Err(QueueError::WrongAsset);
        }
        if buffer.format() != cue.format {
            return Err(QueueError::InvalidFormat);
        }
        if sequence != cue.next_sequence {
            return Err(QueueError::WrongSequence);
        }
        if offset_frames != cue.next_offset {
            return Err(QueueError::WrongOffset);
        }
        if sequence >= self.limits.cue_chunks
            || cue
                .admitted_frames
                .checked_add(buffer.frames())
                .is_none_or(|frames| frames > self.limits.cue_frames)
        {
            return Err(QueueError::CueCapacity);
        }
        if offset_frames.checked_add(buffer.frames()).is_none() {
            return Err(QueueError::FrameOverflow);
        }
        if self.queued.len() + usize::from(self.dispatch.is_some()) >= self.limits.retained_buffers
        {
            return Err(QueueError::BufferCapacity);
        }
        if self
            .sample_bytes
            .checked_add(buffer.sample_bytes())
            .is_none_or(|bytes| bytes > self.limits.sample_bytes)
        {
            return Err(QueueError::SampleCapacity);
        }
        Ok(())
    }

    /// Begin one owned consumer dispatch. Dropping its ticket does not free PCM.
    /// The adapter must create/start its real node only after this succeeds.
    pub fn begin(
        &mut self,
        receipt: &AudioReceipt,
        lease: &PlaybackOutput,
    ) -> Result<Option<BufferTicket>, QueueError> {
        let cue = self.check(receipt, lease)?;
        if cue.state == CueState::Cancelled {
            return Err(QueueError::Closed);
        }
        if let Some(dispatch) = &self.dispatch {
            return Err(if dispatch.retiring {
                QueueError::WaitingForStop
            } else {
                QueueError::Busy
            });
        }
        let asset = cue.asset.clone();
        let Some(queued) = self.queued.pop_front() else {
            return Ok(None);
        };
        let ticket = BufferTicket {
            owner: Rc::clone(&self.owner),
            instance: Rc::new(()),
            identity: receipt.identity,
            sequence: queued.sequence,
        };
        self.dispatch = Some(Dispatch {
            ticket: ticket.clone(),
            asset,
            queued,
            retiring: false,
        });
        Ok(Some(ticket))
    }

    /// Repeated reads remain borrowed and counted. No destructive take or PCM clone is offered.
    pub fn dispatched(&self, ticket: &BufferTicket) -> Result<PresentationPcmView<'_>, QueueError> {
        let dispatch = self.check_callback(ticket)?;
        if dispatch.retiring {
            return Err(QueueError::WaitingForStop);
        }
        Ok(PresentationPcmView {
            identity: dispatch.ticket.identity,
            asset: &dispatch.asset,
            sequence: dispatch.queued.sequence,
            offset_frames: dispatch.queued.offset_frames,
            buffer: &dispatch.queued.buffer,
        })
    }

    /// Exact finite terminal count/offset closes admission and preserves the queued tail.
    /// A mismatch returns Incomplete without inventing completion or dropping resources.
    pub fn close(
        &mut self,
        receipt: &AudioReceipt,
        lease: &PlaybackOutput,
        chunk_count: u64,
        end_frame: u64,
    ) -> Result<QueueState, QueueError> {
        let cue = self.check(receipt, lease)?;
        if cue.state != CueState::Accepting {
            return Err(QueueError::Closed);
        }
        if self
            .dispatch
            .as_ref()
            .is_some_and(|dispatch| dispatch.retiring)
        {
            return Err(QueueError::WaitingForStop);
        }
        if cue.next_sequence != chunk_count || cue.next_offset != end_frame {
            return Err(QueueError::Incomplete);
        }
        if let Some(cue) = self.cue.as_mut() {
            cue.state = CueState::Draining;
        }
        Ok(self.snapshot().state)
    }

    /// Actual node-end acknowledgement. A retiring old end releases only its old
    /// buffer and reports Retired; it cannot finish or acknowledge a newer dispatch.
    pub fn complete(&mut self, ticket: &BufferTicket) -> Result<BufferCompletion, QueueError> {
        self.check_callback(ticket)?;
        let dispatch = self.dispatch.take().ok_or(QueueError::StaleCallback)?;
        self.sample_bytes -= dispatch.queued.buffer.sample_bytes();
        let state = self.snapshot().state;
        Ok(if dispatch.retiring {
            BufferCompletion::Retired(state)
        } else {
            BufferCompletion::Current(state)
        })
    }

    /// The adapter confirms stop only after its real stop/disconnect operation succeeded.
    /// Failure leaves this exact ticket retained; the caller may retry bounded owned work.
    pub fn confirm_stopped(
        &mut self,
        ticket: &BufferTicket,
    ) -> Result<BufferCompletion, QueueError> {
        if !self.check_callback(ticket)?.retiring {
            return Err(QueueError::StopNotRequested);
        }
        self.complete(ticket)
    }

    pub fn cancel(
        &mut self,
        receipt: &AudioReceipt,
        lease: &PlaybackOutput,
    ) -> Result<Cancellation, QueueError> {
        self.check(receipt, lease)?;
        if let Some(cue) = self.cue.as_mut() {
            cue.state = CueState::Cancelled;
        }
        Ok(self.retire())
    }

    /// Applies an already-permitted local stop label to its original admission receipt.
    /// The event owner retains the receipt returned by replace; selecting a newer
    /// current receipt would let a delayed same-identity stop cancel replacement work.
    /// Native media stop records are projected at the native adapter, not retained here.
    pub fn cancel_media(
        &mut self,
        receipt: &AudioReceipt,
        lease: &PlaybackOutput,
        stopped: &PlaybackCue,
    ) -> Result<Cancellation, QueueError> {
        self.check(receipt, lease)?;
        if *stopped != receipt.identity {
            return Err(QueueError::StaleReceipt);
        }
        self.cancel(receipt, lease)
    }

    /// Permanently closes admission. At most one stop request remains; its exact
    /// confirmation can still retire that buffer after disposal. No background wait.
    pub fn dispose(&mut self) -> Cancellation {
        self.disposed = true;
        if let Some(cue) = self.cue.as_mut() {
            cue.state = CueState::Cancelled;
        }
        self.retire()
    }

    pub fn snapshot(&self) -> QueueSnapshot {
        let waiting = self
            .dispatch
            .as_ref()
            .is_some_and(|dispatch| dispatch.retiring);
        let state = if waiting {
            QueueState::WaitingForStop
        } else if self.disposed {
            QueueState::Disposed
        } else {
            match self.cue.as_ref().map(|cue| cue.state) {
                None => QueueState::Idle,
                Some(CueState::Accepting) => QueueState::Accepting,
                Some(CueState::Cancelled) => QueueState::Cancelled,
                Some(CueState::Draining) if self.queued.is_empty() && self.dispatch.is_none() => {
                    QueueState::Drained
                }
                Some(CueState::Draining) => QueueState::Draining,
            }
        };
        QueueSnapshot {
            state,
            retained_buffers: self.queued.len() + usize::from(self.dispatch.is_some()),
            sample_bytes: self.sample_bytes,
            dispatched: self.dispatch.is_some(),
            admission_closed: self.disposed || state != QueueState::Accepting,
        }
    }

    fn retire(&mut self) -> Cancellation {
        let discarded_buffers = self.queued.len();
        let discarded_sample_bytes = self
            .queued
            .iter()
            .map(|queued| queued.buffer.sample_bytes())
            .sum();
        self.queued.clear();
        self.sample_bytes -= discarded_sample_bytes;
        let stop_required = self.dispatch.as_mut().map(|dispatch| {
            dispatch.retiring = true;
            dispatch.ticket.clone()
        });
        Cancellation {
            discarded_buffers,
            discarded_sample_bytes,
            stop_required,
        }
    }

    pub(crate) fn check_basis(
        &self,
        basis: PlaybackBasis,
        identity_basis: PlaybackBasis,
    ) -> Result<(), QueueError> {
        if basis.session() != self.basis.session()
            || basis.run() != self.basis.run()
            || basis.revision() < self.basis.revision()
            || identity_basis != basis
        {
            return Err(QueueError::WrongBasis);
        }
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn check_receipt(
        &self,
        receipt: &AudioReceipt,
        lease: &PlaybackOutput,
    ) -> Result<(), QueueError> {
        self.check(receipt, lease).map(|_| ())
    }

    pub(crate) fn check_output(&self, lease: &PlaybackOutput) -> Result<(), QueueError> {
        if self.disposed {
            return Err(QueueError::Disposed);
        }
        if lease != &self.lease {
            return Err(QueueError::WrongLease);
        }
        Ok(())
    }

    fn check(&self, receipt: &AudioReceipt, lease: &PlaybackOutput) -> Result<&Cue, QueueError> {
        self.check_output(lease)?;
        if !Rc::ptr_eq(&self.owner, &receipt.owner) {
            return Err(QueueError::ForeignOwner);
        }
        self.cue
            .as_ref()
            .filter(|cue| {
                cue.receipt.identity == receipt.identity
                    && Rc::ptr_eq(&cue.receipt.instance, &receipt.instance)
            })
            .ok_or(QueueError::StaleReceipt)
    }

    fn check_callback(&self, ticket: &BufferTicket) -> Result<&Dispatch, QueueError> {
        if !Rc::ptr_eq(&self.owner, &ticket.owner) {
            return Err(QueueError::ForeignOwner);
        }
        self.dispatch
            .as_ref()
            .filter(|dispatch| {
                Rc::ptr_eq(&dispatch.ticket.instance, &ticket.instance)
                    && dispatch.ticket.identity == ticket.identity
                    && dispatch.ticket.sequence == ticket.sequence
            })
            .ok_or(QueueError::StaleCallback)
    }
}
