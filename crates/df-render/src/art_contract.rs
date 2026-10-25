//! The art contract: the one rule for every picture the game shows or asks an image model to
//! copy. The game's look is documentary photography. Painted or illustrated art never reaches a
//! screen (the TV display, a phone, the battle map) and never reaches an image-generation
//! request.
//!
//! # Depictions and chrome
//!
//! A *depiction* is any picture of a person, a creature, a place, a ground or a prop. Its stem
//! starts with `portrait-`, `figure-`, `sprite-`, `scene-`, `ground-` or `prop-` ([`class_of`]).
//! Everything else is *chrome* (icons, frames, dividers, dice, ribbons, status icons, the
//! wordmark, buttons, sound) and is allowed as it is.
//!
//! A depiction may come from exactly two sources:
//!
//! 1. **Live generated photographic imagery**, served by key under `/gameplay/imagery/<key>`.
//! 2. **A reviewed photographic library asset**: a `-documentary-photo` file under
//!    `assets/ui/generated`, listed in [`REVIEWED_PHOTOS`] and in
//!    `documentary-photo-v2.assets.json`.
//!
//! There is no third source. The painted files that used to sit beside the reviewed ones are
//! deleted, and the server's static route, the image-generation reference loader, content
//! validation and the browser renderer all refuse to name one ([`is_servable_stem`],
//! [`check_content_reference`], [`fetch_target`]).
//!
//! # Three kinds of reference string
//!
//! * A **library stem** names a file: a reviewed photograph (chrome stems name chrome files).
//! * An **identity key** names a *subject*, never a file: `portrait-hero-2` is hero seat two,
//!   `portrait-ant-wolf` is the wolf creature family. [`identity_photo`] maps a key to its
//!   reviewed photograph when one exists; otherwise the key has no library picture and the
//!   role's fallback order applies.
//! * A **role token** says which source to use when content has no reviewed photograph:
//!   [`ROOM_PHOTO`] (the place's live room photograph or current scene plate),
//!   [`CANONICAL_PHOTO`] (the character's live canonical photograph) or [`NEUTRAL`] (no
//!   picture; the neutral chrome stands). Content (packs, battle scenes, cast entries) may use
//!   library stems that are reviewed, role tokens and live paths, nothing else
//!   ([`check_content_reference`]).
//!
//! # One resolver, one fallback order per role
//!
//! [`plan`] takes a [`Role`] and a request (a stem, a path, an identity key or a token) and
//! returns a [`Plan`]: the reviewed photograph that answers it (if any), the live source that
//! serves the role, and the neutral fallback. It never names a painted file. Server and
//! clients walk the plan in the same order ([`Plan::steps`]):
//!
//! 1. A live image already generated and cached for this subject.
//! 2. A reviewed `-documentary-photo` asset.
//! 3. Request the live generation now ([`Step::RequestLive`]). Until it is ready, show the
//!    neutral chrome fallback (silhouette, initial, plain floor or block); never painted art.
//! 4. If generation fails, keep the neutral fallback and record it in the audit log.
//!
//! # Generation is on demand, bounded and server-only
//!
//! A depiction without a reviewed photograph is generated the first time it is needed, through
//! the existing live photographic imagery path (the canonical character path for people and
//! creatures, the canonical room or scene plate for places), and cached by key like every other
//! live image. Nothing is generated ahead of time and no library is pre-generated. Only the
//! server requests it, at the point the depiction is first needed in a view; a test or a client
//! render loop never does. The request is bounded and idempotent per [`Subject`]: at most one
//! request is in flight for a subject key, the imagery worker's own concurrency (nothing caps spend)
//! bounds the rest, and each state change is written to the `ART`, `RESOURCE` and `IMAGERY` audit lines with the
//! subject key. A failure keeps the neutral fallback.
//!
//! # Roles and their live sources
//!
//! | Role | Live source (steps 1 and 3) | Neutral (step 4) |
//! | --- | --- | --- |
//! | [`Role::BattleBackdrop`] | room photograph, then scene plate | none (dark stage) |
//! | [`Role::BattleGround`] | room photograph | plain floor colour |
//! | [`Role::BattleProp`] | room photograph | neutral block |
//! | [`Role::CharacterToken`] | canonical character photograph | silhouette or initial |
//! | [`Role::HeroPortrait`] | canonical character photograph | silhouette or initial |
//! | [`Role::LobbyArt`] | room photograph, then scene plate | none |
//! | [`Role::StagePlace`] | scene plate, then room photograph | none |
//! | [`Role::GenerationReference`] | none | the neutral photographic carrier |
//!
//! A ground or prop is drawn as a plain floor or neutral block; the place's own room
//! photograph is the live source that sets it, so no separate ground or prop image is ever
//! generated.

