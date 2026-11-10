//! Tests of the standard memory: the pure rules, every route, every typed error, the stored shape,
//! the gallery sheet and many owners with many records.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::memory::cli::{MemoryCommand, parse as parse_memory};
use super::memory::request::{
    Composed, Deleted, History, MemoryError, MemoryPage, MemoryReply, OwnerList,
};
use super::memory::routes::path;
use super::memory::{
    LinkKind, Memory, MemoryBook, MemoryDraft, MemoryField, MemoryFilter, MemoryLink, Put,
    References, SCHEMA, check_draft, named, page, put_into, references_for, select,
};
use crate::command::{CommandError, split_words};
use crate::dev_client::{get_json, post_json};
use crate::dev_http::{DevReply, RouteError};
use crate::test_support::{Running, STARTER_FILES, start};

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = u32::try_from(data.len()).unwrap().to_be_bytes().to_vec();
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
    out
}

/// A real PNG of one colour, stored without compression, so a test needs no encoder.
fn solid_png(width: u32, height: u32, color: [u8; 3]) -> Vec<u8> {
    let mut raw = Vec::new();
    for _ in 0..height {
        raw.extend_from_slice(&[0]);
        for _ in 0..width {
            raw.extend_from_slice(&color);
        }
    }
    let mut zlib = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
    for (index, block) in blocks.iter().enumerate() {
        zlib.push(u8::from(index + 1 == blocks.len()));
        let length = u16::try_from(block.len()).unwrap();
        zlib.extend_from_slice(&length.to_le_bytes());
        zlib.extend_from_slice(&(!length).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    let (mut a, mut b) = (1_u32, 0_u32);
    for byte in &raw {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    zlib.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut header = width.to_be_bytes().to_vec();
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend(chunk(b"IHDR", &header));
    png.extend(chunk(b"IDAT", &zlib));
    png.extend(chunk(b"IEND", &[]));
    png
}

fn picture_in(host: &Running, name: &str, color: [u8; 3]) {
    let directory = host.playground_path("assets/media");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join(name), solid_png(64, 36, color)).unwrap();
}

fn draft(owner: &str, id: &str, kind: &str, pictures: &[&str]) -> MemoryDraft {
    MemoryDraft {
        id: id.to_owned(),
        owner: owner.to_owned(),
        kind: kind.to_owned(),
        title: format!("title of {id}"),
        description: format!("how {id} looks"),
        place: None,
        pictures: pictures.iter().map(|name| (*name).to_owned()).collect(),
        links: Vec::new(),
        tags: Vec::new(),
        created_by: None,
    }
}

fn book_with(owner: &str, count: u64) -> MemoryBook {
    let mut book = MemoryBook::new(owner).unwrap();
    for number in 1..=count {
        let kind = if number % 2 == 0 { "scene" } else { "artwork" };
        let mut one = draft(
            owner,
            &format!("m{number}"),
            kind,
            &[&format!("p{number}.png")],
        );
        if number % 3 == 0 {
            one.tags = vec!["third".to_owned()];
        }
        put_into(&mut book, one).unwrap();
    }
    book
}

#[test]
fn closed_keys_are_their_serde_names() {
    for kind in LinkKind::ALL {
        assert_eq!(json!(kind), json!(kind.key()));
        assert_eq!(LinkKind::from_key(kind.key()), Some(kind));
    }
    for field in MemoryField::ALL {
        assert_eq!(json!(field), json!(field.key()));
        assert_eq!(MemoryField::from_key(field.key()), Some(field));
    }
    assert_eq!(LinkKind::from_key("note"), None);
}

#[test]
fn a_draft_is_checked_before_it_is_kept() {
    let good = draft("danny", "scene-1", "scene", &["a.png"]);
    assert_eq!(check_draft(&good), Ok(()));
    let with = |change: &dyn Fn(&mut MemoryDraft)| {
        let mut bad = good.clone();
        change(&mut bad);
        check_draft(&bad).unwrap_err()
    };
    let key = |error: MemoryError| error.key();
    assert_eq!(key(with(&|d| d.owner = "Danny".into())), "invalid_owner");
    assert_eq!(key(with(&|d| d.owner = String::new())), "invalid_owner");
    assert_eq!(key(with(&|d| d.id = "Scene 1".into())), "invalid_id");
    assert_eq!(key(with(&|d| d.kind = "a kind".into())), "invalid_kind");
    assert_eq!(
        key(with(&|d| d.place = Some("Town".into()))),
        "invalid_place"
    );
    assert_eq!(key(with(&|d| d.title = "  ".into())), "empty_title");
    assert_eq!(
        key(with(&|d| d.description = String::new())),
        "empty_description"
    );
    assert_eq!(key(with(&|d| d.title = "t".repeat(1_001))), "too_long");
    assert_eq!(
        key(with(&|d| d.pictures = vec!["a.png".into(); 2])),
        "duplicate_picture"
    );
    assert_eq!(
        key(with(&|d| d.pictures = vec!["../a.png".into()])),
        "invalid_picture"
    );
    assert_eq!(
        key(with(&|d| d.pictures = vec!["a.mp3".into()])),
        "invalid_picture"
    );
    assert_eq!(
        key(with(
            &|d| d.pictures = (0..65).map(|n| format!("{n}.png")).collect()
        )),
        "too_many"
    );
    assert_eq!(key(with(&|d| d.tags = vec!["Big".into()])), "invalid_tag");
    assert_eq!(
        key(with(&|d| d.tags = vec!["a".into(), "a".into()])),
        "duplicate_tag"
    );
    let link = |id: &str| MemoryLink {
        what: LinkKind::Item,
        id: id.to_owned(),
    };
    assert_eq!(
        key(with(&|d| d.links = vec![link("Easel")])),
        "invalid_link"
    );
    assert_eq!(
        key(with(&|d| d.links = vec![link("easel"), link("easel")])),
        "duplicate_link"
    );
    assert_eq!(
        MemoryField::Title,
        match with(&|d| d.title = "t".repeat(1_001)) {
            MemoryError::TooLong { field, maximum } => {
                assert_eq!(maximum, 1_000);
                field
            }
            other => panic!("{other:?}"),
        }
    );
}

#[test]
fn a_put_creates_with_the_next_seq_and_replaces_keeping_it() {
    let mut book = MemoryBook::new("danny").unwrap();
    let first = put_into(&mut book, draft("danny", "a", "scene", &["a.png"])).unwrap();
    let second = put_into(&mut book, draft("danny", "b", "artwork", &["b.png"])).unwrap();
    assert_eq!((first.memory.seq, first.created), (1, true));
    assert_eq!((second.memory.seq, second.created), (2, true));
    let mut again = draft("danny", "a", "scene", &["a2.png"]);
    again.title = "changed".to_owned();
    let replaced = put_into(&mut book, again).unwrap();
    assert_eq!((replaced.memory.seq, replaced.created), (1, false));
    assert_eq!(book.memories.len(), 2);
    assert_eq!(book.memory("a").unwrap().title, "changed");
    assert_eq!(book.next_seq, 3);
    assert_eq!(book.check(), Ok(()));
}

#[test]
fn a_refused_put_changes_nothing() {
    let mut book = book_with("danny", 3);
    let before = book.clone();
    let mut link = draft("danny", "x", "note", &[]);
    link.links = vec![MemoryLink {
        what: LinkKind::Memory,
        id: "ghost".to_owned(),
    }];
    assert_eq!(
        put_into(&mut book, link),
        Err(MemoryError::UnknownMemory {
            owner: "danny".to_owned(),
            id: "ghost".to_owned()
        })
    );
    assert_eq!(
        put_into(&mut book, draft("mira", "x", "note", &[])),
        Err(MemoryError::OwnerMismatch {
            owner: "mira".to_owned(),
            book: "danny".to_owned()
        })
    );
    assert_eq!(book, before);
    let mut linked = draft("danny", "x", "note", &[]);
    linked.links = vec![MemoryLink {
        what: LinkKind::Memory,
        id: "m1".to_owned(),
    }];
    assert!(put_into(&mut book, linked).is_ok());
}

#[test]
fn paging_walks_every_memory_once_oldest_first() {
    let book = book_with("danny", 25);
    let all = MemoryFilter::default();
    let mut seen: Vec<u64> = Vec::new();
    let mut after: Option<String> = None;
    let mut pages = 0;
    loop {
        let (found, next) = page(&book, &all, 10, after.as_deref()).unwrap();
        pages += 1;
        seen.extend(found.iter().map(|memory| memory.seq));
        match next {
            Some(cursor) => after = Some(cursor),
            None => break,
        }
    }
    assert_eq!(pages, 3);
    assert_eq!(seen, (1..=25).collect::<Vec<u64>>());
    // A page that ends exactly at the end has no next.
    let (found, next) = page(&book, &all, 25, None).unwrap();
    assert_eq!((found.len(), next), (25, None));
    // A filter pages on its own.
    let scenes = MemoryFilter {
        kind: Some("scene".to_owned()),
        ..MemoryFilter::default()
    };
    let (found, next) = page(&book, &scenes, 5, None).unwrap();
    assert_eq!(found.len(), 5);
    let (rest, last) = page(&book, &scenes, 50, next.as_deref()).unwrap();
    assert_eq!((rest.len(), last), (7, None));
    assert!(
        found
            .iter()
            .chain(&rest)
            .all(|memory| memory.kind == "scene")
    );
    for bad in ["", "c", "x1", "c-1", "c1 ", "cg"] {
        assert_eq!(
            page(&book, &all, 10, Some(bad)).unwrap_err(),
            MemoryError::InvalidCursor,
            "{bad:?}"
        );
    }
}

#[test]
fn a_filter_means_kind_place_and_tag_together() {
    let mut book = book_with("danny", 6);
    let mut placed = draft("danny", "p", "scene", &["p.png"]);
    placed.place = Some("market".to_owned());
    placed.tags = vec!["third".to_owned(), "wide".to_owned()];
    put_into(&mut book, placed).unwrap();
    let filter = |kind: Option<&str>, place: Option<&str>, tag: Option<&str>| MemoryFilter {
        kind: kind.map(str::to_owned),
        place: place.map(str::to_owned),
        tag: tag.map(str::to_owned),
    };
    let ids = |filter: &MemoryFilter| -> Vec<String> {
        select(&book, filter)
            .into_iter()
            .map(|memory| memory.id.clone())
            .collect()
    };
    assert_eq!(ids(&filter(None, None, None)).len(), 7);
    assert_eq!(ids(&filter(Some("scene"), None, None)).len(), 4);
    assert_eq!(ids(&filter(None, Some("market"), None)), ["p"]);
    assert_eq!(ids(&filter(None, None, Some("third"))), ["m3", "m6", "p"]);
    assert_eq!(
        ids(&filter(Some("scene"), None, Some("third"))),
        ["m6", "p"]
    );
    assert_eq!(ids(&filter(Some("artwork"), Some("market"), None)).len(), 0);
    assert_eq!(
        filter(Some("Bad kind"), None, None)
            .check()
            .unwrap_err()
            .key(),
        "invalid_kind"
    );
}

#[test]
fn references_fit_a_budget_newest_first_or_in_the_order_given() {
    let book = book_with("danny", 6);
    let mut chosen = select(&book, &MemoryFilter::default());
    chosen.reverse();
    let fitted = references_for(&chosen, 4);
    assert_eq!(fitted.pictures, ["p6.png", "p5.png", "p4.png", "p3.png"]);
    assert_eq!(fitted.memories, ["m6", "m5", "m4", "m3"]);
    assert_eq!(fitted.left_out, ["p2.png", "p1.png"]);
    let none = references_for(&chosen, 0);
    assert_eq!((none.pictures.len(), none.left_out.len()), (0, 6));
    let given = named(&book, &["m2".to_owned(), "m1".to_owned()]).unwrap();
    assert_eq!(references_for(&given, 8).pictures, ["p2.png", "p1.png"]);
    // One picture twice is attached once; a memory with none stands for none.
    let mut twin = book_with("danny", 1);
    put_into(&mut twin, draft("danny", "same", "scene", &["p1.png"])).unwrap();
    put_into(&mut twin, draft("danny", "note", "note", &[])).unwrap();
    let all = select(&twin, &MemoryFilter::default());
    assert_eq!(
        references_for(&all, 8),
        References {
            pictures: vec!["p1.png".to_owned()],
            memories: vec!["m1".to_owned()],
            left_out: Vec::new()
        }
    );
    assert_eq!(
        named(&book, &["m1".to_owned(), "m1".to_owned()])
            .unwrap_err()
            .key(),
        "duplicate_selected"
    );
    assert_eq!(
        named(&book, &["zz".to_owned()]).unwrap_err().key(),
        "unknown_memory"
    );
}

async fn host() -> Running {
    start(&STARTER_FILES).await
}

async fn post<B: Serialize, T: DeserializeOwned>(
    host: &Running,
    address: &str,
    body: B,
) -> MemoryReply<T> {
    post_json(host.port, address, &body).await.unwrap()
}

async fn read<T: DeserializeOwned>(host: &Running, address: &str) -> MemoryReply<T> {
    get_json(host.port, address).await.unwrap()
}

fn done<T: std::fmt::Debug>(reply: MemoryReply<T>) -> T {
    match reply {
        DevReply::Done { value } => value,
        DevReply::Rejected { error, message } => panic!("refused: {error:?} {message}"),
    }
}

fn refusal<T: std::fmt::Debug>(reply: MemoryReply<T>) -> MemoryError {
    match reply {
        DevReply::Done { value } => panic!("done: {value:?}"),
        DevReply::Rejected { error, .. } => error,
    }
}

async fn put(host: &Running, draft: &MemoryDraft) -> Put {
    done(post(host, path::MEMORY_PUT, draft).await)
}

async fn capture(host: &Running, owner: &str, kind: &str, picture: &str, title: &str) -> Put {
    done(
        post(
            host,
            path::MEMORY_CAPTURE,
            json!({"owner": owner, "kind": kind, "picture": picture, "title": title}),
        )
        .await,
    )
}

async fn listing(host: &Running, owner: &str, query: &str) -> MemoryPage {
    done(read(host, &format!("{}?owner={owner}{query}", path::MEMORY_LIST)).await)
}

fn lines(host: &Running) -> Vec<String> {
    host.state
        .log()
        .after_sequence(0, usize::MAX)
        .into_iter()
        .map(|line| line.text)
        .filter(|text| text.starts_with("WORLD"))
        .collect()
}

/// Every file under `directory` with its text, by its path below it.
fn snapshot(directory: &Path) -> BTreeMap<String, String> {
    fn walk(root: &Path, at: &Path, out: &mut BTreeMap<String, String>) {
        let Ok(entries) = std::fs::read_dir(at) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let name = path.strip_prefix(root).unwrap().display().to_string();
                out.insert(name, std::fs::read_to_string(&path).unwrap_or_default());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(directory, directory, &mut out);
    out
}

#[tokio::test]
async fn two_owners_put_get_list_and_delete_across_the_routes() {
    let host = host().await;
    picture_in(&host, "market.png", [200, 160, 90]);
    picture_in(&host, "study.png", [90, 120, 200]);
    assert!(
        done(read::<OwnerList>(&host, path::MEMORY).await)
            .owners
            .is_empty()
    );

    let mut scene = draft("danny", "market-day", "scene", &["market.png"]);
    scene.place = Some("market".to_owned());
    scene.tags = vec!["morning".to_owned()];
    scene.created_by = Some("runner".to_owned());
    let created = put(&host, &scene).await;
    assert_eq!((created.created, created.memory.seq), (true, 1));

    let mut study = draft("danny", "study-1", "artwork", &["study.png", "market.png"]);
    study.links = vec![
        MemoryLink {
            what: LinkKind::Place,
            id: "market".to_owned(),
        },
        MemoryLink {
            what: LinkKind::Memory,
            id: "market-day".to_owned(),
        },
        MemoryLink {
            what: LinkKind::Item,
            id: "painting_01".to_owned(),
        },
    ];
    assert_eq!(put(&host, &study).await.memory.seq, 2);
    // The second owner has its own ids and its own sequence.
    let other = put(
        &host,
        &draft("mira", "market-day", "scene", &["market.png"]),
    )
    .await;
    assert_eq!(other.memory.seq, 1);

    let owners = done(read::<OwnerList>(&host, path::MEMORY).await).owners;
    assert_eq!(
        owners
            .iter()
            .map(|owner| (owner.owner.as_str(), owner.memories, owner.next_seq))
            .collect::<Vec<_>>(),
        [("danny", 2, 3), ("mira", 1, 2)]
    );
    assert_eq!(
        owners[0]
            .kinds
            .iter()
            .map(|kind| (kind.kind.as_str(), kind.count))
            .collect::<Vec<_>>(),
        [("artwork", 1), ("scene", 1)]
    );

    let got: Memory = done(
        read(
            &host,
            &format!("{}?owner=danny&id=study-1", path::MEMORY_GET),
        )
        .await,
    );
    assert_eq!(got, study_as_kept(&study, 2));
    assert_eq!(got.primary(), Some("study.png"));

    let artworks = listing(&host, "danny", "&kind=artwork").await;
    assert_eq!(artworks.memories.len(), 1);
    assert_eq!(artworks.ledger.items.len(), 1);
    assert_eq!(artworks.ledger.items[0].name, "title of study-1");
    assert_eq!(
        artworks.ledger.items[0].picture.as_deref(),
        Some("study.png")
    );
    assert_eq!(
        listing(&host, "danny", "&place=market")
            .await
            .memories
            .len(),
        1
    );
    assert_eq!(
        listing(&host, "danny", "&tag=morning").await.memories.len(),
        1
    );
    assert_eq!(listing(&host, "danny", "").await.memories.len(), 2);

    // A replace keeps its place in the order.
    let mut changed = scene.clone();
    changed.title = "market day, later".to_owned();
    let replaced = put(&host, &changed).await;
    assert_eq!((replaced.created, replaced.memory.seq), (false, 1));
    assert_eq!(
        listing(&host, "danny", "").await.memories[0].title,
        "market day, later"
    );

    // A delete removes the memory and not its pictures, and a seq is not given again.
    let deleted: Deleted = done(
        post(
            &host,
            path::MEMORY_DELETE,
            json!({"owner": "danny", "id": "study-1"}),
        )
        .await,
    );
    assert_eq!(deleted.memory.id, "study-1");
    assert!(host.playground_path("assets/media/study.png").is_file());
    assert_eq!(listing(&host, "danny", "").await.memories.len(), 1);
    let later = put(&host, &draft("danny", "study-2", "artwork", &["study.png"])).await;
    assert_eq!(later.memory.seq, 3);
    assert!(
        refusal::<Memory>(
            read(
                &host,
                &format!("{}?owner=danny&id=study-1", path::MEMORY_GET)
            )
            .await
        )
        .key()
            == "unknown_memory"
    );

    let log = lines(&host);
    assert!(
        log.contains(
            &concat!(
                "WORLD memory_put owner=danny id=market-day kind=scene place=market seq=1 ",
                "pictures=1 created=true"
            )
            .to_owned()
        )
    );
    assert!(
        log.contains(
            &concat!(
                "WORLD memory_put owner=danny id=market-day kind=scene place=market seq=1 ",
                "pictures=1 created=false"
            )
            .to_owned()
        )
    );
    assert!(log.contains(&"WORLD memory_deleted owner=danny id=study-1 kind=artwork".to_owned()));
    assert!(
        log.iter().all(|line| !line.contains("market day")),
        "words stay out"
    );
    let history: History = done(read(&host, &format!("{}?owner=mira", path::MEMORY_HISTORY)).await);
    assert_eq!(history.entries.len(), 1);
    let history: History = done(read(&host, &format!("{}?limit=2", path::MEMORY_HISTORY)).await);
    assert_eq!(history.entries.len(), 2);
    assert!(history.entries[0].sequence < history.entries[1].sequence);
}

fn study_as_kept(draft: &MemoryDraft, seq: u64) -> Memory {
    Memory {
        id: draft.id.clone(),
        owner: draft.owner.clone(),
        kind: draft.kind.clone(),
        title: draft.title.clone(),
        description: draft.description.clone(),
        place: draft.place.clone(),
        seq,
        pictures: draft.pictures.clone(),
        links: draft.links.clone(),
        tags: draft.tags.clone(),
        created_by: draft.created_by.clone(),
    }
}

#[tokio::test]
async fn the_stored_shape_is_a_book_for_each_owner_an_index_and_a_sidecar_for_each_gallery() {
    let host = host().await;
    picture_in(&host, "a.png", [10, 10, 10]);
    capture(&host, "danny", "scene", "a.png", "first").await;
    capture(&host, "mira", "scene", "a.png", "other").await;
    let composed: Composed =
        done(post(&host, path::MEMORY_COMPOSE, json!({"owner": "danny"})).await);
    let root = host.playground_path("assets/world/memory");
    let files: Vec<String> = snapshot(&root)
        .into_keys()
        .map(|name| name.replace('\\', "/"))
        .collect();
    assert_eq!(
        files,
        [
            "galleries/gallery-danny-1.json".to_owned(),
            "index.json".to_owned(),
            "owners/danny.json".to_owned(),
            "owners/mira.json".to_owned(),
        ]
    );
    let book: Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("owners/danny.json")).unwrap())
            .unwrap();
    assert_eq!(book["schema"], SCHEMA);
    assert_eq!(book["owner"], "danny");
    assert_eq!(book["next_seq"], 2);
    assert_eq!(book["galleries"], 1);
    assert_eq!(book["memories"][0]["id"], "scene-1");
    let sidecar: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("galleries/gallery-danny-1.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(sidecar["members"], json!(["scene-1"]));
    assert_eq!(sidecar["picture"], composed.picture);
    // The pictures of a book are names, never bytes; the library is untouched by the memories.
    assert!(
        host.playground_path("assets/media/gallery-danny-1.png")
            .is_file()
    );
}

#[tokio::test]
async fn every_route_refuses_with_a_typed_error_and_changes_nothing() {
    let host = host().await;
    picture_in(&host, "a.png", [10, 10, 10]);
    capture(&host, "danny", "scene", "a.png", "first").await;
    let root = host.playground_path("assets/world/memory");
    let before = snapshot(&root);
    let media_before = snapshot(&host.playground_path("assets/media"));

    let good = json!({
        "id": "x", "owner": "danny", "kind": "note", "title": "t", "description": "d",
        "pictures": ["a.png"]
    });
    let with = |key: &str, value: Value| {
        let mut body = good.clone();
        body[key] = value;
        body
    };
    let puts: Vec<(Value, &str)> = vec![
        (with("owner", json!("Danny")), "invalid_owner"),
        (with("id", json!("X Y")), "invalid_id"),
        (with("kind", json!("a kind")), "invalid_kind"),
        (with("title", json!("")), "empty_title"),
        (with("description", json!("   ")), "empty_description"),
        (with("pictures", json!(["missing.png"])), "unknown_picture"),
        (
            with("pictures", json!(["a.png", "a.png"])),
            "duplicate_picture",
        ),
        (with("pictures", json!(["../a.png"])), "invalid_picture"),
        (with("tags", json!(["A"])), "invalid_tag"),
        (
            with("links", json!([{"what": "memory", "id": "ghost"}])),
            "unknown_memory",
        ),
        (
            with("links", json!([{"what": "item", "id": "Bad"}])),
            "invalid_link",
        ),
        (with("title", json!("t".repeat(1_001))), "too_long"),
        (with("extra", json!(1)), "invalid_body"),
        (json!({"id": "x"}), "invalid_body"),
        (
            with("links", json!([{"what": "person", "id": "a"}])),
            "invalid_body",
        ),
    ];
    for (body, key) in puts {
        let error = refusal::<Value>(post(&host, path::MEMORY_PUT, &body).await);
        assert_eq!(error.key(), key, "put {body}");
    }

    let captures: Vec<(Value, &str)> = vec![
        (
            json!({"owner": "danny", "kind": "scene", "picture": "a.png", "title": "t",
                   "id": "scene-1"}),
            "duplicate_id",
        ),
        (
            json!({"owner": "danny", "kind": "scene", "picture": "gone.png", "title": "t"}),
            "unknown_picture",
        ),
        (
            json!({"owner": "danny", "kind": "scene", "picture": "a.txt", "title": "t"}),
            "invalid_picture",
        ),
        (
            json!({"owner": "danny", "kind": "Scene", "picture": "a.png", "title": "t"}),
            "invalid_kind",
        ),
        (
            json!({"owner": "danny", "kind": "scene", "picture": "a.png", "title": " "}),
            "empty_title",
        ),
        (
            json!({"owner": "danny", "kind": "scene", "picture": "a.png", "title": "t",
                   "place": "Town"}),
            "invalid_place",
        ),
    ];
    for (body, key) in captures {
        let error = refusal::<Value>(post(&host, path::MEMORY_CAPTURE, &body).await);
        assert_eq!(error.key(), key, "capture {body}");
    }

    let deletes = [
        (json!({"owner": "danny", "id": "ghost"}), "unknown_memory"),
        (json!({"owner": "nobody", "id": "scene-1"}), "unknown_owner"),
        (json!({"owner": "Danny", "id": "scene-1"}), "invalid_owner"),
        (json!({"owner": "danny", "id": "Scene 1"}), "invalid_id"),
    ];
    for (body, key) in deletes {
        let error = refusal::<Value>(post(&host, path::MEMORY_DELETE, &body).await);
        assert_eq!(error.key(), key, "delete {body}");
    }

    let selections = [
        (
            json!({"owner": "danny", "budget": 4, "ids": ["scene-1"], "filter": {"kind": "scene"}}),
            "conflicting_selection",
        ),
        (
            json!({"owner": "danny", "budget": 4, "ids": ["ghost"]}),
            "unknown_memory",
        ),
        (
            json!({"owner": "danny", "budget": 4, "ids": ["scene-1", "scene-1"]}),
            "duplicate_selected",
        ),
        (
            json!({"owner": "danny", "budget": 4, "filter": {"kind": "Bad"}}),
            "invalid_kind",
        ),
        (json!({"owner": "nobody", "budget": 4}), "unknown_owner"),
        (json!({"owner": "danny"}), "invalid_body"),
    ];
    for (body, key) in selections {
        let error = refusal::<Value>(post(&host, path::MEMORY_REFERENCES, &body).await);
        assert_eq!(error.key(), key, "references {body}");
    }

    let composes = [
        (
            json!({"owner": "danny", "name": "Bad Name"}),
            "invalid_name",
        ),
        (json!({"owner": "nobody"}), "unknown_owner"),
        (
            json!({"owner": "danny", "filter": {"kind": "artwork"}}),
            "nothing_to_compose",
        ),
        (
            json!({"owner": "danny", "ids": ["scene-1"], "filter": {"kind": "scene"}}),
            "conflicting_selection",
        ),
        (json!({"owner": "danny", "columns": 0}), "gallery"),
        (json!({"owner": "danny", "tile_width_px": 0}), "gallery"),
        (json!({"owner": "danny", "tile_width_px": 5000}), "gallery"),
        (
            json!({"owner": "danny", "background": [1, 2]}),
            "invalid_body",
        ),
    ];
    for (body, key) in composes {
        let error = refusal::<Value>(post(&host, path::MEMORY_COMPOSE, &body).await);
        assert_eq!(error.key(), key, "compose {body}");
    }

    let queries = [
        (
            format!("{}?owner=danny&id=ghost", path::MEMORY_GET),
            "unknown_memory",
        ),
        (
            format!("{}?owner=nobody&id=a", path::MEMORY_GET),
            "unknown_owner",
        ),
        (format!("{}?owner=danny", path::MEMORY_GET), "invalid_query"),
        (
            format!("{}?owner=danny&limit=0", path::MEMORY_LIST),
            "invalid_query",
        ),
        (
            format!("{}?owner=danny&limit=20000", path::MEMORY_LIST),
            "invalid_query",
        ),
        (
            format!("{}?owner=danny&limit=many", path::MEMORY_LIST),
            "invalid_query",
        ),
        (
            format!("{}?owner=danny&after=nope", path::MEMORY_LIST),
            "invalid_cursor",
        ),
        (
            format!("{}?owner=danny&kind=Bad", path::MEMORY_LIST),
            "invalid_kind",
        ),
        (
            format!("{}?owner=danny&place=Bad", path::MEMORY_LIST),
            "invalid_place",
        ),
        (
            format!("{}?owner=danny&tag=Bad", path::MEMORY_LIST),
            "invalid_tag",
        ),
        (
            format!("{}?owner=Danny", path::MEMORY_LIST),
            "invalid_owner",
        ),
        (
            format!("{}?owner=nobody", path::MEMORY_LIST),
            "unknown_owner",
        ),
        (
            format!("{}?owner=Danny", path::MEMORY_HISTORY),
            "invalid_owner",
        ),
        (
            format!("{}?who=danny", path::MEMORY_HISTORY),
            "invalid_query",
        ),
    ];
    for (address, key) in queries {
        let error = refusal::<Value>(read(&host, &address).await);
        assert_eq!(error.key(), key, "{address}");
    }

    assert_eq!(snapshot(&root), before, "nothing in the memories changed");
    assert_eq!(
        snapshot(&host.playground_path("assets/media")),
        media_before,
        "no picture was made or lost"
    );
    let refused = lines(&host)
        .into_iter()
        .filter(|line| line.starts_with("WORLD refused verb=memory_"))
        .count();
    assert_eq!(
        refused,
        15 + 6 + 4 + 6 + 8 + 14,
        "every refusal is one line"
    );
}

#[tokio::test]
async fn a_capture_makes_a_memory_of_one_picture_in_one_call() {
    let host = host().await;
    picture_in(&host, "plate-1.png", [1, 2, 3]);
    picture_in(&host, "plate-2.png", [4, 5, 6]);
    let first = capture(&host, "danny", "scene", "plate-1.png", "the old bridge").await;
    assert_eq!(first.memory.id, "scene-1");
    assert_eq!(first.memory.description, "the old bridge");
    assert_eq!(first.memory.pictures, ["plate-1.png"]);
    let second: Put = done(
        post(
            &host,
            path::MEMORY_CAPTURE,
            json!({
                "owner": "danny", "kind": "scene", "picture": "plate-2.png",
                "title": "the mill", "place": "mill", "tags": ["wide", "dusk"],
                "description": "a mill on a river at dusk", "created_by": "runner",
                "links": [{"what": "place", "id": "mill"}]
            }),
        )
        .await,
    );
    assert_eq!(second.memory.id, "scene-2");
    assert_eq!(second.memory.place.as_deref(), Some("mill"));
    assert_eq!(second.memory.tags, ["wide", "dusk"]);
    assert_eq!(second.memory.created_by.as_deref(), Some("runner"));
    // An id asked for is used, and the next derived id follows the sequence.
    let named: Put = done(
        post(
            &host,
            path::MEMORY_CAPTURE,
            json!({"owner": "danny", "kind": "artwork", "picture": "plate-2.png",
                   "title": "study", "id": "painting-01"}),
        )
        .await,
    );
    assert_eq!(named.memory.id, "painting-01");
    assert_eq!(
        capture(&host, "danny", "scene", "plate-1.png", "again")
            .await
            .memory
            .id,
        "scene-4"
    );
}

#[tokio::test]
async fn references_answer_the_pictures_for_a_plate_within_the_budget() {
    let host = host().await;
    for number in 1..=6 {
        let name = format!("s{number}.png");
        picture_in(&host, &name, [number * 10, 0, 0]);
        capture(
            &host,
            "danny",
            if number % 2 == 0 { "scene" } else { "artwork" },
            &name,
            "t",
        )
        .await;
    }
    let newest: References = done(
        post(
            &host,
            path::MEMORY_REFERENCES,
            json!({"owner": "danny", "budget": 4}),
        )
        .await,
    );
    assert_eq!(newest.pictures, ["s6.png", "s5.png", "s4.png", "s3.png"]);
    assert_eq!(newest.left_out, ["s2.png", "s1.png"]);
    let scenes: References = done(
        post(
            &host,
            path::MEMORY_REFERENCES,
            json!({"owner": "danny", "budget": 2, "filter": {"kind": "scene"}}),
        )
        .await,
    );
    assert_eq!(scenes.memories, ["scene-6", "scene-4"]);
    assert_eq!(scenes.left_out, ["s2.png"]);
    let chosen: References = done(
        post(
            &host,
            path::MEMORY_REFERENCES,
            json!({"owner": "danny", "budget": 8, "ids": ["scene-2", "artwork-1"]}),
        )
        .await,
    );
    assert_eq!(chosen.pictures, ["s2.png", "s1.png"]);
}

fn png_size(bytes: &[u8]) -> (u32, u32) {
    let read = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    (read(16), read(20))
}

#[tokio::test]
async fn compose_tiles_twenty_paintings_into_one_stored_picture() {
    let host = host().await;
    for number in 1..=20_u8 {
        let name = format!("painting-{number:02}.png");
        picture_in(&host, &name, [number * 10, 100, 255 - number * 10]);
        capture(
            &host,
            "danny",
            "artwork",
            &name,
            &format!("painting {number}"),
        )
        .await;
    }
    picture_in(&host, "plate.png", [9, 9, 9]);
    capture(&host, "danny", "scene", "plate.png", "town").await;

    let composed: Composed = done(
        post(
            &host,
            path::MEMORY_COMPOSE,
            json!({"owner": "danny", "filter": {"kind": "artwork"}}),
        )
        .await,
    );
    assert_eq!(composed.picture, "gallery-danny-1.png");
    assert_eq!(composed.members.len(), 20);
    assert_eq!(composed.members[0], "artwork-1");
    assert_eq!(composed.members[19], "artwork-20");
    assert_eq!(composed.pictures[0], "painting-01.png");
    assert_eq!((composed.columns, composed.rows), (4, 5));
    let sheet = std::fs::read(host.playground_path("assets/media/gallery-danny-1.png")).unwrap();
    assert_eq!(png_size(&sheet), (4 * 448 + 5 * 12, 5 * 252 + 6 * 12));
    assert!(lines(&host).contains(
        &"WORLD memory_composed owner=danny picture=gallery-danny-1.png members=20 columns=4 rows=5"
            .to_owned()
    ));

    // The next sheet is numbered on, and explicit ids and the layout are honoured.
    let second: Composed = done(
        post(
            &host,
            path::MEMORY_COMPOSE,
            json!({"owner": "danny", "ids": ["artwork-3", "artwork-1"], "columns": 2,
                   "tile_width_px": 64, "tile_height_px": 36, "gap_px": 2,
                   "background": [10, 20, 30]}),
        )
        .await,
    );
    assert_eq!(second.picture, "gallery-danny-2.png");
    assert_eq!(second.members, ["artwork-1", "artwork-3"], "in seq order");
    assert_eq!((second.columns, second.rows), (2, 1));
    let sheet = std::fs::read(host.playground_path("assets/media/gallery-danny-2.png")).unwrap();
    assert_eq!(png_size(&sheet), (2 * 64 + 3 * 2, 36 + 2 * 2));

    // A name asked for is used as it is, and a second compose under it replaces the sheet.
    let named: Composed = done(
        post(
            &host,
            path::MEMORY_COMPOSE,
            json!({"owner": "danny", "name": "the-day", "filter": {"kind": "scene"}}),
        )
        .await,
    );
    assert_eq!(named.picture, "the-day.png");
    assert!(
        host.playground_path("assets/world/memory/galleries/the-day.json")
            .is_file()
    );
    let everything: Composed = done(
        post(
            &host,
            path::MEMORY_COMPOSE,
            json!({"owner": "danny", "name": "the-day"}),
        )
        .await,
    );
    assert_eq!(everything.members.len(), 21);
    assert_eq!(
        done(read::<OwnerList>(&host, path::MEMORY).await).owners[0].memories,
        21,
        "a gallery is not a memory"
    );
}

#[tokio::test]
async fn a_memory_whose_picture_is_gone_names_it_when_composed() {
    let host = host().await;
    picture_in(&host, "a.png", [1, 1, 1]);
    capture(&host, "danny", "scene", "a.png", "t").await;
    std::fs::remove_file(host.playground_path("assets/media/a.png")).unwrap();
    let error =
        refusal::<Composed>(post(&host, path::MEMORY_COMPOSE, json!({"owner": "danny"})).await);
    assert_eq!(error.key(), "picture_unreadable");
    // The failed compose took no gallery number and left no sheet.
    picture_in(&host, "a.png", [1, 1, 1]);
    let composed: Composed =
        done(post(&host, path::MEMORY_COMPOSE, json!({"owner": "danny"})).await);
    assert_eq!(composed.picture, "gallery-danny-1.png");
}

#[tokio::test]
async fn concurrent_puts_to_one_owner_each_get_their_own_seq() {
    let host = host().await;
    picture_in(&host, "a.png", [1, 1, 1]);
    let port = host.port;
    let handles: Vec<_> = (0..24)
        .map(|number| {
            tokio::spawn(async move {
                let body = draft("danny", &format!("m{number}"), "scene", &["a.png"]);
                let reply: MemoryReply<Put> =
                    post_json(port, path::MEMORY_PUT, &body).await.unwrap();
                done(reply).memory.seq
            })
        })
        .collect();
    let mut seqs = Vec::new();
    for handle in handles {
        seqs.push(handle.await.unwrap());
    }
    seqs.sort_unstable();
    assert_eq!(seqs, (1..=24).collect::<Vec<u64>>());
}

/// Writes the files of `count` memories for each of `owners` as the store would, without the
/// writes' cost.
fn seed_books(host: &Running, owners: usize, count: u64) {
    let root = host.playground_path("assets/world/memory");
    std::fs::create_dir_all(root.join("owners")).unwrap();
    let mut index = Vec::new();
    for number in 0..owners {
        let owner = format!("npc-{number:02}");
        let memories: Vec<Value> = (1..=count)
            .map(|seq| {
                json!({
                    "id": format!("m{seq}"), "owner": owner,
                    "kind": if seq % 2 == 0 { "scene" } else { "artwork" },
                    "title": format!("{owner} {seq}"), "description": "how it looks",
                    "seq": seq, "pictures": ["a.png"]
                })
            })
            .collect();
        let book = json!({
            "schema": 1, "owner": owner, "next_seq": count + 1, "galleries": 0,
            "memories": memories
        });
        std::fs::write(
            root.join(format!("owners/{owner}.json")),
            serde_json::to_string_pretty(&book).unwrap(),
        )
        .unwrap();
        index.push(json!({
            "owner": owner, "memories": count, "next_seq": count + 1,
            "kinds": [
                {"kind": "artwork", "count": count.div_ceil(2)},
                {"kind": "scene", "count": count / 2}
            ]
        }));
    }
    std::fs::write(
        root.join("index.json"),
        serde_json::to_string_pretty(&json!({"schema": 1, "owners": index})).unwrap(),
    )
    .unwrap();
}

#[tokio::test]
async fn many_owners_with_many_records_behave_as_one_does() {
    let host = host().await;
    picture_in(&host, "a.png", [1, 1, 1]);
    const OWNERS: usize = 60;
    const RECORDS: u64 = 200;
    seed_books(&host, OWNERS, RECORDS);

    let owners = done(read::<OwnerList>(&host, path::MEMORY).await).owners;
    assert_eq!(owners.len(), OWNERS);
    assert!(owners.windows(2).all(|pair| pair[0].owner < pair[1].owner));
    assert_eq!(
        owners.iter().map(|owner| owner.memories).sum::<usize>(),
        12_000
    );

    // A list pages through one owner's records in order, once each.
    let mut seen = Vec::new();
    let mut after = String::new();
    loop {
        let query = if after.is_empty() {
            "&limit=50".to_owned()
        } else {
            format!("&limit=50&after={after}")
        };
        let page = listing(&host, "npc-17", &query).await;
        assert!(page.memories.len() <= 50);
        seen.extend(page.memories.iter().map(|memory| memory.seq));
        match page.next {
            Some(next) => after = next,
            None => break,
        }
    }
    assert_eq!(seen, (1..=RECORDS).collect::<Vec<u64>>());
    let scenes = listing(&host, "npc-17", "&kind=scene&limit=1000").await;
    assert_eq!(scenes.memories.len(), 100);
    assert!(scenes.next.is_none());
    let default_page = listing(&host, "npc-17", "").await;
    assert_eq!(default_page.memories.len(), 100);
    assert!(default_page.next.is_some());

    // A change to one owner writes that owner's book and the index, and no other owner's.
    let root = host.playground_path("assets/world/memory");
    let untouched = std::fs::read_to_string(root.join("owners/npc-07.json")).unwrap();
    let fresh = put(&host, &draft("npc-03", "fresh", "scene", &["a.png"])).await;
    assert_eq!(fresh.memory.seq, RECORDS + 1);
    let deleted: Deleted = done(
        post(
            &host,
            path::MEMORY_DELETE,
            json!({"owner": "npc-10", "id": "m5"}),
        )
        .await,
    );
    assert_eq!(deleted.memory.seq, 5);
    assert_eq!(
        std::fs::read_to_string(root.join("owners/npc-07.json")).unwrap(),
        untouched
    );
    let owners = done(read::<OwnerList>(&host, path::MEMORY).await).owners;
    let summary = |owner: &str| {
        owners
            .iter()
            .find(|row| row.owner == owner)
            .map(|row| (row.memories, row.next_seq))
    };
    assert_eq!(summary("npc-03"), Some((201, 202)));
    assert_eq!(summary("npc-10"), Some((199, 201)));
    assert_eq!(summary("npc-07"), Some((200, 201)));
    assert_eq!(owners.len(), OWNERS);

    // Selections and sheets work on a big book.
    let newest: References = done(
        post(
            &host,
            path::MEMORY_REFERENCES,
            json!({"owner": "npc-30", "budget": 8}),
        )
        .await,
    );
    assert_eq!(newest.memories.len(), 1, "one picture is attached once");
    let sheet: Composed = done(
        post(
            &host,
            path::MEMORY_COMPOSE,
            json!({"owner": "npc-30", "filter": {"kind": "scene"}, "tile_width_px": 8,
                   "tile_height_px": 8, "gap_px": 0}),
        )
        .await,
    );
    assert_eq!(sheet.members.len(), 100);
}

#[test]
fn the_verbs_read_their_words() {
    let parse = |line: &str| parse_memory(&split_words(line).unwrap());
    assert_eq!(parse("").unwrap(), MemoryCommand::Owners);
    assert_eq!(parse("owners").unwrap(), MemoryCommand::Owners);
    assert_eq!(
        parse("put a/b.json").unwrap(),
        MemoryCommand::Put {
            file: "a/b.json".into()
        }
    );
    assert_eq!(
        parse("get danny scene-1").unwrap(),
        MemoryCommand::Get {
            owner: "danny".into(),
            id: "scene-1".into()
        }
    );
    match parse("list danny kind=artwork place=market tag=wide limit=20 after=c1f").unwrap() {
        MemoryCommand::List {
            owner,
            filter,
            limit,
            after,
        } => {
            assert_eq!(owner, "danny");
            assert_eq!(filter.kind.as_deref(), Some("artwork"));
            assert_eq!(filter.place.as_deref(), Some("market"));
            assert_eq!(filter.tag.as_deref(), Some("wide"));
            assert_eq!((limit, after.as_deref()), (Some(20), Some("c1f")));
        }
        other => panic!("{other:?}"),
    }
    match parse(concat!(
        "capture danny scene plate.png \"the old mill\" place=mill tag=wide tag=dusk ",
        "link=place:mill by=runner"
    ))
    .unwrap()
    {
        MemoryCommand::Capture(request) => {
            assert_eq!(request.title, "the old mill");
            assert_eq!(request.tags, ["wide", "dusk"]);
            assert_eq!(request.place.as_deref(), Some("mill"));
            assert_eq!(request.links.len(), 1);
            assert_eq!(request.links[0].what, LinkKind::Place);
            assert_eq!(request.created_by.as_deref(), Some("runner"));
            assert_eq!(request.id, None);
        }
        other => panic!("{other:?}"),
    }
    match parse("references danny 5 kind=scene").unwrap() {
        MemoryCommand::References(request) => {
            assert_eq!(request.budget, 5);
            assert_eq!(request.filter.kind.as_deref(), Some("scene"));
            assert!(request.ids.is_empty());
        }
        other => panic!("{other:?}"),
    }
    match parse(
        "compose danny name=gallery kind=artwork columns=4 tile=448x252 gap=12 background=808080",
    )
    .unwrap()
    {
        MemoryCommand::Compose(request) => {
            assert_eq!(request.name.as_deref(), Some("gallery"));
            assert_eq!(request.columns, Some(4));
            assert_eq!(
                (request.tile_width_px, request.tile_height_px),
                (Some(448), Some(252))
            );
            assert_eq!(request.gap_px, Some(12));
            assert_eq!(request.background, Some([128, 128, 128]));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        parse("history danny limit=3").unwrap(),
        MemoryCommand::History {
            owner: Some("danny".into()),
            limit: Some(3)
        }
    );
    assert_eq!(
        parse("history limit=3").unwrap(),
        MemoryCommand::History {
            owner: None,
            limit: Some(3)
        }
    );
    for bad in [
        "get danny",
        "list danny colour=red",
        "list danny limit=many",
        "list danny limit=1 limit=2",
        "capture danny scene plate.png",
        "capture danny scene plate.png t link=person:bob",
        "references danny many",
        "compose danny tile=big",
        "compose danny background=80808",
        "compose danny background=gggggg",
        "delete danny",
        "frobnicate",
    ] {
        assert!(
            matches!(
                parse(bad),
                Err(CommandError::Usage(_)
                    | CommandError::BadChoice { .. }
                    | CommandError::DuplicateKey(_))
            ),
            "{bad}"
        );
    }
}

#[test]
fn every_error_has_its_own_key_a_sentence_and_a_status() {
    use std::collections::BTreeSet;

    use axum::http::StatusCode;

    use crate::dev_http::BodyError;
    use crate::io_cause::IoCause;

    let text = || "x".to_owned();
    let errors = vec![
        MemoryError::InvalidBody(BodyError::from_json(
            &serde_json::from_str::<Value>("{").unwrap_err(),
        )),
        MemoryError::InvalidQuery { detail: text() },
        MemoryError::InvalidOwner { owner: text() },
        MemoryError::InvalidId { id: text() },
        MemoryError::InvalidKind { kind: text() },
        MemoryError::InvalidPlace { place: text() },
        MemoryError::InvalidTag { tag: text() },
        MemoryError::InvalidLink { id: text() },
        MemoryError::InvalidPicture { picture: text() },
        MemoryError::InvalidName { name: text() },
        MemoryError::InvalidCursor,
        MemoryError::InvalidSchema { schema: 9 },
        MemoryError::InvalidSequence { id: text() },
        MemoryError::UnknownOwner { owner: text() },
        MemoryError::UnknownMemory {
            owner: text(),
            id: text(),
        },
        MemoryError::UnknownPicture { picture: text() },
        MemoryError::OwnerMismatch {
            owner: text(),
            book: text(),
        },
        MemoryError::DuplicateId { id: text() },
        MemoryError::DuplicatePicture { picture: text() },
        MemoryError::DuplicateTag { tag: text() },
        MemoryError::DuplicateLink { id: text() },
        MemoryError::DuplicateSelected { id: text() },
        MemoryError::ConflictingSelection,
        MemoryError::EmptyTitle { id: text() },
        MemoryError::EmptyDescription { id: text() },
        MemoryError::TooLong {
            field: MemoryField::Title,
            maximum: 1,
        },
        MemoryError::TooMany {
            field: MemoryField::Pictures,
            maximum: 1,
        },
        MemoryError::NothingToCompose,
        MemoryError::Gallery {
            cause: df_imagery::gallery::GalleryError::Empty,
        },
        MemoryError::PictureUnreadable {
            picture: text(),
            cause: IoCause::Other,
        },
        MemoryError::PictureNotStored { detail: text() },
        MemoryError::StoreUnreadable {
            cause: IoCause::Other,
        },
        MemoryError::StoreInvalid {
            cause: BodyError::from_json(&serde_json::from_str::<Value>("{").unwrap_err()),
        },
        MemoryError::StoreUnwritable {
            cause: IoCause::Other,
        },
        MemoryError::Stopped,
    ];
    let keys: BTreeSet<&str> = errors.iter().map(MemoryError::key).collect();
    assert_eq!(keys.len(), errors.len(), "every error has its own key");
    for error in &errors {
        assert_eq!(json!(error)["error"], error.key(), "{error:?}");
        assert!(!error.to_string().is_empty());
        assert!(
            error
                .to_string()
                .chars()
                .next()
                .is_some_and(|c| !c.is_uppercase()),
            "{error} is a lowercase sentence"
        );
        let back: MemoryError = serde_json::from_value(json!(error)).unwrap();
        assert_eq!(&back, error);
    }
    assert_eq!(MemoryError::InvalidCursor.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        MemoryError::UnknownOwner { owner: text() }.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        MemoryError::NothingToCompose.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        MemoryError::Stopped.status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
}
