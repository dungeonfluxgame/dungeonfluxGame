//! The development email adapter (coordinator decision 4): each message becomes one `.eml` file
//! in a bounded directory under `artifacts/`, for the dev CLI to read. It never sends mail.
//!
//! Bounded: at most `MAXIMUM_OUTBOX_MESSAGES` files; the oldest are removed first. Files are
//! written under a temporary name and renamed, so a reader never sees half a message.

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use crate::{
    email::Message,
    ports::{EmailError, EmailSender},
};

/// Messages kept; older ones are deleted when a new one is written.
pub const MAXIMUM_OUTBOX_MESSAGES: usize = 200;
const SUFFIX: &str = ".eml";

/// One stored message's name, for the dev CLI index.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutboxEntry {
    pub sequence: u64,
    pub file_name: String,
}

pub struct DevOutbox {
    directory: PathBuf,
    next_sequence: AtomicU64,
    // Serializes write-and-prune so the bound holds under concurrent sends.
    writer: tokio::sync::Mutex<()>,
}

fn sequence_of(file_name: &str) -> Option<u64> {
    let stem = file_name.strip_suffix(SUFFIX)?;
    stem.split('-').next()?.parse().ok()
}

fn entries(directory: &Path) -> io::Result<Vec<OutboxEntry>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(directory)? {
        let file_name = entry?.file_name().to_string_lossy().into_owned();
        if let Some(sequence) = sequence_of(&file_name) {
            entries.push(OutboxEntry {
                sequence,
                file_name,
            });
        }
    }
    entries.sort_by_key(|entry| entry.sequence);
    Ok(entries)
}

fn render(sequence: u64, message: &Message) -> String {
    format!(
        "X-DungeonFlux-Sequence: {sequence}\nFrom: {}\nReply-To: {}\nTo: {}\nSubject: {}\n\
         X-DungeonFlux-Purpose: {}\nContent-Type: text/plain; charset=utf-8\n\n{}",
        message.from,
        message.reply_to,
        message.to,
        message.subject,
        message.purpose.label(),
        message.body
    )
}

fn write_and_prune(directory: &Path, sequence: u64, message: &Message) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    let file_name = format!("{sequence:08}-{}{SUFFIX}", message.purpose.label());
    let temporary = directory.join(format!(".{file_name}.tmp"));
    fs::write(&temporary, render(sequence, message))?;
    fs::rename(&temporary, directory.join(&file_name))?;
    let existing = entries(directory)?;
    let excess = existing.len().saturating_sub(MAXIMUM_OUTBOX_MESSAGES);
    for old in &existing[..excess] {
        fs::remove_file(directory.join(&old.file_name))?;
    }
    Ok(())
}

impl DevOutbox {
    /// Opens (creating) the directory and continues numbering after the newest message there.
    pub async fn open(directory: PathBuf) -> io::Result<Arc<Self>> {
        let scan = directory.clone();
        let newest = tokio::task::spawn_blocking(move || -> io::Result<u64> {
            fs::create_dir_all(&scan)?;
            Ok(entries(&scan)?.last().map_or(0, |entry| entry.sequence))
        })
        .await
        .map_err(io::Error::other)??;
        Ok(Arc::new(Self {
            directory,
            next_sequence: AtomicU64::new(newest + 1),
            writer: tokio::sync::Mutex::new(()),
        }))
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// The stored messages, oldest first.
    pub async fn index(&self) -> io::Result<Vec<OutboxEntry>> {
        let directory = self.directory.clone();
        tokio::task::spawn_blocking(move || entries(&directory))
            .await
            .map_err(io::Error::other)?
    }

    /// One stored message's full text, or `None` when it is gone.
    pub async fn message(&self, sequence: u64) -> io::Result<Option<String>> {
        let directory = self.directory.clone();
        tokio::task::spawn_blocking(move || {
            let Some(entry) = entries(&directory)?
                .into_iter()
                .find(|entry| entry.sequence == sequence)
            else {
                return Ok(None);
            };
            fs::read_to_string(directory.join(entry.file_name)).map(Some)
        })
        .await
        .map_err(io::Error::other)?
    }
}

impl EmailSender for Arc<DevOutbox> {
    async fn send(&self, message: Message) -> Result<(), EmailError> {
        let _writer = self.writer.lock().await;
        let sequence = self.next_sequence.fetch_add(1, Ordering::SeqCst);
        let directory = self.directory.clone();
        tokio::task::spawn_blocking(move || write_and_prune(&directory, sequence, &message))
            .await
            .map_err(|_| EmailError::Failed)?
            .map_err(|_| EmailError::Failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::email::MailPurpose;

    fn message(body: &str) -> Message {
        Message {
            from: "DungeonFlux <accounts@mail.example.test>".to_owned(),
            reply_to: "support@example.test".to_owned(),
            to: "robin@example.test".to_owned(),
            purpose: MailPurpose::SignupAttempt,
            subject: "subject",
            body: body.to_owned(),
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "df-customer-outbox-{name}-{}",
            crate::tokens::hex(&crate::tokens::random_bytes::<8>().unwrap())
        ));
        let _ = fs::remove_dir_all(&directory);
        directory
    }

    #[tokio::test]
    async fn the_outbox_keeps_the_newest_messages_and_removes_the_oldest_first() {
        let directory = scratch("bound");
        let outbox = DevOutbox::open(directory.clone()).await.unwrap();
        for index in 0..(MAXIMUM_OUTBOX_MESSAGES + 3) {
            outbox
                .send(message(&format!("body {index}")))
                .await
                .unwrap();
        }
        let index = outbox.index().await.unwrap();
        assert_eq!(index.len(), MAXIMUM_OUTBOX_MESSAGES);
        assert_eq!(index.first().unwrap().sequence, 4);
        let newest = outbox
            .message(index.last().unwrap().sequence)
            .await
            .unwrap()
            .unwrap();
        assert!(newest.ends_with(&format!("body {}", MAXIMUM_OUTBOX_MESSAGES + 2)));
        assert_eq!(outbox.message(1).await.unwrap(), None);
        fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn a_reopened_outbox_continues_numbering_after_its_newest_message() {
        let directory = scratch("reopen");
        DevOutbox::open(directory.clone())
            .await
            .unwrap()
            .send(message("first"))
            .await
            .unwrap();
        let reopened = DevOutbox::open(directory.clone()).await.unwrap();
        reopened.send(message("second")).await.unwrap();
        let sequences: Vec<u64> = reopened
            .index()
            .await
            .unwrap()
            .iter()
            .map(|entry| entry.sequence)
            .collect();
        assert_eq!(sequences, [1, 2]);
        fs::remove_dir_all(directory).unwrap();
    }
}