/// The served route of the generated-art directory.
pub const GENERATED_ROUTE: &str = "/assets/ui/generated";

/// The route prefix of live generated images; the rest is the image key.
pub const LIVE_IMAGERY_PREFIX: &str = "/gameplay/imagery/";

/// Role token: the place's live room photograph or current scene plate.
pub const ROOM_PHOTO: &str = "live-room-photo";

/// Role token: the character's live canonical photograph.
pub const CANONICAL_PHOTO: &str = "live-canonical-photo";

/// Role token: no picture; the neutral chrome fallback stands.
pub const NEUTRAL: &str = "neutral";

/// Suffix that marks a reviewed photographic library asset.
pub const REVIEWED_SUFFIX: &str = "-documentary-photo";

/// Every reviewed photographic library asset, by file stem. Each is listed in
/// `assets/ui/generated/documentary-photo-v2.assets.json` and exists on disk (tests check both).
pub const REVIEWED_PHOTOS: [&str; 10] = [
    "sprite-courier-documentary-photo",
    "sprite-bandit-documentary-photo",
    "scene-salt-quay-arrival-documentary-photo",
    "scene-salt-quay-candid-documentary-photo",
    "portrait-patron-merchant-documentary-photo",
    "portrait-shop-adventurer-documentary-photo",
    "portrait-shop-soldier-documentary-photo",
    "sprite-patron-merchant-documentary-photo",
    "sprite-shop-adventurer-documentary-photo",
    "sprite-shop-soldier-documentary-photo",
];

/// The reviewed harbor photograph: lobby art and the establishing picture of the quay.
pub const HARBOR_PHOTO: &str = "scene-salt-quay-arrival-documentary-photo";

/// The neutral photographic carrier the image edit endpoint needs when no reference exists. It is
/// a checked-in empty photograph under `assets/scenes`, not a library depiction.
pub const NEUTRAL_CARRIER: &str = "neutral-photographic-carrier";

const DEPICTION_PREFIXES: [&str; 6] = [
    "portrait-",
    "figure-",
    "sprite-",
    "scene-",
    "ground-",
    "prop-",
];

/// Whether a stem is a picture of something or interface chrome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtClass {
    Chrome,
    Depiction,
}

/// Classifies a bare stem by its prefix.
pub fn class_of(stem: &str) -> ArtClass {
    if DEPICTION_PREFIXES
        .iter()
        .any(|prefix| stem.starts_with(prefix))
    {
        ArtClass::Depiction
    } else {
        ArtClass::Chrome
    }
}

/// The stem of a reference: the last path segment without a query and without a `.webp`, `.png`
/// or `.jpg` extension. `/assets/ui/generated/portrait-x.webp` gives `portrait-x`.
pub fn stem_of(reference: &str) -> &str {
    let name = reference.rsplit('/').next().unwrap_or(reference);
    let name = name.split(['?', '#']).next().unwrap_or(name);
    for extension in [".webp", ".png", ".jpg"] {
        if let Some(stem) = name.strip_suffix(extension) {
            return stem;
        }
    }
    name
}

/// The reviewed photograph a stem names, whatever extension or path it came with.
pub fn reviewed_photo(reference: &str) -> Option<&'static str> {
    let stem = stem_of(reference);
    REVIEWED_PHOTOS.iter().copied().find(|known| *known == stem)
}

