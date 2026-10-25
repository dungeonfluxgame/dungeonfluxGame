//! Native-only canonical admission adapter. Projection is never an authorization check.
use df_assets::AssetManifest;
use df_media::speech::{SpeechIdentity, SpeechStopped};
use df_model::checkpoint::{
    AssetKind, AssetReference, AudienceScope, AudioDestination, AudioOutputLease, Basis, JobId,
};
use df_types::ClientBindingId;

use crate::{
    AudioReceipt, BufferCompletion, BufferTicket, Cancellation, EnqueueRefusal, PcmBuffer,
    PcmFormat, PcmSource, PlaybackBasis, PlaybackCue, PlaybackDestination, PlaybackOutput,
    PresentationQueue, QueueError, QueueLimits, QueueSnapshot, QueueState, Replacement,
};

fn basis(canonical_basis: Basis) -> PlaybackBasis {
    PlaybackBasis::new(
        canonical_basis.session,
        canonical_basis.run,
        canonical_basis.revision,
    )
}

fn cue(speech_identity: SpeechIdentity) -> Result<PlaybackCue, QueueError> {
    PlaybackCue::new(
        basis(speech_identity.basis),
        *speech_identity.job.as_bytes(),
        speech_identity.operation,
        speech_identity.generation,
    )
}

pub(crate) fn speech_identity(playback_cue: PlaybackCue) -> SpeechIdentity {
    // PlaybackCue::new validated a nonzero fixed 16-byte job; the canonical JobId
    // uses that same validator. This invariant is independent of external input.
    let job = JobId::from_bytes(&playback_cue.job()).expect("checked nonzero playback job");
    let projected = playback_cue.basis();
    SpeechIdentity {
        basis: Basis {
            session: projected.session(),
            run: projected.run(),
            revision: projected.revision(),
        },
        job,
        operation: playback_cue.operation(),
        generation: playback_cue.generation(),
    }
}

fn output(output_lease: &AudioOutputLease) -> Result<PlaybackOutput, QueueError> {
    let (binding, destination) = match output_lease.destination {
        AudioDestination::PublicRoom(binding) => (binding, PlaybackDestination::PublicRoom),
        AudioDestination::PrivateListener { binding, .. } => {
            (binding, PlaybackDestination::PrivateListener)
        }
    };
    PlaybackOutput::new(
        *output_lease.id.as_bytes(),
        binding,
        output_lease.generation,
        destination,
        output_lease.device_unlocked,
    )
}

fn source(asset_reference: &AssetReference) -> Result<PcmSource, QueueError> {
    if asset_reference.kind != AssetKind::Audio {
        return Err(QueueError::WrongAsset);
    }
    PcmSource::new(
        asset_reference.key.clone(),
        AssetManifest {
            byte_len: asset_reference.byte_length,
            sha256: asset_reference.digest.0,
        },
    )
}

/// Native callers retain the complete lease. Same projected labels do not admit changed
/// canonical audience/private-member state. All PCM and callback ownership lives in core.
pub struct AudioQueue {
    core: PresentationQueue,
    lease: AudioOutputLease,
    cue_asset: Option<AssetReference>,
    dispatch_asset: Option<AssetReference>,
}

/// Native source metadata remains borrowed while the single core owns and accounts PCM.
pub struct PcmView<'a> {
    pub identity: SpeechIdentity,
    pub asset: &'a AssetReference,
    pub sequence: u64,
    pub offset_frames: u64,
    pub buffer: &'a PcmBuffer,
}

impl AudioQueue {
    pub fn new(
        binding: ClientBindingId,
        lease: AudioOutputLease,
        current: Basis,
        limits: QueueLimits,
    ) -> Result<Self, QueueError> {
        crate::queue::validate_limits(limits)?;
        let projected = output(&lease)?;
        if projected.binding() != binding {
            return Err(QueueError::InvalidLease);
        }
        if let AudienceScope::Members(members) = &lease.audience
            && members.capacity() > 256
        {
            return Err(QueueError::InvalidLease);
        }
        let core = PresentationQueue::new(binding, projected, basis(current), limits)?;
        Ok(Self {
            core,
            lease,
            cue_asset: None,
            dispatch_asset: None,
        })
    }

    fn check_lease(&self, lease: &AudioOutputLease) -> Result<(), QueueError> {
        // The core's disposed check precedes canonical equality, as before the split.
        self.core.check_output(&output(&self.lease)?)?;
        if lease != &self.lease {
            return Err(QueueError::WrongLease);
        }
        Ok(())
    }

