//! The text each `speech` verb prints: keys and values. A transcript is printed because the
//! verb's job is to show it; nothing else here carries what was said.

use super::request::{
    ClipPlayed, ClipRow, ClipsCleared, SpeechStatus, Transcript, VoiceChanged, Voiced,
};

pub(super) fn format_status(status: &SpeechStatus) -> String {
    let mut lines = vec![format!(
        "speech: configured={} missing={} tts_model={} stt_model={} clips_kept={}",
        status.configured,
        status.missing.as_deref().unwrap_or("-"),
        status.tts_model,
        status.stt_model,
        status.clips_kept
    )];
    lines.push(format!(
        "  default voice: {}",
        status.default_voice.as_deref().unwrap_or("-")
    ));
    if status.voices.is_empty() {
        lines.push("  no character has a voice of their own".to_owned());
    }
    lines.extend(
        status
            .voices
            .iter()
            .map(|row| format!("  {}: {}", row.character, row.voice)),
    );
    let limits = &status.limits;
    lines.push(format!(
        "  limits: audio {}..{} ms, text {} characters, {} key terms, {} clips kept",
        limits.minimum_audio_ms,
        limits.maximum_audio_ms,
        limits.maximum_text_characters,
        limits.maximum_key_terms,
        limits.maximum_kept_clips
    ));
    lines.join("\n")
}

pub(super) fn format_played(played: &ClipPlayed) -> String {
    format!(
        "ok (revision {}) voiced {} ({} bytes, {} ms, {})",
        played.revision, played.clip, played.bytes, played.synthesis_ms, played.voice_profile
    )
}

pub(super) fn format_transcript(transcript: &Transcript) -> String {
    format!(
        "transcript ({}, {} ms of audio, {} ms): {}",
        transcript.language, transcript.audio_ms, transcript.duration_ms, transcript.text
    )
}

pub(super) fn format_voice(changed: &VoiceChanged) -> String {
    format!(
        "ok {} {}",
        changed.character,
        match (changed.has_voice, changed.changed) {
            (true, true) => "has a voice now",
            (true, false) => "already had that voice",
            (false, true) => "has no voice of their own now",
            (false, false) => "had no voice of their own",
        }
    )
}

pub(super) fn format_clips(clips: &[ClipRow]) -> String {
    if clips.is_empty() {
        return "no voice clips".to_owned();
    }
    let lines: Vec<String> = clips
        .iter()
        .map(|clip| format!("  {} ({} bytes)", clip.name, clip.bytes))
        .collect();
    format!("voice clips:\n{}", lines.join("\n"))
}

pub(super) fn format_cleared(cleared: &ClipsCleared) -> String {
    format!("ok removed {} voice clips", cleared.removed)
}

/// The line `persona say` and `persona ask` print about a line's voice.
pub fn format_voiced(voiced: &Voiced) -> String {
    match voiced {
        Voiced::Played {
            clip,
            bytes,
            voice_profile,
            synthesis_ms,
        } => format!("  voice: {clip} ({bytes} bytes, {synthesis_ms} ms, {voice_profile})"),
        Voiced::Failed { error } => format!("  voice: failed({}): {error}", error.key()),
    }
}