/// The reviewed photograph that answers an identity key, if one exists. The keys name subjects
/// (a hero seat, a courier, a shop patron), never files.
pub fn identity_photo(key: &str) -> Option<&'static str> {
    let key = stem_of(key);
    let key = key.strip_suffix("-full").unwrap_or(key);
    let name = DEPICTION_PREFIXES
        .iter()
        .take(3)
        .find_map(|prefix| key.strip_prefix(prefix))?;
    let (figure, sprite, portrait) = match name {
        "courier" => (
            "sprite-courier-documentary-photo",
            "sprite-courier-documentary-photo",
            "sprite-courier-documentary-photo",
        ),
        "bandit" => (
            "sprite-bandit-documentary-photo",
            "sprite-bandit-documentary-photo",
            "sprite-bandit-documentary-photo",
        ),
        "patron-merchant" => (
            "sprite-patron-merchant-documentary-photo",
            "sprite-patron-merchant-documentary-photo",
            "portrait-patron-merchant-documentary-photo",
        ),
        "shop-adventurer" => (
            "sprite-shop-adventurer-documentary-photo",
            "sprite-shop-adventurer-documentary-photo",
            "portrait-shop-adventurer-documentary-photo",
        ),
        "shop-soldier" => (
            "sprite-shop-soldier-documentary-photo",
            "sprite-shop-soldier-documentary-photo",
            "portrait-shop-soldier-documentary-photo",
        ),
        _ => return None,
    };
    Some(if key.starts_with("figure-") {
        figure
    } else if key.starts_with("sprite-") {
        sprite
    } else {
        portrait
    })
}

/// The reviewed full-body photograph of a reference, when one exists: the reviewed `figure-`
/// photograph that goes with a non-player library reference, when one exists.
/// Player heroes require their own accepted image.
pub fn figure_photo(reference: &str) -> Option<&'static str> {
    let photo = match classify(reference) {
        Reference::Reviewed(photo) => photo,
        Reference::Identity {
            photo: Some(photo), ..
        } => photo,
        _ => return None,
    };
    let name = photo
        .strip_prefix("sprite-")
        .or_else(|| photo.strip_prefix("figure-"))
        .or_else(|| photo.strip_prefix("portrait-"))?;
    REVIEWED_PHOTOS
        .iter()
        .copied()
        .find(|known| known.strip_prefix("figure-") == Some(name))
}

/// Whether a stem is an identity key: it names a subject and never a file. The hero seats and
/// the courier, bandit and shop cast have reviewed photographs; every `ant-` creature family key
/// has none and relies on the live canonical photograph.
pub fn is_identity_key(stem: &str) -> bool {
    let stem = stem_of(stem);
    if identity_photo(stem).is_some() {
        return true;
    }
    let stem = stem.strip_suffix("-full").unwrap_or(stem);
    DEPICTION_PREFIXES.iter().take(3).any(|prefix| {
        stem.strip_prefix(prefix).is_some_and(|name| {
            matches!(name, "hero-1" | "hero-2" | "hero-3") || creature_family(name)
        })
    })
}

fn creature_family(name: &str) -> bool {
    name.strip_prefix("ant-").is_some_and(|rest| {
        !rest.is_empty()
            && rest
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    })
}

/// The creature family an identity key names: `portrait-ant-wolf` gives `ant-wolf`.
pub fn creature_of(stem: &str) -> &str {
    let stem = stem_of(stem);
    let base = DEPICTION_PREFIXES
        .iter()
        .take(3)
        .find_map(|prefix| stem.strip_prefix(prefix))
        .unwrap_or(stem);
    base.strip_suffix("-full").unwrap_or(base)
}

/// A role token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Token {
    RoomPhoto,
    CanonicalPhoto,
    Neutral,
}

impl Token {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            ROOM_PHOTO => Some(Self::RoomPhoto),
            CANONICAL_PHOTO => Some(Self::CanonicalPhoto),
            NEUTRAL => Some(Self::Neutral),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RoomPhoto => ROOM_PHOTO,
            Self::CanonicalPhoto => CANONICAL_PHOTO,
            Self::Neutral => NEUTRAL,
        }
    }
}