    pub fn replace(
        &mut self,
        lease: &AudioOutputLease,
        current: Basis,
        identity: SpeechIdentity,
        asset: AssetReference,
        format: PcmFormat,
        first_frame: u64,
    ) -> Result<Replacement, QueueError> {
        self.check_lease(lease)?;
        if !lease.device_unlocked {
            return Err(QueueError::Locked);
        }
        // Keep original pre-admission ordering: scope precedes generation and source.
        let projected_basis = basis(current);
        self.core
            .check_basis(projected_basis, basis(identity.basis))?;
        let projected_cue = cue(identity)?;
        let projected_source = source(&asset)?;
        let replacement = self.core.replace(
            &output(lease)?,
            projected_basis,
            projected_cue,
            projected_source,
            format,
            first_frame,
        )?;
        self.cue_asset = Some(asset);
        Ok(replacement)
    }

    pub fn enqueue(
        &mut self,
        receipt: &AudioReceipt,
        lease: &AudioOutputLease,
        manifest: AssetManifest,
        sequence: u64,
        offset_frames: u64,
        buffer: PcmBuffer,
    ) -> Result<(), EnqueueRefusal> {
        if let Err(reason) = self.check_lease(lease) {
            return Err(EnqueueRefusal { reason, buffer });
        }
        // Stored canonical lease was checked at construction; do not manufacture a default.
        let projected = match output(lease) {
            Ok(projected) => projected,
            Err(reason) => return Err(EnqueueRefusal { reason, buffer }),
        };
        self.core.enqueue(
            receipt,
            &projected,
            manifest,
            sequence,
            offset_frames,
            buffer,
        )
    }

    pub fn begin(
        &mut self,
        receipt: &AudioReceipt,
        lease: &AudioOutputLease,
    ) -> Result<Option<BufferTicket>, QueueError> {
        self.check_lease(lease)?;
        let ticket = self.core.begin(receipt, &output(lease)?)?;
        if ticket.is_some() {
            self.dispatch_asset = self.cue_asset.clone();
        }
        Ok(ticket)
    }

    pub fn dispatched(&self, ticket: &BufferTicket) -> Result<PcmView<'_>, QueueError> {
        let view = self.core.dispatched(ticket)?;
        let asset = self
            .dispatch_asset
            .as_ref()
            .ok_or(QueueError::StaleCallback)?;
        Ok(PcmView {
            identity: speech_identity(view.identity),
            asset,
            sequence: view.sequence,
            offset_frames: view.offset_frames,
            buffer: view.buffer,
        })
    }

    pub fn close(
        &mut self,
        receipt: &AudioReceipt,
        lease: &AudioOutputLease,
        chunk_count: u64,
        end_frame: u64,
    ) -> Result<QueueState, QueueError> {
        self.check_lease(lease)?;
        self.core
            .close(receipt, &output(lease)?, chunk_count, end_frame)
    }

    pub fn complete(&mut self, ticket: &BufferTicket) -> Result<BufferCompletion, QueueError> {
        let completed = self.core.complete(ticket)?;
        self.dispatch_asset = None;
        Ok(completed)
    }

    pub fn confirm_stopped(
        &mut self,
        ticket: &BufferTicket,
    ) -> Result<BufferCompletion, QueueError> {
        let completed = self.core.confirm_stopped(ticket)?;
        self.dispatch_asset = None;
        Ok(completed)
    }

    pub fn cancel(
        &mut self,
        receipt: &AudioReceipt,
        lease: &AudioOutputLease,
    ) -> Result<Cancellation, QueueError> {
        self.check_lease(lease)?;
        self.core.cancel(receipt, &output(lease)?)
    }

    /// The producer MUST retain its original AudioReceipt from admission. An old event
    /// cannot select current work merely because a later cue reused its identity.
    pub fn cancel_media(
        &mut self,
        receipt: &AudioReceipt,
        lease: &AudioOutputLease,
        stopped: &SpeechStopped,
    ) -> Result<Cancellation, QueueError> {
        self.check_lease(lease)?;
        // Validate receipt owner/instance before inspecting or projecting the event.
        self.core.check_receipt(receipt, &output(lease)?)?;
        // Compare the actual canonical stop before checked projection so a mismatched
        // generation (including zero) retains the original StaleReceipt classification.
        if stopped.identity != receipt.identity() {
            return Err(QueueError::StaleReceipt);
        }
        self.core
            .cancel_media(receipt, &output(lease)?, &cue(stopped.identity)?)
    }

    pub fn dispose(&mut self) -> Cancellation {
        self.core.dispose()
    }
    pub fn snapshot(&self) -> QueueSnapshot {
        self.core.snapshot()
    }
}
