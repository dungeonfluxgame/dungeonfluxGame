//! REF-046: local PC identity library, separate from campaign state.
//! Opaque identity records are versioned by their transport owner; no mechanics are stored here.
use std::{fmt, time::Duration};
use tokio_postgres::{Client, Config, NoTls, Row};

pub const DATABASE: &str = "df_gameplay_demo_library";
pub const MAX_PORTRAIT_BYTES: usize = 6 * 1024 * 1024;
pub const MAX_IDENTITY_BYTES: usize = 24 * 1024;

#[derive(Debug)]
pub enum LibraryError {
    Database(tokio_postgres::Error),
    Deadline,
    InvalidRecord,
    NotFound,
}
impl fmt::Display for LibraryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Database(_) => "hero library database unavailable",
            Self::Deadline => "hero library deadline exceeded",
            Self::InvalidRecord => "hero library record invalid",
            Self::NotFound => "hero library record unavailable",
        })
    }
}
impl std::error::Error for LibraryError {}
impl From<tokio_postgres::Error> for LibraryError {
    fn from(error: tokio_postgres::Error) -> Self {
        Self::Database(error)
    }
}

/// Secret input deliberately implements neither Debug nor Display.
pub struct PhoneKey([u8; 32]);
impl PhoneKey {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, LibraryError> {
        Ok(Self(
            bytes.try_into().map_err(|_| LibraryError::InvalidRecord)?,
        ))
    }
}