/// A server-published personal hero image. Seat numbers and library art are never identities.
/// Creator bytes are checked by their serving boundary; canonical bytes by the imagery cache.
pub fn player_hero_image(reference: &str) -> Option<&str> {
    let key = reference
        .strip_prefix("/gameplay/imagery/")
        .or_else(|| reference.strip_prefix("/gameplay/creator-portrait/"))?;
    (key.len() == 64
        && key
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    .then_some(reference)
}

/// What a reference string is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Reference<'a> {
    /// No reference at all.
    Empty,
    /// A role token.
    Token(Token),
    /// A live generated image, `/gameplay/imagery/<key>`.
    Live(&'a str),
    /// A reviewed photographic library asset.
    Reviewed(&'static str),
    /// An identity key and the reviewed photograph that answers it, if any.
    Identity {
        key: &'a str,
        photo: Option<&'static str>,
    },
    /// Interface chrome (or any non-depiction stem such as `none`).
    Chrome(&'a str),
    /// A depiction stem that is not reviewed, not an identity key and not a token: a painted
    /// file reference. It resolves to nothing.
    Painted(&'a str),
}

/// Classifies any reference string: a served path, an authored asset path, a bare stem, an
/// identity key, a token or a live path.
pub fn classify(reference: &str) -> Reference<'_> {
    if reference.is_empty() {
        return Reference::Empty;
    }
    if player_hero_image(reference).is_some() || reference.starts_with(LIVE_IMAGERY_PREFIX) {
        return Reference::Live(reference);
    }
    let stem = stem_of(reference);
    if let Some(token) = Token::parse(stem) {
        return Reference::Token(token);
    }
    if let Some(photo) = reviewed_photo(stem) {
        return Reference::Reviewed(photo);
    }
    if class_of(stem) == ArtClass::Chrome {
        return Reference::Chrome(stem);
    }
    if is_identity_key(stem) {
        return Reference::Identity {
            key: stem,
            photo: identity_photo(stem),
        };
    }
    Reference::Painted(stem)
}

/// Whether the server may serve this generated-art stem: chrome or a reviewed photograph. A
/// depiction that is not a reviewed photograph is refused even if a file of that name exists.
pub fn is_servable_stem(stem: &str) -> bool {
    match class_of(stem) {
        ArtClass::Chrome => true,
        ArtClass::Depiction => reviewed_photo(stem).is_some_and(|known| known == stem),
    }
}

/// What a client may fetch for a reference: a live image path, a chrome stem, a reviewed
/// photograph, or the reviewed photograph that answers an identity key. Role tokens, identity keys
/// without a photograph and painted stems have nothing to fetch (); the role's neutral
/// fallback stands.
pub fn fetch_target(reference: &str) -> Option<&str> {
    match classify(reference) {
        Reference::Live(path) => Some(path),
        Reference::Reviewed(photo) => Some(photo),
        Reference::Chrome(stem) => Some(stem),
        Reference::Identity { photo, .. } => photo,
        Reference::Empty | Reference::Token(_) | Reference::Painted(_) => None,
    }
}

/// A content reference that breaks the contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentArtError {
    /// A painted depiction stem.
    Painted,
    /// An identity key; content names a reviewed photograph or a role token instead.
    IdentityKey,
}

/// Validates a depiction reference in content (a pack's scene asset, cast sprite or portrait, a
/// battle scene's ground, backdrop or prop sprite). Accepts nothing, a reviewed photograph, a
/// role token, a live path or a chrome-class value such as `none`. Rejects a painted depiction
/// stem and an identity key.
pub fn check_content_reference(reference: &str) -> Result<(), ContentArtError> {
    match classify(reference) {
        Reference::Painted(_) => Err(ContentArtError::Painted),
        Reference::Identity { .. } => Err(ContentArtError::IdentityKey),
        Reference::Empty
        | Reference::Token(_)
        | Reference::Live(_)
        | Reference::Reviewed(_)
        | Reference::Chrome(_) => Ok(()),
    }
}

/// Where a picture is wanted, which fixes its fallback order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    BattleBackdrop,
    BattleGround,
    BattleProp,
    /// A creature or non-player character token, card or cut-out.
    CharacterToken,
    HeroPortrait,
    LobbyArt,
    /// The picture behind a story scene on the TV.
    StagePlace,
    /// A reference image for an image-generation request.
    GenerationReference,
}

/// The live source that serves a role.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveSource {
    /// The place's canonical room photograph.
    RoomPhoto,
    /// The current composed scene plate.
    ScenePlate,
    /// The character's canonical photograph.
    CanonicalCharacter,
}

