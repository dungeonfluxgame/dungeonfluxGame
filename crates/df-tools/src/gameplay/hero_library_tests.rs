//! Local wire fake: exercises the actual PostgreSQL client and transaction commands,
//! without starting a database or making network/provider calls.
use super::*;
use futures::FutureExt;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

#[derive(Clone)]
struct StoredHero {
    deleted: bool,
    revisions: usize,
    portrait_bytes: usize,
    receipts: usize,
    sources: usize,
}
struct FakeDatabase {
    hero: Option<StoredHero>,
    queries: Vec<String>,
    fail_revisions: bool,
}
impl FakeDatabase {
    fn seeded() -> Self {
        Self {
            hero: Some(StoredHero {
                deleted: false,
                revisions: 2,
                portrait_bytes: 93,
                receipts: 2,
                sources: 1,
            }),
            queries: Vec::new(),
            fail_revisions: false,
        }
    }
}
fn field_types(sql: &str) -> (Vec<u32>, Vec<u32>) {
    if sql.contains("df_local_demo.grants") {
        (vec![17; 4], vec![23])
    } else if sql.starts_with("SELECT DISTINCT r.portrait_sha256") {
        (vec![20], vec![17])
    } else if sql.starts_with("SELECT id FROM profiles") {
        (vec![17], vec![20])
    } else if sql.starts_with("SELECT p.id") {
        (vec![20], vec![20])
    } else if sql.starts_with("UPDATE heroes") {
        (vec![20, 20], vec![])
    } else {
        (vec![20], vec![])
    }
}
async fn frame(stream: &mut DuplexStream, kind: u8, bytes: &[u8]) {
    stream.write_u8(kind).await.unwrap();
    stream.write_u32((bytes.len() + 4) as u32).await.unwrap();
    stream.write_all(bytes).await.unwrap();
}
fn cstring(bytes: &[u8], cursor: &mut usize) -> String {
    let length = bytes[*cursor..].iter().position(|byte| *byte == 0).unwrap();
    let result = String::from_utf8(bytes[*cursor..*cursor + length].to_vec()).unwrap();
    *cursor += length + 1;
    result
}
fn word(bytes: &[u8], cursor: &mut usize) -> usize {
    let value = u16::from_be_bytes(bytes[*cursor..*cursor + 2].try_into().unwrap());
    *cursor += 2;
    usize::from(value)
}
fn parameters(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut cursor = 0;
    cstring(bytes, &mut cursor);
    cstring(bytes, &mut cursor);
    let formats = word(bytes, &mut cursor);
    cursor += 2 * formats;
    let count = word(bytes, &mut cursor);
    (0..count)
        .map(|_| {
            let length = i32::from_be_bytes(bytes[cursor..cursor + 4].try_into().unwrap());
            cursor += 4;
            assert!(length >= 0);
            let result = bytes[cursor..cursor + length as usize].to_vec();
            cursor += length as usize;
            result
        })
        .collect()
}
fn number(bytes: &[u8]) -> i64 {
    i64::from_be_bytes(bytes.try_into().unwrap())
}
async fn fake_connection(
    database: Arc<Mutex<FakeDatabase>>,
) -> (Connection, tokio::task::JoinHandle<()>) {
    let (client_stream, mut server) = tokio::io::duplex(16384);
    let task = tokio::spawn(async move {
        let run = async {
            let startup = server.read_u32().await.unwrap();
            let mut body = vec![0; startup as usize - 4];
            server.read_exact(&mut body).await.unwrap();
            frame(&mut server, b'R', &0u32.to_be_bytes()).await;
            frame(&mut server, b'Z', b"I").await;
            let mut sql = String::new();
            let mut params = Vec::new();
            let mut snapshot = None;
            loop {
                let Ok(kind) = server.read_u8().await else {
                    break;
                };
                let length = server.read_u32().await.unwrap();
                let mut body = vec![0; length as usize - 4];
                server.read_exact(&mut body).await.unwrap();
                match kind {
                    b'P' => {
                        let mut cursor = 0;
                        cstring(&body, &mut cursor);
                        sql = cstring(&body, &mut cursor);
                        frame(&mut server, b'1', &[]).await;
                    }
                    b'D' => {
                        let (params, columns) = field_types(&sql);
                        let mut description = (params.len() as u16).to_be_bytes().to_vec();
                        for oid in params {
                            description.extend(oid.to_be_bytes());
                        }
                        frame(&mut server, b't', &description).await;
                        if columns.is_empty() {
                            frame(&mut server, b'n', &[]).await;
                        } else {
                            let mut description = (columns.len() as u16).to_be_bytes().to_vec();
                            for oid in columns {
                                description.extend(b"value\0");
                                description.extend(0u32.to_be_bytes());
                                description.extend(0u16.to_be_bytes());
                                description.extend(oid.to_be_bytes());
                                description.extend((-1i16).to_be_bytes());
                                description.extend((-1i32).to_be_bytes());
                                description.extend(1u16.to_be_bytes());
                            }
                            frame(&mut server, b'T', &description).await;
                        }
                    }
                    b'B' => {
                        params = parameters(&body);
                        frame(&mut server, b'2', &[]).await;
                    }
                    b'E' => {
                        let result = {
                            let mut db = database.lock().unwrap();
                            db.queries.push(sql.clone());
                            if sql.starts_with("DELETE FROM hero_revisions") && db.fail_revisions {
                                Err(())
                            } else {
                                let mut row = None;
                                let mut changed = 0;
                                if sql.contains("df_local_demo.grants") {
                                    if params[0] == [9; 32] {
                                        row = Some(1i32.to_be_bytes().to_vec());
                                    }
                                } else if sql.starts_with("SELECT DISTINCT r.portrait_sha256") {
                                    assert!(db.hero.as_ref().unwrap().deleted);
                                    row = Some(vec![77; 32]);
                                } else if sql.starts_with("SELECT id FROM profiles") {
                                    row = match params[0].as_slice() {
                                        bytes if bytes == [7; 32] => {
                                            Some(1i64.to_be_bytes().to_vec())
                                        }
                                        bytes if bytes == [8; 32] => {
                                            Some(2i64.to_be_bytes().to_vec())
                                        }
                                        _ => None,
                                    };
                                } else if sql.starts_with("SELECT p.id") {
                                    if number(&params[0]) == 42 && db.hero.is_some() {
                                        row = Some(1i64.to_be_bytes().to_vec());
                                    }
                                } else if sql.starts_with("UPDATE heroes") {
                                    if number(&params[0]) == 42
                                        && number(&params[1]) == 1
                                        && let Some(hero) =
                                            db.hero.as_mut().filter(|hero| !hero.deleted)
                                    {
                                        hero.deleted = true;
                                        changed = 1;
                                    }
                                } else if let Some(hero) = db.hero.as_mut() {
                                    assert_eq!(number(&params[0]), 42);
                                    assert!(
                                        hero.deleted,
                                        "erasure requires tombstoning in this transaction"
                                    );
                                    if sql.starts_with("DELETE FROM hero_save_receipts") {
                                        hero.receipts = 0;
                                    } else if sql.starts_with("DELETE FROM hero_sources") {
                                        hero.sources = 0;
                                    } else if sql.starts_with("DELETE FROM hero_revisions") {
                                        assert_eq!(
                                            hero.receipts, 0,
                                            "revision foreign keys must be removed first"
                                        );
                                        hero.revisions = 0;
                                        hero.portrait_bytes = 0;
                                    } else if sql.starts_with("DELETE FROM heroes") {
                                        assert_eq!(
                                            (
                                                hero.revisions,
                                                hero.portrait_bytes,
                                                hero.sources,
                                                hero.receipts
                                            ),
                                            (0, 0, 0, 0)
                                        );
                                        db.hero = None;
                                    } else {
                                        panic!("unexpected SQL {sql}");
                                    }
                                }
                                Ok((row, changed))
                            }
                        };
                        match result {
                            Err(()) => {
                                frame(
                                    &mut server,
                                    b'E',
                                    b"SERROR\0C23503\0Msynthetic revision failure\0\0",
                                )
                                .await
                            }
                            Ok((row, changed)) => {
                                if let Some(row) = row {
                                    let mut data = 1u16.to_be_bytes().to_vec();
                                    data.extend((row.len() as u32).to_be_bytes());
                                    data.extend(row);
                                    frame(&mut server, b'D', &data).await;
                                }
                                let command = if sql.starts_with("SELECT") {
                                    "SELECT 1".to_owned()
                                } else if sql.starts_with("UPDATE") {
                                    format!("UPDATE {changed}")
                                } else {
                                    "DELETE 1".to_owned()
                                };
                                frame(&mut server, b'C', format!("{command}\0").as_bytes()).await;
                            }
                        }
                    }
                    b'Q' => {
                        let command = std::str::from_utf8(&body[..body.len() - 1]).unwrap();
                        let mut tag = command.to_owned();
                        {
                            let mut db = database.lock().unwrap();
                            db.queries.push(command.into());
                            match command {
                                "BEGIN" | "START TRANSACTION" => snapshot = Some(db.hero.clone()),
                                "COMMIT" => {
                                    snapshot = None;
                                }
                                "ROLLBACK" => {
                                    db.hero = snapshot.take().unwrap();
                                }
                                _ => {
                                    tag = "SET".into();
                                }
                            }
                        }
                        frame(&mut server, b'C', format!("{tag}\0").as_bytes()).await;
                        frame(
                            &mut server,
                            b'Z',
                            if snapshot.is_some() { b"T" } else { b"I" },
                        )
                        .await;
                    }
                    b'S' => {
                        frame(
                            &mut server,
                            b'Z',
                            if snapshot.is_some() { b"T" } else { b"I" },
                        )
                        .await
                    }
                    b'C' => frame(&mut server, b'3', &[]).await,
                    b'X' => break,
                    _ => panic!("unexpected protocol byte {kind}"),
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(20), run)
            .await
            .unwrap();
    });
    let mut config = tokio_postgres::Config::new();
    config
        .user("synthetic")
        .ssl_mode(tokio_postgres::config::SslMode::Disable);
    let (client, connection) = config
        .connect_raw(client_stream, tokio_postgres::NoTls)
        .await
        .unwrap();
    (
        Connection {
            client,
            driver: tokio::spawn(connection),
        },
        task,
    )
}

#[tokio::test]
async fn delete_erases_revisions_portraits_and_receipts_without_foreign_existence_leak() {
    let database = Arc::new(Mutex::new(FakeDatabase::seeded()));
    let (mut connection, server) = fake_connection(database.clone()).await;
    assert!(
        !delete_from(&mut connection.client, Some(&[8; 32]), 42)
            .await
            .unwrap()
    );
    assert!(
        !delete_from(&mut connection.client, Some(&[8; 32]), 999)
            .await
            .unwrap()
    );
    assert!(
        !delete_from(&mut connection.client, Some(&[6; 32]), 42)
            .await
            .unwrap()
    );
    assert_eq!(database.lock().unwrap().hero.as_ref().unwrap().revisions, 2);
    assert!(
        delete_from(&mut connection.client, Some(&[7; 32]), 42)
            .await
            .unwrap()
    );
    assert!(database.lock().unwrap().hero.is_none());
    assert!(
        !delete_from(&mut connection.client, Some(&[7; 32]), 42)
            .await
            .unwrap()
    );
    database.lock().unwrap().hero = FakeDatabase::seeded().hero;
    assert!(delete_from(&mut connection.client, None, 42).await.unwrap());
    assert!(!delete_from(&mut connection.client, None, 42).await.unwrap());
    drop(connection);
    server.await.unwrap();
}

#[tokio::test]
async fn failed_erasure_rolls_back_every_library_row() {
    let database = Arc::new(Mutex::new(FakeDatabase::seeded()));
    database.lock().unwrap().fail_revisions = true;
    let (mut connection, server) = fake_connection(database.clone()).await;
    assert!(
        delete_from(&mut connection.client, Some(&[7; 32]), 42)
            .await
            .is_err()
    );
    // The driver's queued rollback is ordered before the next query.
    connection
        .client
        .simple_query("SET application_name='rollback-barrier'")
        .await
        .unwrap();
    let hero = database.lock().unwrap().hero.clone().unwrap();
    assert!(!hero.deleted);
    assert_eq!(
        (
            hero.revisions,
            hero.portrait_bytes,
            hero.receipts,
            hero.sources
        ),
        (2, 93, 2, 1)
    );
    drop(connection);
    server.await.unwrap();
}

fn service() -> super::super::Service {
    let (_, updates) = tokio::sync::watch::channel(super::super::model::initial().unwrap());
    let (actor, _inbox) = df_session::inbox::bounded_inbox();
    super::super::Service {
        look_ahead: Default::default(),
        actor,
        display_credential: [11; 32],
        updates,
        streams: Arc::new(tokio::sync::Semaphore::new(1)),
        voice: super::super::voice::Voice::disabled(
            super::super::startup_configuration::VoiceOff::Qualification,
        ),
        npc_voice: Arc::new(super::super::npc_reply::NpcVoice::new(Arc::new(
            super::super::storyteller::Storyteller::disabled(),
        ))),
        table_log: super::super::table_log::TableLog::new(),
        social: super::super::social::SocialLedger::none(),
        living: super::super::living_scenes::LivingScenes::disabled(),
        layout: super::super::dev_control::layout::LayoutState::new(),
        portraits: super::super::creator_portraits::Portraits::disabled(),
    }
}
#[tokio::test]
async fn load_authenticates_before_touching_imports_and_portraits_reject_before_database_work() {
    let database = Arc::new(Mutex::new(FakeDatabase::seeded()));
    let (library, library_server) = fake_connection(database.clone()).await;
    let (campaign, campaign_server) = fake_connection(database.clone()).await;
    *ACCESS.lock().await = Some(DatabaseAccess { library, campaign });
    let credential = [10; 32];
    PORTRAIT_IMPORTS.lock().unwrap().insert(
        credential,
        PortraitImport {
            identity: rpc::LibraryHeroIdentity::default(),
            attachment: super::super::creator_portraits::Attachment {
                key: "a".repeat(64),
                digest: [1; 32],
                bytes: 5,
                revision: 1,
            },
        },
    );
    let service = service();
    let mut request = Request::new(rpc::LoadHeroRequest {
        key: vec![7; 32],
        hero_id: 0,
        new_hero: true,
    });
    request.metadata_mut().insert(
        "x-df-local-binding",
        super::super::hex(&credential).parse().unwrap(),
    );
    let error = service.load_hero_inner(request).await.unwrap_err();
    assert_eq!(error.code(), tonic::Code::PermissionDenied);
    assert!(PORTRAIT_IMPORTS.lock().unwrap().contains_key(&credential));
    assert_eq!(
        database.lock().unwrap().queries.len(),
        1,
        "only grant authentication ran"
    );
    // Hold the only database access lock. Invalid/unknown URLs must still finish immediately.
    let access = ACCESS.lock().await;
    for key in ["invalid".to_owned(), "z".repeat(64), "f".repeat(64)] {
        let response = serve_portrait(axum::extract::Path(key))
            .now_or_never()
            .expect("rejected without waiting for a database");
        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }
    assert_eq!(database.lock().unwrap().queries.len(), 1);
    drop(access);
    use rpc::hero_library_service_server::HeroLibraryService;
    let seat_hash = df_observe::seat_hash(&[9; 32]).to_string();
    let audit_prefix = format!("HERO_LIBRARY operation=delete seat_hash={seat_hash}");
    let before = super::super::actor::audit_lines_containing(&audit_prefix).len();
    for (key, id, removed) in [
        (8, 42, false),
        (8, 999, false),
        (7, 42, true),
        (7, 42, false),
    ] {
        let body = rpc::DeleteHeroRequest {
            key: vec![key; 32],
            hero_id: id,
        };
        let body = rpc::DeleteHeroRequest::decode(body.encode_to_vec().as_slice()).unwrap();
        let mut request = Request::new(body);
        request.metadata_mut().insert(
            "x-df-local-binding",
            super::super::hex(&[9; 32]).parse().unwrap(),
        );
        assert_eq!(
            service
                .delete_hero(request)
                .await
                .unwrap()
                .into_inner()
                .removed,
            removed
        );
    }
    let lines = super::super::actor::audit_lines_containing(&audit_prefix);
    assert_eq!(lines.len() - before, 4);
    for line in &lines[before..] {
        assert!(line.starts_with(&format!(
            "{audit_prefix} outcome=answered reason=success step=delete ms="
        )));
        assert!(line.contains(&format!("seat={seat_hash}")));
        assert!(
            line.split_ascii_whitespace()
                .find_map(|field| field.strip_prefix("ms="))
                .is_some_and(|value| value.parse::<u128>().is_ok())
        );
    }
    // Erasing a library row must not revoke the existing creator's approved cache copy.
    assert!(known_portrait_key(&super::super::hex(&[1; 32])).is_some());
    PORTRAIT_IMPORTS.lock().unwrap().remove(&credential);
    assert!(known_portrait_key(&super::super::hex(&[1; 32])).is_none());
    ACCESS.lock().await.take();
    library_server.await.unwrap();
    campaign_server.await.unwrap();
}
