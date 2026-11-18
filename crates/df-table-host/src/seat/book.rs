//! The players books of the dev host: the seat book (key hash to seat), the friends book (the
//! account holder's friend profiles; the dev host has no accounts, so this stands in for them),
//! the hero library and the requests waiting for the host's approval.
//!
//! The three books are files in the playground's `players` directory, each written whole or not at
//! all; the requests live in memory only, so a phone whose request a restart forgot asks again.
//! Every decision here is plain and synchronous over `Books`; reading and writing the files is
//! blocking work the caller runs off the async workers.

use std::io::ErrorKind;
use std::path::Path;

use df_playground::look::wire::{HeroPack, LookDraft, PackItem, PackStatus};
use df_playground::seat::wire::{FriendCard, HeroCard, JoinStatus, PlayerKind, SeatCard};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::key::KeyHash;
use super::request::{PendingLine, PendingState, PlayersReport, SeatError, SeatLine};
use crate::engine::limits::DEV_SEAT_COUNT;
use crate::io_cause::IoCause;
use crate::limits::{MAXIMUM_FRIEND_PROFILES, MAXIMUM_HEROES, MAXIMUM_PENDING_APPROVALS};

/// The directory of the players books, under the playground's directory.
pub(crate) const PLAYERS_DIRECTORY: &str = "players";
const SEAT_BOOK_FILE: &str = "seat-book.json";
const FRIENDS_FILE: &str = "friends.json";
const HEROES_FILE: &str = "heroes.json";

/// Who holds a seat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Holder {
    Guest,
    Friend { profile: u64 },
}

impl Holder {
    fn kind(self) -> PlayerKind {
        match self {
            Self::Guest => PlayerKind::Guest,
            Self::Friend { .. } => PlayerKind::Friend,
        }
    }

    fn profile(self) -> Option<u64> {
        match self {
            Self::Guest => None,
            Self::Friend { profile } => Some(profile),
        }
    }
}

/// One seat the book gave a key. Times are milliseconds since the Unix epoch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SeatRecord {
    pub(crate) key: KeyHash,
    pub(crate) seat: u64,
    pub(crate) holder: Holder,
    pub(crate) name: String,
    pub(crate) joined_ms: u64,
    pub(crate) last_seen_ms: u64,
    /// The hero the player chose for this seat; none until they choose one.
    #[serde(default)]
    pub(crate) hero: Option<u64>,
    /// The persona the seat's hero plays (`danny`); only this seat speaks for it from a phone.
    #[serde(default)]
    pub(crate) persona: Option<String>,
}

/// A friend profile of the account holder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FriendProfile {
    pub(crate) id: u64,
    pub(crate) name: String,
}

/// Whose a hero is: a guest's key, or a friend profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum HeroOwner {
    Key { key: KeyHash },
    Friend { profile: u64 },
}

/// One hero of the library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeroRecord {
    pub(crate) id: u64,
    pub(crate) owner: HeroOwner,
    pub(crate) name: String,
    pub(crate) species: String,
    pub(crate) class: String,
    pub(crate) portrait: Option<String>,
    /// The pictures made from the portrait at lock-in (`look`).
    #[serde(default)]
    pub(crate) pack: HeroPack,
    /// Whether the look is locked in. The one mark of a finished hero: set at lock-in whatever
    /// became of its pictures (REF-059), so a hero whose portrait failed still plays.
    #[serde(default)]
    pub(crate) locked: bool,
    /// The look the hero was locked in with, read by Jev and checked: a portrait that failed is
    /// made again from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) look: Option<LookDraft>,
    /// A sheet recorded before the pack existed: read into `pack` when the library loads.
    #[serde(default, rename = "sheet", skip_serializing)]
    pub(crate) legacy_sheet: Option<String>,
    /// A ready picture recorded before the pack existed: read into `pack` when the library loads.
    #[serde(default, rename = "ready", skip_serializing)]
    pub(crate) legacy_ready: Option<String>,
    pub(crate) last_played_ms: Option<u64>,
}