/// What stands when no photograph is available (yet, or because generation failed).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Neutral {
    /// A silhouette or the subject's initial.
    Silhouette,
    /// A plain floor colour.
    PlainFloor,
    /// A neutral block or outline.
    Block,
    /// The neutral photographic carrier an edit request needs.
    Carrier,
    /// Nothing is drawn.
    Nothing,
}

/// One step of a fallback order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Step {
    /// A live image already generated and cached for this subject.
    LiveCached(LiveSource),
    /// A reviewed library photograph.
    Reviewed(&'static str),
    /// Ask the server to generate the live image now. Server only; see the module notes.
    RequestLive(LiveSource),
    /// The neutral fallback.
    Neutral(Neutral),
}

/// The resolved plan for one depiction request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Plan {
    pub role: Role,
    /// The reviewed photograph that answers the request, if any.
    pub reviewed: Option<&'static str>,
    /// The live sources in order; the first is also the one requested on demand.
    pub live: &'static [LiveSource],
    pub neutral: Neutral,
}

impl Plan {
    /// The fallback order. Every plan has the same four steps, in this order. A role with no
    /// live source or no reviewed photograph skips the step ([`Plan::walk`] yields only the
    /// steps that exist).
    pub fn steps(&self) -> [Option<Step>; 4] {
        let live = self.live.first().copied();
        [
            live.map(Step::LiveCached),
            self.reviewed.map(Step::Reviewed),
            live.map(Step::RequestLive),
            Some(Step::Neutral(self.neutral)),
        ]
    }

    /// The steps that exist, in order. Cached live sources after the first are tried inside
    /// step one by the caller (`live` lists them in order).
    pub fn walk(&self) -> impl Iterator<Item = Step> {
        self.steps().into_iter().flatten()
    }
}

/// The resolver: the plan for a role and a request. The request may be a library or served
/// path, a bare stem, an identity key, a role token, a live path or empty. Painted stems answer
/// with no reviewed photograph and the role's live source and neutral fallback; they are never
/// named in the plan.
pub fn plan(role: Role, request: &str) -> Plan {
    let reviewed = match classify(request) {
        Reference::Reviewed(photo) => Some(photo),
        Reference::Identity { photo, .. } => photo,
        _ => None,
    };
    let (live, neutral): (&'static [LiveSource], Neutral) = match role {
        Role::BattleBackdrop => (
            &[LiveSource::RoomPhoto, LiveSource::ScenePlate],
            Neutral::Nothing,
        ),
        Role::BattleGround => (&[LiveSource::RoomPhoto], Neutral::PlainFloor),
        Role::BattleProp => (&[LiveSource::RoomPhoto], Neutral::Block),
        Role::CharacterToken | Role::HeroPortrait => {
            (&[LiveSource::CanonicalCharacter], Neutral::Silhouette)
        }
        Role::LobbyArt => (
            &[LiveSource::RoomPhoto, LiveSource::ScenePlate],
            Neutral::Nothing,
        ),
        Role::StagePlace => (
            &[LiveSource::ScenePlate, LiveSource::RoomPhoto],
            Neutral::Nothing,
        ),
        Role::GenerationReference => (&[], Neutral::Carrier),
    };
    let reviewed = match role {
        // A reference for a generation request is a reviewed photograph (a bare base stem whose
        // `-documentary-photo` form is reviewed counts) or the carrier, never a role's live
        // picture.
        Role::GenerationReference if reviewed.is_none() => {
            reviewed_photo(&format!("{}{REVIEWED_SUFFIX}", stem_of(request)))
        }
        _ => reviewed,
    };
    Plan {
        role,
        reviewed,
        live,
        neutral,
    }
}

/// The served URL of the reviewed photograph that answers a request for a role, if any.
pub fn library_url(role: Role, request: &str) -> Option<String> {
    plan(role, request)
        .reviewed
        .map(|photo| format!("{GENERATED_ROUTE}/{photo}"))
}