pub struct IdentityRecord {
    pub name: String,
    pub species: String,
    pub character_class: String,
    pub encoded: Vec<u8>,
    pub portrait: Vec<u8>,
    pub portrait_thumbnail: Vec<u8>,
    pub portrait_media_type: String,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionRef {
    pub hero_id: i64,
    pub revision: i64,
}
pub struct HeroRevision {
    pub reference: RevisionRef,
    pub record: IdentityRecord,
    pub portrait_sha256: Vec<u8>,
}
/// Connections are operation-owned; dropping or cancelling work stops its driver.
pub struct HeroLibrary {
    client: Client,
    driver: tokio::task::JoinHandle<Result<(), tokio_postgres::Error>>,
}
impl Drop for HeroLibrary {
    fn drop(&mut self) {
        self.driver.abort();
    }
}

const SCHEMA: &str = "
BEGIN;
SELECT pg_advisory_xact_lock(724190046);
CREATE TABLE IF NOT EXISTS profiles (
 id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
 key_sha256 bytea NOT NULL UNIQUE CHECK(octet_length(key_sha256)=32),
 created timestamptz NOT NULL DEFAULT now(), last_seen timestamptz NOT NULL DEFAULT now());
CREATE TABLE IF NOT EXISTS heroes (
 id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY, profile bigint NOT NULL REFERENCES profiles(id),
 created timestamptz NOT NULL DEFAULT now(), deleted boolean NOT NULL DEFAULT false);
CREATE INDEX IF NOT EXISTS heroes_profile ON heroes(profile,id);
CREATE TABLE IF NOT EXISTS hero_revisions (
 hero bigint NOT NULL REFERENCES heroes(id), revision bigint NOT NULL CHECK(revision>0),
 created timestamptz NOT NULL DEFAULT now(), name text NOT NULL, species text NOT NULL,
 character_class text NOT NULL, identity_record bytea NOT NULL CHECK(octet_length(identity_record)<=24576),
 identity_digest bytea NOT NULL CHECK(octet_length(identity_digest)=32),
 portrait bytea NOT NULL CHECK(octet_length(portrait)<=6291456),
 portrait_sha256 bytea NOT NULL CHECK(octet_length(portrait_sha256)=32), portrait_media_type text NOT NULL,
 portrait_thumbnail bytea NOT NULL CHECK(octet_length(portrait_thumbnail)<=32768),
 PRIMARY KEY(hero,revision), UNIQUE(hero,identity_digest));
CREATE TABLE IF NOT EXISTS hero_sources (
 profile bigint NOT NULL REFERENCES profiles(id), campaign text NOT NULL, seat bytea NOT NULL,
 hero bigint NOT NULL REFERENCES heroes(id), PRIMARY KEY(profile,campaign,seat));
CREATE TABLE IF NOT EXISTS hero_save_receipts (
 profile bigint NOT NULL REFERENCES profiles(id), campaign text NOT NULL, seat bytea NOT NULL,
 identity_digest bytea NOT NULL, hero bigint NOT NULL, revision bigint NOT NULL,
 PRIMARY KEY(profile,campaign,seat,identity_digest),
 FOREIGN KEY(hero,revision) REFERENCES hero_revisions(hero,revision));
CREATE OR REPLACE FUNCTION df_library_immutable_revision() RETURNS trigger LANGUAGE plpgsql AS $$
 BEGIN RAISE EXCEPTION 'hero revisions are immutable'; END $$;
DROP TRIGGER IF EXISTS immutable_revision ON hero_revisions;
CREATE TRIGGER immutable_revision BEFORE UPDATE OR DELETE ON hero_revisions
 FOR EACH ROW EXECUTE FUNCTION df_library_immutable_revision();
COMMIT;";

impl HeroLibrary {
    /// Uses the game's existing cluster/user, creates only the fixed library database.
    /// Duplicate database races are benign; all queries have a five-second server deadline.
    pub async fn open(cluster: &Config) -> Result<Self, LibraryError> {
        let mut config = cluster.clone();
        config.dbname("postgres");
        let admin = Self::connect(&config).await?;
        // CREATE DATABASE cannot run in a transaction. A session lock serializes the
        // absent-database check across launches; dropping this owned connection releases it.
        admin
            .client
            .query_one("SELECT pg_advisory_lock(724190046)", &[])
            .await?;
        if admin
            .client
            .query_opt("SELECT 1 FROM pg_database WHERE datname=$1", &[&DATABASE])
            .await?
            .is_none()
            && let Err(error) = admin
                .client
                .batch_execute("CREATE DATABASE df_gameplay_demo_library")
                .await
            && error.code() != Some(&tokio_postgres::error::SqlState::DUPLICATE_DATABASE)
        {
            return Err(error.into());
        }
        drop(admin);
        config.dbname(DATABASE);
        let db = Self::connect(&config).await?;
        db.client.batch_execute(SCHEMA).await?;
        Ok(db)
    }
    /// Exposed for owned scratch PostgreSQL fixtures; does not create a database.
    pub async fn in_database(config: &Config) -> Result<Self, LibraryError> {
        let db = Self::connect(config).await?;
        db.client.batch_execute(SCHEMA).await?;
        Ok(db)
    }
    async fn connect(config: &Config) -> Result<Self, LibraryError> {
        let (client, connection) =
            tokio::time::timeout(Duration::from_secs(5), config.connect(NoTls))
                .await
                .map_err(|_| LibraryError::Deadline)??;
        let db = Self {
            client,
            driver: tokio::spawn(connection),
        };
        db.client.batch_execute("SET statement_timeout='5s'; SET lock_timeout='5s'; SET idle_in_transaction_session_timeout='5s'").await?;
        Ok(db)
    }
    pub async fn list(&self, key: &PhoneKey) -> Result<Vec<HeroRevision>, LibraryError> {
        self.list_after(key, 0).await
    }
    pub async fn list_after(
        &self,
        key: &PhoneKey,
        after: i64,
    ) -> Result<Vec<HeroRevision>, LibraryError> {
        self.client
            .execute(
                "UPDATE profiles SET last_seen=now() WHERE key_sha256=sha256($1)",
                &[&&key.0[..]],
            )
            .await?;
        self.rows("SELECT r.hero,r.revision,r.name,r.species,r.character_class,r.identity_record,r.portrait_thumbnail AS portrait,r.portrait_thumbnail,r.portrait_sha256,r.portrait_media_type FROM heroes h JOIN profiles p ON p.id=h.profile JOIN LATERAL (SELECT * FROM hero_revisions WHERE hero=h.id ORDER BY revision DESC LIMIT 1) r ON true WHERE p.key_sha256=sha256($1) AND NOT h.deleted AND h.id>$2 ORDER BY h.id LIMIT 100", &[&&key.0[..],&after]).await
    }
    pub async fn load(&self, key: &PhoneKey, hero: i64) -> Result<HeroRevision, LibraryError> {
        self.rows("SELECT r.* FROM heroes h JOIN profiles p ON p.id=h.profile JOIN hero_revisions r ON r.hero=h.id WHERE p.key_sha256=sha256($1) AND h.id=$2 AND NOT h.deleted ORDER BY r.revision DESC LIMIT 1", &[&&key.0[..], &hero]).await?.pop().ok_or(LibraryError::NotFound)
    }
    /// A phone may name only an existing revision of one of its own active heroes.
    pub async fn load_revision(
        &self,
        key: &PhoneKey,
        reference: &RevisionRef,
    ) -> Result<HeroRevision, LibraryError> {
        self.rows("SELECT r.* FROM heroes h JOIN profiles p ON p.id=h.profile JOIN hero_revisions r ON r.hero=h.id WHERE p.key_sha256=sha256($1) AND h.id=$2 AND r.revision=$3 AND NOT h.deleted", &[&&key.0[..], &reference.hero_id, &reference.revision]).await?.pop().ok_or(LibraryError::NotFound)
    }
    /// Content-addressed portrait route; no process-local loaded-hero registry is needed.
    pub async fn portrait_by_digest(
        &self,
        digest: &[u8; 32],
    ) -> Result<Option<Vec<u8>>, LibraryError> {
        Ok(self.client.query_opt("SELECT r.portrait FROM hero_revisions r JOIN heroes h ON h.id=r.hero WHERE r.portrait_sha256=$1 AND r.portrait_media_type='image/jpeg' AND NOT h.deleted LIMIT 1", &[&&digest[..]]).await?.map(|row| row.get(0)))
    }
    /// Serializes saves per profile. Canonical identity + portrait digest deduplicates across
    /// retries; a receipt pins the exact source campaign/seat/digest even after later edits.
    pub async fn save(
        &mut self,
        key: &PhoneKey,
        campaign: &str,
        seat: &[u8],
        record: &IdentityRecord,
    ) -> Result<RevisionRef, LibraryError> {
        self.save_from(key, campaign, seat, record, None).await
    }
    pub async fn save_from(
        &mut self,
        key: &PhoneKey,
        campaign: &str,
        seat: &[u8],
        record: &IdentityRecord,
        origin: Option<&RevisionRef>,
    ) -> Result<RevisionRef, LibraryError> {
        if campaign.is_empty()
            || campaign.len() > 128
            || seat.len() != 16
            || record.encoded.len() > MAX_IDENTITY_BYTES
            || record.encoded.is_empty()
            || record.name.is_empty()
            || record.name.len() > 96
            || record.species.len() > 64
            || record.character_class.len() > 64
            || record.portrait.len() > MAX_PORTRAIT_BYTES
            || record.portrait_thumbnail.len() > 32768
            || !matches!(
                record.portrait_media_type.as_str(),
                "" | "image/jpeg" | "image/png" | "image/webp"
            )
        {
            return Err(LibraryError::InvalidRecord);
        }
        let transaction = self.client.transaction().await?;
        let profile: i64 = transaction.query_one("INSERT INTO profiles(key_sha256) VALUES(sha256($1)) ON CONFLICT(key_sha256) DO UPDATE SET last_seen=now() RETURNING id", &[&&key.0[..]]).await?.get(0);
        // Untrusted hints never expose another profile or make an otherwise valid save fail.
        let has_origin_hint = origin.is_some();
        let origin = if let Some(origin) = origin {
            transaction.query_opt("SELECT h.id FROM heroes h JOIN hero_revisions r ON r.hero=h.id WHERE h.id=$1 AND h.profile=$2 AND r.revision=$3 AND NOT h.deleted", &[&origin.hero_id, &profile, &origin.revision]).await?.map(|row| row.get::<_, i64>(0))
        } else {
            None
        };
        let digest: Vec<u8> = transaction
            .query_one(
                "SELECT sha256($1::bytea || sha256($2::bytea) || convert_to($3::text,'UTF8'))",
                &[
                    &record.encoded,
                    &record.portrait,
                    &record.portrait_media_type,
                ],
            )
            .await?
            .get(0);
        if let Some(row) = transaction.query_opt("SELECT hero,revision FROM hero_save_receipts WHERE profile=$1 AND campaign=$2 AND seat=$3 AND identity_digest=$4", &[&profile,&campaign,&seat,&digest]).await? {
            let reference=RevisionRef{hero_id:row.get(0),revision:row.get(1)};
            transaction.commit().await?;
            return Ok(reference);
        }
        if !has_origin_hint && let Some(row) = transaction.query_opt("SELECT r.hero,r.revision FROM hero_revisions r JOIN heroes h ON h.id=r.hero WHERE h.profile=$1 AND NOT h.deleted AND r.identity_digest=$2 ORDER BY r.hero,r.revision LIMIT 1", &[&profile,&digest]).await? {
            let hero: i64=row.get(0);let revision:i64=row.get(1);
            transaction.execute("INSERT INTO hero_sources(profile,campaign,seat,hero) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING", &[&profile,&campaign,&seat,&hero]).await?;
            transaction.execute("INSERT INTO hero_save_receipts(profile,campaign,seat,identity_digest,hero,revision) VALUES($1,$2,$3,$4,$5,$6)", &[&profile,&campaign,&seat,&digest,&hero,&revision]).await?;
            transaction.commit().await?;
            return Ok(RevisionRef{hero_id:hero,revision});
        }
        if let Some(origin) = origin {
            transaction.execute("INSERT INTO hero_sources(profile,campaign,seat,hero) VALUES($1,$2,$3,$4) ON CONFLICT(profile,campaign,seat) DO UPDATE SET hero=excluded.hero", &[&profile,&campaign,&seat,&origin]).await?;
        }
        let existing = if has_origin_hint && origin.is_none() {
            None
        } else {
            transaction.query_opt("SELECT s.hero FROM hero_sources s JOIN heroes h ON h.id=s.hero WHERE s.profile=$1 AND s.campaign=$2 AND s.seat=$3 AND NOT h.deleted", &[&profile,&campaign,&seat]).await?
        };
        let hero: i64 = if let Some(row) = existing {
            row.get(0)
        } else {
            let hero = transaction
                .query_one(
                    "INSERT INTO heroes(profile) VALUES($1) RETURNING id",
                    &[&profile],
                )
                .await?
                .get(0);
            transaction.execute("INSERT INTO hero_sources(profile,campaign,seat,hero) VALUES($1,$2,$3,$4) ON CONFLICT(profile,campaign,seat) DO UPDATE SET hero=excluded.hero", &[&profile,&campaign,&seat,&hero]).await?;
            hero
        };
        let revision: i64 = if let Some(row) = transaction
            .query_opt(
                "SELECT revision FROM hero_revisions WHERE hero=$1 AND identity_digest=$2",
                &[&hero, &digest],
            )
            .await?
        {
            row.get(0)
        } else {
            let revision: i64 = transaction
                .query_one(
                    "SELECT coalesce(max(revision),0)+1 FROM hero_revisions WHERE hero=$1",
                    &[&hero],
                )
                .await?
                .get(0);
            transaction.execute("INSERT INTO hero_revisions(hero,revision,name,species,character_class,identity_record,identity_digest,portrait,portrait_sha256,portrait_media_type,portrait_thumbnail) VALUES($1,$2,$3,$4,$5,$6,$7,$8,sha256($8),$9,$10)", &[&hero,&revision,&record.name,&record.species,&record.character_class,&record.encoded,&digest,&record.portrait,&record.portrait_media_type,&record.portrait_thumbnail]).await?;
            revision
        };
        transaction.execute("INSERT INTO hero_save_receipts(profile,campaign,seat,identity_digest,hero,revision) VALUES($1,$2,$3,$4,$5,$6)", &[&profile,&campaign,&seat,&digest,&hero,&revision]).await?;
        transaction.commit().await?;
        Ok(RevisionRef {
            hero_id: hero,
            revision,
        })
    }
    async fn rows(
        &self,
        sql: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> Result<Vec<HeroRevision>, LibraryError> {
        self.client
            .query(sql, params)
            .await?
            .iter()
            .map(decode)
            .collect()
    }
    pub async fn admin_list(&self) -> Result<Vec<HeroRevision>, LibraryError> {
        self.rows("SELECT r.* FROM heroes h JOIN LATERAL (SELECT * FROM hero_revisions WHERE hero=h.id ORDER BY revision DESC LIMIT 1) r ON true WHERE NOT h.deleted ORDER BY h.id LIMIT 100", &[]).await
    }
    pub async fn revisions(&self, hero: i64) -> Result<Vec<HeroRevision>, LibraryError> {
        self.rows(
            "SELECT * FROM hero_revisions WHERE hero=$1 ORDER BY revision DESC LIMIT 100",
            &[&hero],
        )
        .await
    }
    pub async fn delete(&self, hero: i64) -> Result<bool, LibraryError> {
        Ok(self
            .client
            .execute(
                "UPDATE heroes SET deleted=true WHERE id=$1 AND NOT deleted",
                &[&hero],
            )
            .await?
            == 1)
    }
    /// Campaign links are durable metadata, never a second authority over the committed hero.
    /// A failed link write is retried with the same library receipt, without another revision.
    pub async fn record_campaign_origin(
        config: &Config,
        seat: &[u8],
        reference: &RevisionRef,
    ) -> Result<(), LibraryError> {
        let db = Self::connect(config).await?;
        db.client.batch_execute("CREATE TABLE IF NOT EXISTS df_hero_library_origins(seat bytea PRIMARY KEY,hero bigint NOT NULL,revision bigint NOT NULL,created timestamptz NOT NULL DEFAULT now())").await?;
        db.client.execute("INSERT INTO df_hero_library_origins(seat,hero,revision) VALUES($1,$2,$3) ON CONFLICT DO NOTHING", &[&seat,&reference.hero_id,&reference.revision]).await?;
        Ok(())
    }
    pub async fn record_campaign_link(
        config: &Config,
        seat: &[u8],
        reference: &RevisionRef,
    ) -> Result<(), LibraryError> {
        let db = Self::connect(config).await?;
        db.client.batch_execute("CREATE TABLE IF NOT EXISTS df_hero_library_links(seat bytea NOT NULL, hero bigint NOT NULL, revision bigint NOT NULL, created timestamptz NOT NULL DEFAULT now(), PRIMARY KEY(seat,hero,revision))").await?;
        db.client.execute("INSERT INTO df_hero_library_links(seat,hero,revision) VALUES($1,$2,$3) ON CONFLICT DO NOTHING", &[&seat,&reference.hero_id,&reference.revision]).await?;
        Ok(())
    }
}
fn decode(row: &Row) -> Result<HeroRevision, LibraryError> {
    Ok(HeroRevision {
        reference: RevisionRef {
            hero_id: row.try_get("hero")?,
            revision: row.try_get("revision")?,
        },
        record: IdentityRecord {
            name: row.try_get("name")?,
            species: row.try_get("species")?,
            character_class: row.try_get("character_class")?,
            encoded: row.try_get("identity_record")?,
            portrait: row.try_get("portrait")?,
            portrait_thumbnail: row.try_get("portrait_thumbnail")?,
            portrait_media_type: row.try_get("portrait_media_type")?,
        },
        portrait_sha256: row.try_get("portrait_sha256")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn library_key_requires_exactly_32_bytes() {
        assert!(PhoneKey::from_bytes(&[0; 31]).is_err());
        assert!(PhoneKey::from_bytes(&[1; 32]).is_ok());
    }
    #[test]
    fn loaded_reference_survives_fresh_connection_and_reuses_or_advances_revision() {
        with_owned_library(|config| async move {
            let key = PhoneKey::from_bytes(&[81; 32]).unwrap();
            let mut record = fixture_record("Restart hero");
            let campaign = fixture_campaign("restart");
            record.encoded.extend_from_slice(campaign.as_bytes());
            let initial_encoded = record.encoded.clone();
            let mut first_run = HeroLibrary::in_database(&config).await.unwrap();
            let first = first_run
                .save(&key, &campaign, &[8; 16], &record)
                .await
                .unwrap();
            let loaded = first_run.load(&key, first.hero_id).await.unwrap().reference;
            drop(first_run);
            let mut restarted = HeroLibrary::in_database(&config).await.unwrap();
            let unchanged = restarted
                .save_from(
                    &key,
                    &format!("{campaign}-new"),
                    &[9; 16],
                    &record,
                    Some(&loaded),
                )
                .await
                .unwrap();
            assert_eq!(unchanged, first);
            drop(restarted);
            let mut restarted = HeroLibrary::in_database(&config).await.unwrap();
            record.encoded.extend_from_slice(b" edited");
            let edited = restarted
                .save_from(
                    &key,
                    &format!("{campaign}-edited-first"),
                    &[12; 16],
                    &record,
                    Some(&loaded),
                )
                .await
                .unwrap();
            assert_eq!(edited.hero_id, first.hero_id);
            assert_eq!(edited.revision, first.revision + 1);
            assert_eq!(restarted.revisions(first.hero_id).await.unwrap().len(), 2);
            assert_eq!(
                restarted
                    .load_revision(&key, &first)
                    .await
                    .unwrap()
                    .record
                    .encoded,
                initial_encoded
            );
        });
    }
    #[test]
    fn foreign_loaded_reference_saves_for_this_key_without_changing_other_hero() {
        with_owned_library(|config| async move {
            let owner = PhoneKey::from_bytes(&[82; 32]).unwrap();
            let other = PhoneKey::from_bytes(&[83; 32]).unwrap();
            let campaign = fixture_campaign("foreign");
            let mut db = HeroLibrary::in_database(&config).await.unwrap();
            let mut record = fixture_record("Private hero");
            let original = db
                .save(&owner, &campaign, &[10; 16], &record)
                .await
                .unwrap();
            drop(db);
            let mut db = HeroLibrary::in_database(&config).await.unwrap();
            record.encoded.extend_from_slice(b" edited by other key");
            let saved = db
                .save_from(&other, &campaign, &[11; 16], &record, Some(&original))
                .await
                .unwrap();
            assert_ne!(saved.hero_id, original.hero_id);
            assert_eq!(saved.revision, 1);
            assert_eq!(
                db.load(&other, saved.hero_id).await.unwrap().record.encoded,
                record.encoded
            );
            assert!(matches!(
                db.load(&other, original.hero_id).await,
                Err(LibraryError::NotFound)
            ));
            assert_eq!(
                db.load(&owner, original.hero_id)
                    .await
                    .unwrap()
                    .record
                    .encoded,
                b"Private hero"
            );
            assert_eq!(db.revisions(original.hero_id).await.unwrap().len(), 1);
        });
    }
    fn fixture_record(name: &str) -> IdentityRecord {
        IdentityRecord {
            name: name.into(),
            species: "dwarf".into(),
            character_class: "fighter-1".into(),
            encoded: name.as_bytes().to_vec(),
            portrait: vec![1, 2, 3],
            portrait_thumbnail: vec![1],
            portrait_media_type: "image/jpeg".into(),
        }
    }
    fn fixture_campaign(prefix: &str) -> String {
        format!(
            "{prefix}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    }
    fn with_owned_library<F, Fut>(run: F)
    where
        F: FnOnce(Config) -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let Ok(port) = std::env::var("DUNGEONFLUX_HERO_LIBRARY_PG_PORT") else {
            eprintln!(
                "SKIPPED: DUNGEONFLUX_HERO_LIBRARY_PG_PORT must name an owned scratch PostgreSQL"
            );
            return;
        };
        let port = port.parse::<u16>().expect("scratch port");
        assert!(![0, 55517, 55439].contains(&port));
        let mut config = Config::new();
        config
            .host("127.0.0.1")
            .port(port)
            .user(&std::env::var("DUNGEONFLUX_HERO_LIBRARY_PG_USER").unwrap_or("memtest".into()))
            .dbname(
                &std::env::var("DUNGEONFLUX_HERO_LIBRARY_PG_DATABASE").unwrap_or("postgres".into()),
            );
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(run(config));
    }
    #[test]
    fn hero_library_round_trip_owned_postgres() {
        let Ok(port) = std::env::var("DUNGEONFLUX_HERO_LIBRARY_PG_PORT") else {
            eprintln!(
                "SKIPPED: DUNGEONFLUX_HERO_LIBRARY_PG_PORT must name an owned scratch PostgreSQL"
            );
            return;
        };
        let port: u16 = port.parse().expect("scratch port");
        assert!(![0, 55517, 55439].contains(&port));
        let mut config = Config::new();
        config
            .host("127.0.0.1")
            .port(port)
            .user(&std::env::var("DUNGEONFLUX_HERO_LIBRARY_PG_USER").unwrap_or("memtest".into()))
            .dbname(
                &std::env::var("DUNGEONFLUX_HERO_LIBRARY_PG_DATABASE").unwrap_or("postgres".into()),
            );
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let mut db = HeroLibrary::in_database(&config).await.unwrap();
                let _again = HeroLibrary::in_database(&config).await.unwrap();
                let key = PhoneKey::from_bytes(&[73; 32]).unwrap();
                let wrong = PhoneKey::from_bytes(&[74; 32]).unwrap();
                let campaign = format!(
                    "library-test-{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                );
                let mut record = IdentityRecord {
                    name: "Hero".into(),
                    species: "dwarf".into(),
                    character_class: "fighter-1".into(),
                    encoded: "Exact  \r\n\u{96ea} \u{1f409}".as_bytes().to_vec(),
                    portrait: vec![1, 2, 3],
                    portrait_thumbnail: vec![1, 2, 3],
                    portrait_media_type: "image/jpeg".into(),
                };
                let first = db.save(&key, &campaign, &[3; 16], &record).await.unwrap();
                assert_eq!(
                    first,
                    db.save(&key, &campaign, &[3; 16], &record).await.unwrap()
                );
                let loaded = db.load(&key, first.hero_id).await.unwrap();
                assert_eq!(loaded.record.encoded, record.encoded);
                assert_eq!(loaded.record.portrait, record.portrait);
                assert!(db.list(&wrong).await.unwrap().is_empty());
                assert!(db.load(&wrong, first.hero_id).await.is_err());
                record.portrait.push(4);
                let changed = db.save(&key, &campaign, &[3; 16], &record).await.unwrap();
                assert_eq!(changed.revision, 2);
                assert_eq!(changed.hero_id, first.hero_id);
                assert_eq!(
                    first,
                    db.save(
                        &key,
                        &campaign,
                        &[3; 16],
                        &IdentityRecord {
                            portrait: vec![1, 2, 3],
                            ..record
                        }
                    )
                    .await
                    .unwrap()
                );
                assert_eq!(db.revisions(first.hero_id).await.unwrap().len(), 2);
                assert!(
                    db.client
                        .execute(
                            "UPDATE hero_revisions SET name='mutated' WHERE hero=$1",
                            &[&first.hero_id]
                        )
                        .await
                        .is_err()
                );
                assert!(db.delete(first.hero_id).await.unwrap());
                assert!(db.load(&key, first.hero_id).await.is_err());
            });
    }
}
