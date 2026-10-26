//! Browser seat identity survives a closed tab; the server validates every restored credential.
use df_protocol::common as rpc;

const VERSION: &str = "v2";

pub(super) struct SavedPlayer {
    pub(super) credential: String,
    pub(super) session: rpc::SessionId,
    pub(super) run: rpc::RunId,
    pub(super) binding: rpc::ClientBindingId,
    /// The server run the seat was joined in, as that run published it ("" when it did not).
    pub(super) run_identity: String,
}

fn encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decode_id(text: &str) -> Option<Vec<u8>> {
    if text.len() != 32 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    text.as_bytes()
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect()
}

pub(super) fn parse(text: &str) -> Option<SavedPlayer> {
    let mut parts = text.split('.');
    let (version, credential, session, run, binding, run_identity) = (
        parts.next()?,
        parts.next()?,
        parts.next()?,
        parts.next()?,
        parts.next()?,
        parts.next()?,
    );
    // The server accepts only lowercase hexadecimal, 64 characters.
    if parts.next().is_some()
        || version != VERSION
        || credential.len() != 64
        || !credential
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return None;
    }
    Some(SavedPlayer {
        credential: credential.to_owned(),
        session: rpc::SessionId {
            value: Some(decode_id(session)?),
        },
        run: rpc::RunId {
            value: Some(decode_id(run)?),
        },
        binding: rpc::ClientBindingId {
            value: Some(decode_id(binding)?),
        },
        run_identity: run_identity.to_owned(),
    })
}

/// Persistent storage and tab storage share this small fallible boundary.
pub(super) trait Storage {
    type Error;
    fn get(&self, key: &str) -> Result<Option<String>, Self::Error>;
    fn set(&self, key: &str, value: &str) -> Result<(), Self::Error>;
    fn remove(&self, key: &str) -> Result<(), Self::Error>;
}
fn key(room_code: &str) -> String {
    format!("df.gameplay.{room_code}.player")
}

pub(super) fn load<S: Storage>(
    persistent: &S,
    tab: &S,
    room_code: &str,
) -> Result<Option<SavedPlayer>, S::Error> {
    let key = key(room_code);
    if let Some(text) = tab.get(&key)? {
        if let Some(saved) = parse(&text) {
            persistent.set(&key, &text)?;
            return Ok(Some(saved));
        }
        tab.remove(&key)?;
    }
    let Some(text) = persistent.get(&key)? else {
        return Ok(None);
    };
    let saved = parse(&text);
    if saved.is_some() {
        tab.set(&key, &text)?;
    } else {
        persistent.remove(&key)?;
    }
    Ok(saved)
}

pub(super) fn save<S: Storage>(
    persistent: &S,
    tab: &S,
    room_code: &str,
    saved: &SavedPlayer,
) -> Result<(), S::Error> {
    let id = |value: &Option<Vec<u8>>| encode(value.as_deref().unwrap_or_default());
    let encoded = format!(
        "{VERSION}.{}.{}.{}.{}.{}",
        saved.credential,
        id(&saved.session.value),
        id(&saved.run.value),
        id(&saved.binding.value),
        saved.run_identity
    );
    persistent.set(&key(room_code), &encoded)?;
    tab.set(&key(room_code), &encoded)
}

pub(super) fn clear<S: Storage>(persistent: &S, tab: &S, room_code: &str) -> Result<(), S::Error> {
    persistent.remove(&key(room_code))?;
    tab.remove(&key(room_code))
}

pub(super) fn is_from_run(saved: &SavedPlayer, served_run: &str) -> bool {
    saved.run_identity == served_run
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    #[derive(Default)]
    struct MemoryStorage(RefCell<HashMap<String, String>>);
    impl Storage for MemoryStorage {
        type Error = ();
        fn get(&self, key: &str) -> Result<Option<String>, ()> {
            Ok(self.0.borrow().get(key).cloned())
        }
        fn set(&self, key: &str, value: &str) -> Result<(), ()> {
            self.0.borrow_mut().insert(key.to_owned(), value.to_owned());
            Ok(())
        }
        fn remove(&self, key: &str) -> Result<(), ()> {
            self.0.borrow_mut().remove(key);
            Ok(())
        }
    }
    fn seat() -> SavedPlayer {
        SavedPlayer {
            credential: "ab".repeat(32),
            session: rpc::SessionId {
                value: Some(vec![1; 16]),
            },
            run: rpc::RunId {
                value: Some(vec![2; 16]),
            },
            binding: rpc::ClientBindingId {
                value: Some(vec![3; 16]),
            },
            run_identity: "current-run".to_owned(),
        }
    }
    #[test]
    fn closing_a_tab_or_browser_restores_the_same_seat_without_another_join() {
        let persistent = MemoryStorage::default();
        let tab = MemoryStorage::default();
        let original = seat();
        save(&persistent, &tab, "ROOM", &original).unwrap();
        let fresh_tab = MemoryStorage::default();
        let restored = load(&persistent, &fresh_tab, "ROOM").unwrap().unwrap();
        assert_eq!(restored.credential, original.credential);
        assert_eq!(restored.session, original.session);
        assert_eq!(restored.run, original.run);
        assert_eq!(restored.binding, original.binding);
        assert!(is_from_run(&restored, "current-run"));
        assert!(!is_from_run(&restored, "another-run"));
    }
    #[test]
    fn an_existing_tab_seat_migrates_and_leave_clears_both_stores() {
        let persistent = MemoryStorage::default();
        let tab = MemoryStorage::default();
        save(&tab, &MemoryStorage::default(), "ROOM", &seat()).unwrap();
        let migrated = load(&persistent, &tab, "ROOM").unwrap().unwrap();
        assert_eq!(migrated.credential, seat().credential);
        assert!(tab.get(&key("ROOM")).unwrap().is_some());
        assert!(
            load(&persistent, &MemoryStorage::default(), "ROOM")
                .unwrap()
                .is_some()
        );
        clear(&persistent, &tab, "ROOM").unwrap();
        assert!(load(&persistent, &tab, "ROOM").unwrap().is_none());
    }
    #[test]
    fn another_browser_or_room_has_no_saved_authority() {
        let persistent = MemoryStorage::default();
        let tab = MemoryStorage::default();
        save(&persistent, &tab, "ROOM", &seat()).unwrap();
        assert!(
            load(&MemoryStorage::default(), &MemoryStorage::default(), "ROOM")
                .unwrap()
                .is_none()
        );
        assert!(load(&persistent, &tab, "OTHER").unwrap().is_none());
    }
    #[test]
    fn an_existing_second_hero_tab_keeps_its_own_seat_when_another_is_remembered() {
        let persistent = MemoryStorage::default();
        let first_tab = MemoryStorage::default();
        let second_tab = MemoryStorage::default();
        let first = seat();
        let mut second = seat();
        second.credential = "cd".repeat(32);
        second.binding.value = Some(vec![4; 16]);
        save(&persistent, &first_tab, "ROOM", &first).unwrap();
        save(&MemoryStorage::default(), &second_tab, "ROOM", &second).unwrap();
        let resumed_second = load(&persistent, &second_tab, "ROOM").unwrap().unwrap();
        assert_eq!(resumed_second.credential, second.credential);
        assert_eq!(resumed_second.binding, second.binding);
        let resumed_first = load(&persistent, &first_tab, "ROOM").unwrap().unwrap();
        assert_eq!(resumed_first.credential, first.credential);
        assert_eq!(resumed_first.binding, first.binding);
    }
}