/// What a subject is, for on-demand generation.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SubjectKind {
    /// A person or creature, generated through the canonical character path.
    Character,
    /// A place, generated through the canonical room path.
    Place,
}

/// A thing a view needs a photograph of. Its [`Subject::key`] identifies the single in-flight
/// generation request for it and appears in the `IMAGERY` audit lines.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Subject {
    pub kind: SubjectKind,
    pub id: String,
}

impl Subject {
    pub fn character(id: impl Into<String>) -> Self {
        Self {
            kind: SubjectKind::Character,
            id: id.into(),
        }
    }

    pub fn place(id: impl Into<String>) -> Self {
        Self {
            kind: SubjectKind::Place,
            id: id.into(),
        }
    }

    /// `character:<id>` or `place:<id>`.
    pub fn key(&self) -> String {
        let kind = match self.kind {
            SubjectKind::Character => "character",
            SubjectKind::Place => "place",
        };
        format!("{kind}:{}", self.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_images_are_personal_server_paths_and_never_stock_characters() {
        for prefix in ["/gameplay/imagery/", "/gameplay/creator-portrait/"] {
            let path = format!("{prefix}{}", "a".repeat(64));
            assert_eq!(player_hero_image(&path), Some(path.as_str()));
            assert_eq!(fetch_target(&path), Some(path.as_str()));
        }
        for rejected in [
            "portrait-hero-1",
            "figure-hero-2",
            "sprite-hero-3-documentary-photo",
            "/gameplay/imagery/short",
            "/gameplay/creator-portrait/../../secret",
            "https://camera.invalid/raw.jpg",
        ] {
            assert!(player_hero_image(rejected).is_none());
        }
        for seat in [1, 2, 3] {
            assert!(identity_photo(&format!("portrait-hero-{seat}")).is_none());
            assert!(fetch_target(&format!("sprite-hero-{seat}-documentary-photo")).is_none());
        }
    }

    #[test]
    fn chrome_is_not_a_depiction_and_depictions_are_by_prefix() {
        for stem in [
            "icon-sword",
            "status-dying",
            "panel-frame",
            "d20",
            "wordmark",
            "divider",
        ] {
            assert_eq!(class_of(stem), ArtClass::Chrome, "{stem}");
        }
        for stem in [
            "portrait-x",
            "figure-x",
            "sprite-x",
            "scene-x",
            "ground-x",
            "prop-x",
        ] {
            assert_eq!(class_of(stem), ArtClass::Depiction, "{stem}");
        }
    }

    #[test]
    fn only_chrome_and_reviewed_photographs_are_servable() {
        assert!(is_servable_stem("icon-attack"));
        assert!(!is_servable_stem("figure-hero-1-documentary-photo"));
        for painted in [
            "portrait-hero-1",
            "figure-hero-1",
            "figure-hero-1-photo",
            "scene-salt-quay-arrival",
            "ground-salt-quay",
            "prop-crates",
            "sprite-ant-wolf",
            "scene-place-guardhouse-documentary-photo",
        ] {
            assert!(!is_servable_stem(painted), "{painted}");
        }
    }

    #[test]
    fn identity_keys_map_to_reviewed_photographs_or_to_nothing() {
        assert_eq!(identity_photo("portrait-hero-2"), None);
        assert_eq!(
            identity_photo("/assets/ui/generated/sprite-hero-3.webp"),
            None
        );
        assert_eq!(
            identity_photo("portrait-patron-merchant-full"),
            Some("portrait-patron-merchant-documentary-photo")
        );
        assert_eq!(
            identity_photo("sprite-bandit"),
            Some("sprite-bandit-documentary-photo")
        );
        assert_eq!(identity_photo("portrait-ant-wolf"), None);
        assert!(is_identity_key("portrait-ant-wolf"));
        assert!(is_identity_key("figure-ant-goblin-warrior"));
        assert!(!is_identity_key("portrait-villain-lich"));
        assert!(!is_identity_key("scene-place-inn-taproom"));
        assert_eq!(creature_of("portrait-ant-wolf-full"), "ant-wolf");
    }

    #[test]
    fn stock_hero_figures_never_resolve_to_a_photograph() {
        assert_eq!(
            figure_photo("assets/ui/generated/sprite-hero-2-documentary-photo"),
            None
        );
        assert_eq!(figure_photo("sprite-hero-3"), None);
        for none in [
            "sprite-courier",
            "sprite-shop-soldier-documentary-photo",
            "live-canonical-photo",
            "",
        ] {
            assert_eq!(figure_photo(none), None, "{none}");
        }
    }

    #[test]
    fn classify_names_what_each_reference_is() {
        assert_eq!(classify(""), Reference::Empty);
        assert_eq!(
            classify("live-room-photo"),
            Reference::Token(Token::RoomPhoto)
        );
        assert_eq!(
            classify("assets/ui/generated/live-canonical-photo"),
            Reference::Token(Token::CanonicalPhoto)
        );
        assert_eq!(
            classify("/gameplay/imagery/abc"),
            Reference::Live("/gameplay/imagery/abc")
        );
        assert_eq!(
            classify("assets/ui/generated/sprite-courier-documentary-photo.webp"),
            Reference::Reviewed("sprite-courier-documentary-photo")
        );
        assert_eq!(classify("icon-sword"), Reference::Chrome("icon-sword"));
        assert_eq!(classify("none"), Reference::Chrome("none"));
        assert_eq!(
            classify("scene-place-inn-taproom"),
            Reference::Painted("scene-place-inn-taproom")
        );
        assert_eq!(classify("prop-crates"), Reference::Painted("prop-crates"));
        assert_eq!(
            classify("portrait-hero-1"),
            Reference::Identity {
                key: "portrait-hero-1",
                photo: None
            }
        );
    }

    #[test]
    fn content_may_name_reviewed_photographs_and_tokens_only() {
        for ok in [
            "",
            "live-room-photo",
            "live-canonical-photo",
            "neutral",
            "none",
            "assets/ui/generated/scene-salt-quay-arrival-documentary-photo",
            "sprite-courier-documentary-photo",
            "/gameplay/imagery/key",
        ] {
            assert_eq!(check_content_reference(ok), Ok(()), "{ok}");
        }
        for painted in [
            "assets/ui/generated/scene-salt-quay-arrival",
            "ground-salt-quay",
            "prop-barrels",
            "portrait-villain-lich",
            "sprite-patron-bell-ringer",
            "figure-hero-1-photo",
        ] {
            assert_eq!(
                check_content_reference(painted),
                Err(ContentArtError::Painted),
                "{painted}"
            );
        }
        assert_eq!(
            check_content_reference("portrait-hero-1"),
            Err(ContentArtError::IdentityKey)
        );
    }

    #[test]
    fn clients_fetch_only_live_chrome_and_reviewed_art() {
        assert_eq!(
            fetch_target("/gameplay/imagery/key"),
            Some("/gameplay/imagery/key")
        );
        assert_eq!(fetch_target("icon-attack"), Some("icon-attack"));
        assert_eq!(fetch_target("sprite-hero-1-documentary-photo"), None);
        assert_eq!(fetch_target("portrait-hero-1"), None);
        for refused in [
            "",
            "live-room-photo",
            "neutral",
            "portrait-ant-wolf",
            "ground-salt-quay",
            "scene-courier-road",
        ] {
            assert_eq!(fetch_target(refused), None, "{refused}");
        }
    }

    #[test]
    fn every_plan_walks_cached_live_then_reviewed_then_request_then_neutral() {
        let hero = plan(Role::HeroPortrait, "portrait-hero-1");
        assert_eq!(
            hero.walk().collect::<Vec<_>>(),
            vec![
                Step::LiveCached(LiveSource::CanonicalCharacter),
                Step::RequestLive(LiveSource::CanonicalCharacter),
                Step::Neutral(Neutral::Silhouette),
            ]
        );
        let creature = plan(Role::CharacterToken, "portrait-ant-wolf");
        assert_eq!(creature.reviewed, None);
        assert_eq!(
            creature.walk().collect::<Vec<_>>(),
            vec![
                Step::LiveCached(LiveSource::CanonicalCharacter),
                Step::RequestLive(LiveSource::CanonicalCharacter),
                Step::Neutral(Neutral::Silhouette),
            ]
        );
    }

    #[test]
    fn places_use_the_room_photograph_and_never_resolve_a_painted_stem() {
        for request in [
            "live-room-photo",
            "scene-place-guardhouse",
            "ground-salt-quay",
            "",
        ] {
            let battle_plan = plan(Role::BattleBackdrop, request);
            assert_eq!(battle_plan.reviewed, None, "{request}");
            assert_eq!(
                battle_plan.walk().collect::<Vec<_>>(),
                vec![
                    Step::LiveCached(LiveSource::RoomPhoto),
                    Step::RequestLive(LiveSource::RoomPhoto),
                    Step::Neutral(Neutral::Nothing),
                ],
                "{request}"
            );
        }
        let named = plan(
            Role::BattleBackdrop,
            "assets/ui/generated/scene-salt-quay-arrival-documentary-photo",
        );
        assert_eq!(
            named.reviewed,
            Some("scene-salt-quay-arrival-documentary-photo")
        );
        let ground = plan(Role::BattleGround, "ground-salt-quay");
        assert_eq!(ground.neutral, Neutral::PlainFloor);
        assert_eq!(ground.reviewed, None);
        let prop = plan(Role::BattleProp, "prop-crates");
        assert_eq!(prop.neutral, Neutral::Block);
        assert_eq!(prop.reviewed, None);
    }

    #[test]
    fn lobby_art_is_the_live_room_photograph_or_the_reviewed_photograph_the_scene_names() {
        // A scene that names the reviewed harbor photograph shows it; the live room photograph
        // is tried first, and a painted stem names nothing.
        let harbor = plan(Role::LobbyArt, HARBOR_PHOTO);
        assert_eq!(
            harbor.walk().collect::<Vec<_>>(),
            vec![
                Step::LiveCached(LiveSource::RoomPhoto),
                Step::Reviewed(HARBOR_PHOTO),
                Step::RequestLive(LiveSource::RoomPhoto),
                Step::Neutral(Neutral::Nothing),
            ]
        );
        assert_eq!(
            library_url(
                Role::LobbyArt,
                "assets/ui/generated/scene-salt-quay-arrival-documentary-photo"
            )
            .as_deref(),
            Some("/assets/ui/generated/scene-salt-quay-arrival-documentary-photo")
        );
        assert_eq!(library_url(Role::LobbyArt, "scene-place-guardhouse"), None);
        assert_eq!(library_url(Role::LobbyArt, ROOM_PHOTO), None);
    }

    #[test]
    fn generation_references_are_reviewed_or_the_neutral_carrier() {
        let reference = plan(Role::GenerationReference, "portrait-ant-wolf");
        assert_eq!(reference.reviewed, None);
        assert!(reference.live.is_empty());
        assert_eq!(
            reference.walk().collect::<Vec<_>>(),
            vec![Step::Neutral(Neutral::Carrier)]
        );
        assert_eq!(
            plan(Role::GenerationReference, "figure-hero-2").reviewed,
            None
        );
    }

    #[test]
    fn a_subject_has_one_stable_key() {
        assert_eq!(Subject::character("ant-wolf").key(), "character:ant-wolf");
        assert_eq!(Subject::place("salt-quay").key(), "place:salt-quay");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn every_reviewed_photograph_exists_and_is_listed_in_the_manifest() {
        let directory =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/ui/generated");
        let manifest = std::fs::read_to_string(directory.join("documentary-photo-v2.assets.json"))
            .expect("reviewed photograph manifest");
        for stem in REVIEWED_PHOTOS {
            assert!(
                directory.join(format!("{stem}.webp")).is_file()
                    || directory.join(format!("{stem}.png")).is_file(),
                "{stem} is missing on disk"
            );
            assert!(
                manifest.contains(&format!("\"{stem}.")),
                "{stem} is not listed in documentary-photo-v2.assets.json"
            );
            assert!(stem.ends_with(REVIEWED_SUFFIX));
        }
    }
}