impl HeroRecord {
    /// The record as the pack keeps it: an older record's sheet and ready picture are made pack
    /// items, and an item a stopped run left pending is not made yet.
    fn settled(mut self) -> Self {
        // A hero with a portrait from before the lock mark was locked in.
        self.locked |= self.portrait.is_some();
        if let Some(picture) = self.legacy_sheet.take() {
            self.pack.sheet = PackStatus::Ready { picture };
        }
        if let Some(picture) = self.legacy_ready.take() {
            self.pack.ready = PackStatus::Ready { picture };
        }
        for item in PackItem::ALL {
            if *self.pack.get(item) == PackStatus::Pending {
                self.pack.set(item, PackStatus::Missing);
            }
        }
        self
    }
}

/// A request for a seat as a friend, waiting for the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingApproval {
    pub(crate) request: u64,
    pub(crate) key: KeyHash,
    pub(crate) profile: u64,
    pub(crate) asked_ms: u64,
    pub(crate) state: PendingState,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SeatBookFile {
    seats: Vec<SeatRecord>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FriendsFile {
    friends: Vec<FriendProfile>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct HeroesFile {
    heroes: Vec<HeroRecord>,
}

/// Which book a change touched, so only that file is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Book {
    Seats,
    Friends,
    Heroes,
}

/// The bytes of one book to write, made under the lock and written after.
#[derive(Debug)]
pub(crate) struct BookWrite {
    file_name: &'static str,
    bytes: Vec<u8>,
}

/// The players books in memory.
#[derive(Debug, Default)]
pub(crate) struct Books {
    seats: Vec<SeatRecord>,
    friends: Vec<FriendProfile>,
    heroes: Vec<HeroRecord>,
    pending: Vec<PendingApproval>,
    next_request: u64,
}

fn read_book<T: DeserializeOwned + Default>(directory: &Path, name: &str) -> Result<T, SeatError> {
    let bytes = match std::fs::read(directory.join(name)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(T::default()),
        Err(error) => {
            return Err(SeatError::Storage {
                cause: IoCause::from_kind(error.kind()),
            });
        }
    };
    serde_json::from_slice(&bytes).map_err(|_| SeatError::Storage {
        cause: IoCause::InvalidData,
    })
}

impl Books {
    /// Reads the three books from `directory`; a missing file is an empty book. Blocking.
    pub(crate) fn load(directory: &Path) -> Result<Self, SeatError> {
        let SeatBookFile { mut seats } = read_book(directory, SEAT_BOOK_FILE)?;
        let FriendsFile { friends } = read_book(directory, FRIENDS_FILE)?;
        let HeroesFile { heroes } = read_book(directory, HEROES_FILE)?;
        let heroes = heroes.into_iter().map(HeroRecord::settled).collect();
        seats.sort_by_key(|record| record.seat);
        Ok(Self {
            seats,
            friends,
            heroes,
            pending: Vec::new(),
            next_request: 1,
        })
    }

    /// The bytes of `book` as it stands.
    pub(crate) fn write_of(&self, book: Book) -> Result<BookWrite, SeatError> {
        let unwritable = |_| SeatError::Storage {
            cause: IoCause::InvalidData,
        };
        let (file_name, bytes) = match book {
            Book::Seats => (
                SEAT_BOOK_FILE,
                serde_json::to_vec_pretty(&SeatBookFile {
                    seats: self.seats.clone(),
                }),
            ),
            Book::Friends => (
                FRIENDS_FILE,
                serde_json::to_vec_pretty(&FriendsFile {
                    friends: self.friends.clone(),
                }),
            ),
            Book::Heroes => (
                HEROES_FILE,
                serde_json::to_vec_pretty(&HeroesFile {
                    heroes: self.heroes.clone(),
                }),
            ),
        };
        Ok(BookWrite {
            file_name,
            bytes: bytes.map_err(unwritable)?,
        })
    }

    pub(crate) fn seat_of(&self, key: &KeyHash) -> Option<&SeatRecord> {
        self.seats.iter().find(|record| &record.key == key)
    }

    pub(crate) fn record_at(&self, seat: u64) -> Option<&SeatRecord> {
        self.seats.iter().find(|record| record.seat == seat)
    }

    /// The seat friend `profile` holds, from whichever phone.
    pub(crate) fn profile_seat(&self, profile: u64) -> Option<u64> {
        self.seats
            .iter()
            .find(|record| record.holder == Holder::Friend { profile })
            .map(|record| record.seat)
    }

    /// The seats of the table (1 to `DEV_SEAT_COUNT`, REF-062) no key holds, lowest first.
    pub(crate) fn free_seats(&self) -> Vec<u64> {
        (1..=DEV_SEAT_COUNT)
            .filter(|seat| self.record_at(*seat).is_none())
            .collect()
    }

    /// The seats the book holds, by seat.
    pub(crate) fn records(&self) -> &[SeatRecord] {
        &self.seats
    }

    pub(crate) fn friend(&self, profile: u64) -> Option<&FriendProfile> {
        self.friends.iter().find(|friend| friend.id == profile)
    }

    pub(crate) fn friend_cards(&self) -> Vec<FriendCard> {
        self.friends
            .iter()
            .map(|friend| FriendCard {
                id: friend.id,
                name: friend.name.clone(),
            })
            .collect()
    }

    /// Gives `seat` to `key` (or moves `key` there), as `holder` named `name`. A record that held
    /// the seat under another key is replaced: the seat is handed over.
    pub(crate) fn seat(
        &mut self,
        key: &KeyHash,
        seat: u64,
        holder: Holder,
        name: &str,
        now_ms: u64,
    ) -> SeatCard {
        let kept = self.seat_of(key).filter(|record| record.seat == seat);
        let joined_ms = kept.map_or(now_ms, |record| record.joined_ms);
        let hero = kept.and_then(|record| record.hero);
        let persona = kept.and_then(|record| record.persona.clone());
        self.seats
            .retain(|record| &record.key != key && record.seat != seat);
        let record = SeatRecord {
            key: key.clone(),
            seat,
            holder,
            name: name.to_owned(),
            joined_ms,
            last_seen_ms: now_ms,
            hero,
            persona,
        };
        let card = self.card_of(&record);
        self.seats.push(record);
        self.seats.sort_by_key(|record| record.seat);
        card
    }

    /// Says the player at `key` was seen now. `false` when the key holds no seat.
    pub(crate) fn touch(&mut self, key: &KeyHash, now_ms: u64) -> bool {
        let Some(record) = self.seats.iter_mut().find(|record| &record.key == key) else {
            return false;
        };
        record.last_seen_ms = now_ms;
        true
    }

    /// Frees `seat`; the record it held, if any.
    pub(crate) fn free(&mut self, seat: u64) -> Option<SeatRecord> {
        let index = self.seats.iter().position(|record| record.seat == seat)?;
        Some(self.seats.remove(index))
    }

    /// A request for a seat as friend `profile`. A key that already waits keeps its request, now
    /// for `profile`; a full list first forgets the oldest refusal no phone read, then refuses.
    pub(crate) fn ask_as_friend(
        &mut self,
        key: &KeyHash,
        profile: u64,
        now_ms: u64,
    ) -> Result<u64, SeatError> {
        if self.friend(profile).is_none() {
            return Err(SeatError::UnknownFriend { profile });
        }
        self.pending
            .retain(|pending| &pending.key != key || pending.state == PendingState::Waiting);
        if let Some(waiting) = self.pending.iter_mut().find(|pending| &pending.key == key) {
            waiting.profile = profile;
            waiting.asked_ms = now_ms;
            return Ok(waiting.request);
        }
        if self.pending.len() >= MAXIMUM_PENDING_APPROVALS {
            let refused = self
                .pending
                .iter()
                .position(|pending| pending.state == PendingState::Refused)
                .ok_or(SeatError::PendingFull)?;
            self.pending.remove(refused);
        }
        let request = self.next_request;
        self.next_request = self.next_request.saturating_add(1);
        self.pending.push(PendingApproval {
            request,
            key: key.clone(),
            profile,
            asked_ms: now_ms,
            state: PendingState::Waiting,
        });
        Ok(request)
    }

    /// The waiting request numbered `request`.
    pub(crate) fn waiting(&self, request: u64) -> Result<&PendingApproval, SeatError> {
        self.pending
            .iter()
            .find(|pending| pending.request == request && pending.state == PendingState::Waiting)
            .ok_or(SeatError::UnknownRequest { request })
    }

    /// Takes the waiting request numbered `request` off the list: it was approved.
    pub(crate) fn take_waiting(&mut self, request: u64) -> Result<PendingApproval, SeatError> {
        self.waiting(request)?;
        let index = self
            .pending
            .iter()
            .position(|pending| pending.request == request)
            .ok_or(SeatError::UnknownRequest { request })?;
        Ok(self.pending.remove(index))
    }

    /// Marks the waiting request numbered `request` refused, until its phone reads that.
    pub(crate) fn refuse(&mut self, request: u64) -> Result<u64, SeatError> {
        self.waiting(request)?;
        let pending = self
            .pending
            .iter_mut()
            .find(|pending| pending.request == request)
            .ok_or(SeatError::UnknownRequest { request })?;
        pending.state = PendingState::Refused;
        Ok(pending.profile)
    }

    /// Where the key's request for a seat stands. A refusal is told once, then forgotten.
    pub(crate) fn status(&mut self, key: &KeyHash) -> JoinStatus {
        if let Some(record) = self.seat_of(key) {
            return JoinStatus::Seated {
                seat: self.card_of(record),
            };
        }
        let Some(index) = self.pending.iter().position(|pending| &pending.key == key) else {
            return JoinStatus::NotAsked;
        };
        match self.pending[index].state {
            PendingState::Waiting => JoinStatus::AwaitingApproval {
                request: self.pending[index].request,
            },
            PendingState::Refused => {
                self.pending.remove(index);
                JoinStatus::Refused
            }
        }
    }

    pub(crate) fn add_friend(&mut self, name: &str) -> Result<FriendCard, SeatError> {
        if self.friends.len() >= MAXIMUM_FRIEND_PROFILES {
            return Err(SeatError::FriendsFull);
        }
        let id = self
            .friends
            .iter()
            .map(|friend| friend.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        self.friends.push(FriendProfile {
            id,
            name: name.to_owned(),
        });
        Ok(FriendCard {
            id,
            name: name.to_owned(),
        })
    }

    /// Removes a friend profile and its waiting requests. A seat it holds stays until it leaves.
    pub(crate) fn remove_friend(&mut self, profile: u64) -> Result<(), SeatError> {
        let before = self.friends.len();
        self.friends.retain(|friend| friend.id != profile);
        if self.friends.len() == before {
            return Err(SeatError::UnknownFriend { profile });
        }
        self.pending.retain(|pending| pending.profile != profile);
        Ok(())
    }

    /// The owner of the heroes of the player at `record`.
    fn hero_owner(record: &SeatRecord) -> HeroOwner {
        match record.holder {
            Holder::Guest => HeroOwner::Key {
                key: record.key.clone(),
            },
            Holder::Friend { profile } => HeroOwner::Friend { profile },
        }
    }

    /// The heroes the player at `key` may see: a guest's under their key, a friend's under their
    /// profile. Nobody sees another player's heroes.
    /// Makes the hero at `seat` play `persona`, or no persona. A persona plays at one seat only.
    pub(crate) fn bind_persona(
        &mut self,
        seat: u64,
        persona: Option<&str>,
    ) -> Result<(), SeatError> {
        if let Some(other) = self
            .seats
            .iter()
            .find(|record| record.seat != seat && record.persona.as_deref() == persona)
            .filter(|_| persona.is_some())
        {
            return Err(SeatError::PersonaTaken { seat: other.seat });
        }
        let record = self
            .seats
            .iter_mut()
            .find(|record| record.seat == seat)
            .ok_or(SeatError::UnknownSeat { seat })?;
        record.persona = persona.map(str::to_owned);
        Ok(())
    }

    /// Makes `hero`, one of the player's own heroes, the one they play at their seat.
    pub(crate) fn choose_hero(&mut self, key: &KeyHash, hero: u64) -> Result<SeatCard, SeatError> {
        let record = self.seat_of(key).ok_or(SeatError::UnknownKey)?;
        let owner = Self::hero_owner(record);
        let theirs = self
            .heroes
            .iter()
            .any(|known| known.id == hero && known.owner == owner);
        if !theirs {
            return Err(SeatError::UnknownHero { hero });
        }
        let record = self
            .seats
            .iter_mut()
            .find(|record| &record.key == key)
            .ok_or(SeatError::UnknownKey)?;
        record.hero = Some(hero);
        let record = record.clone();
        Ok(self.card_of(&record))
    }

    /// The seat `record` as its phone is told it: whether the chosen hero's look is locked in comes
    /// from the hero library (a locked look has its approved portrait).
    pub(crate) fn card_of(&self, record: &SeatRecord) -> SeatCard {
        SeatCard {
            seat: record.seat,
            name: record.name.clone(),
            kind: record.holder.kind(),
            hero: record.hero,
            hero_locked: record
                .hero
                .and_then(|hero| self.hero(hero))
                .is_some_and(|hero| hero.locked),
            plays: record.persona.clone(),
        }
    }

    pub(crate) fn hero(&self, hero: u64) -> Option<&HeroRecord> {
        self.heroes.iter().find(|known| known.id == hero)
    }

    fn hero_mut(&mut self, hero: u64) -> Result<&mut HeroRecord, SeatError> {
        self.heroes
            .iter_mut()
            .find(|known| known.id == hero)
            .ok_or(SeatError::UnknownHero { hero })
    }

    /// Points `hero` at its locked-in portrait (`look`).
    pub(crate) fn set_portrait(
        &mut self,
        hero: u64,
        portrait: Option<String>,
    ) -> Result<(), SeatError> {
        self.hero_mut(hero)?.portrait = portrait;
        Ok(())
    }

    /// Sets how item `item` of `hero`'s pack stands (`look`).
    pub(crate) fn set_pack(
        &mut self,
        hero: u64,
        item: PackItem,
        status: PackStatus,
    ) -> Result<(), SeatError> {
        self.hero_mut(hero)?.pack.set(item, status);
        Ok(())
    }

    /// Locks `hero` in with its portrait (none when it failed) and its look, every pack item
    /// pending, in one change, so nothing else starts the pack meanwhile.
    pub(crate) fn lock_hero(
        &mut self,
        hero: u64,
        portrait: Option<String>,
        look: Option<LookDraft>,
    ) -> Result<(), SeatError> {
        let record = self.hero_mut(hero)?;
        record.locked = true;
        record.portrait = portrait;
        record.look = look;
        for item in PackItem::ALL {
            record.pack.set(item, PackStatus::Pending);
        }
        Ok(())
    }

    /// Claims the pack items of `hero` to make, marking them pending in the same change: `item`
    /// unless it is being made, or every item `unmade` takes. The portrait goes first when the
    /// hero has none, as every other item is made from it.
    pub(crate) fn claim_pack(
        &mut self,
        hero: u64,
        item: Option<PackItem>,
        unmade: impl Fn(&PackStatus) -> bool,
    ) -> Result<Vec<PackItem>, SeatError> {
        let record = self.hero_mut(hero)?;
        // A hero not locked in has no pack to make yet.
        if !record.locked {
            return Ok(Vec::new());
        }
        let mut items: Vec<PackItem> = match item {
            Some(item) if *record.pack.get(item) != PackStatus::Pending => vec![item],
            Some(_) => Vec::new(),
            None => PackItem::ALL
                .into_iter()
                .filter(|item| unmade(record.pack.get(*item)))
                .collect(),
        };
        if record.portrait.is_none() {
            items.sort_by_key(|item| *item != PackItem::Portrait);
        }
        for item in &items {
            record.pack.set(*item, PackStatus::Pending);
        }
        Ok(items)
    }

    pub(crate) fn heroes_of(&self, key: &KeyHash) -> Result<Vec<HeroCard>, SeatError> {
        let record = self.seat_of(key).ok_or(SeatError::UnknownKey)?;
        let owner = Self::hero_owner(record);
        Ok(self
            .heroes
            .iter()
            .filter(|hero| hero.owner == owner)
            .map(hero_card)
            .collect())
    }

    /// Adds a hero for the player at `seat`.
    pub(crate) fn add_hero(
        &mut self,
        seat: u64,
        name: &str,
        species: &str,
        class: &str,
    ) -> Result<HeroCard, SeatError> {
        let record = self
            .record_at(seat)
            .ok_or(SeatError::UnknownSeat { seat })?;
        if self.heroes.len() >= MAXIMUM_HEROES {
            return Err(SeatError::HeroesFull);
        }
        let id = self
            .heroes
            .iter()
            .map(|hero| hero.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let hero = HeroRecord {
            id,
            owner: Self::hero_owner(record),
            name: name.to_owned(),
            species: species.to_owned(),
            class: class.to_owned(),
            portrait: None,
            pack: HeroPack::default(),
            locked: false,
            look: None,
            legacy_sheet: None,
            legacy_ready: None,
            last_played_ms: None,
        };
        let card = hero_card(&hero);
        self.heroes.push(hero);
        Ok(card)
    }

    /// The books as `players` lists them.
    pub(crate) fn report(&self, room_code: String) -> PlayersReport {
        PlayersReport {
            room_code,
            seats: self
                .seats
                .iter()
                .map(|record| SeatLine {
                    seat: record.seat,
                    name: record.name.clone(),
                    kind: record.holder.kind(),
                    profile: record.holder.profile(),
                    joined_ms: record.joined_ms,
                    last_seen_ms: record.last_seen_ms,
                    persona: record.persona.clone(),
                })
                .collect(),
            friends: self.friend_cards(),
            pending: self
                .pending
                .iter()
                .map(|pending| PendingLine {
                    request: pending.request,
                    profile: pending.profile,
                    name: self
                        .friend(pending.profile)
                        .map_or_else(String::new, |friend| friend.name.clone()),
                    asked_ms: pending.asked_ms,
                    state: pending.state,
                })
                .collect(),
            heroes: self.heroes.len(),
        }
    }
}

fn hero_card(hero: &HeroRecord) -> HeroCard {
    HeroCard {
        id: hero.id,
        name: hero.name.clone(),
        species: hero.species.clone(),
        class: hero.class.clone(),
        portrait: hero.portrait.clone(),
        pack: hero.pack.clone(),
        last_played_ms: hero.last_played_ms,
    }
}

/// Writes one book's bytes into `directory`, whole or not at all. Blocking.
pub(crate) fn write_book(directory: &Path, write: &BookWrite) -> Result<(), SeatError> {
    crate::atomic_file::write_atomically(directory, write.file_name, &write.bytes).map_err(
        |error| SeatError::Storage {
            cause: IoCause::from_kind(error.kind()),
        },
    )
}
