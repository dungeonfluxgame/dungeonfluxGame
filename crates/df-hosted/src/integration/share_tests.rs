use super::*;
use df_account_app::wire_share::{CreateShare, ShareConsent, ShareDraft, SharePerson};
use df_portal::share::{LinkToken, ShareFailure};

const SESSION: &str = "09090909090909090909090909090909";
async fn seed_session(f: &Fixture) {
    seed_account(&f.accounts, 2).await;
    f.data.client.execute("INSERT INTO \
                    campaigns.campaign(campaign_id,tenant_id,owner_account_id,name,seat_count,status,feed_epoch,created_at_ms) VALUES($1,$2,$3,'Synthetic campaign',4,'active',$4,0)",&[&[8_u8;16].as_slice(),&[44_u8;16].as_slice(),&[1_u8;16].as_slice(),&[11_u8;16].as_slice()]).await.unwrap();
    for (key, role) in [(1_u8, "host"), (2, "player")] {
        f.data.client.execute("INSERT INTO \
                    campaigns.campaign_member(member_id,campaign_id,account_id,role,holds_seat,status,joined_at_ms) VALUES($1,$2,$3,$4,true,'active',0)",&[&[key;16].as_slice(),&[8_u8;16].as_slice(),&[key;16].as_slice(),&role]).await.unwrap();
    }
    for key in [1_u8,2] {
        f.data.client.execute("INSERT INTO campaigns.lobby_member(session_id,member_id,campaign_id,character_choice,ready,updated_at_ms,frozen_at_ms) VALUES($1,$2,$3,'template',true,0,(extract(epoch FROM clock_timestamp()-interval '2 hours')*1000)::bigint)",&[&[9_u8;16].as_slice(),&[key;16].as_slice(),&[8_u8;16].as_slice()]).await.unwrap();
    }
    f.data.client.execute("INSERT INTO \
                    df_runtime.table_sessions(tenant_id,campaign_id,session_id,phase,created_at,ended_at) \
                    VALUES($1,$2,$3,'ended',clock_timestamp()-interval '2 \
                    hours',clock_timestamp()-interval '1 hour')",&[&[44_u8;16].as_slice(),&[8_u8;16].as_slice(),&[9_u8;16].as_slice()]).await.unwrap();
}
fn draft(include_person: bool) -> ShareDraft {
    ShareDraft {
        title: "A public table summary".into(),
        highlights: vec!["We explored an old tower.".into()],
        people: if include_person {
            vec![SharePerson {
                account_id: "02020202020202020202020202020202".into(),
                public_name: "Public hero".into(),
            }]
        } else {
            Vec::new()
        },
    }
}
fn choices(revision: String, embed: bool, index: bool) -> CreateShare {
    CreateShare {
        revision,
        expiry_days: 7,
        allow_indexing: index,
        allow_embeds: embed,
    }
}
async fn public(app: Router, path: &str) -> (StatusCode, axum::http::HeaderMap, String) {
    let response = app
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, String::from_utf8(bytes.to_vec()).unwrap())
}

#[tokio::test]
async fn share_exact_preview_consent_withdrawal_and_omit_person_control_public_access() {
    let f = Fixture::open().await;
    seed_session(&f).await;
    let player = AccountId::from_bytes(&[2; 16]).unwrap();
    let preview = f
        .shares
        .save_preview(f.caller.account.id, SESSION, draft(true))
        .await
        .unwrap();
    let token = LinkToken::generate().unwrap();
    assert!(matches!(
        f.shares
            .create(
                f.caller.account.id,
                SESSION,
                choices(preview.revision.clone(), false, false),
                token.digest()
            )
            .await,
        Err(ShareFailure::ConsentRequired)
    ));
    assert!(matches!(
        f.shares
            .consent(
                player,
                SESSION,
                ShareConsent {
                    revision: "stale".into(),
                    agrees: true
                }
            )
            .await,
        Err(ShareFailure::Invalid)
    ));
    f.shares
        .consent(
            player,
            SESSION,
            ShareConsent {
                revision: preview.revision.clone(),
                agrees: true,
            },
        )
        .await
        .unwrap();
    f.shares
        .create(
            f.caller.account.id,
            SESSION,
            choices(preview.revision.clone(), false, false),
            token.digest(),
        )
        .await
        .unwrap();
    let path = format!("/s/{}", token.text());
    let (status, _, html) = public(f.router(), &path).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Public hero"));
    assert!(
        !html.contains(
            &player
                .as_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        )
    );
    f.shares
        .consent(
            player,
            SESSION,
            ShareConsent {
                revision: preview.revision,
                agrees: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(public(f.router(), &path).await.0, StatusCode::NOT_FOUND);
    let preview = f
        .shares
        .save_preview(f.caller.account.id, SESSION, draft(false))
        .await
        .unwrap();
    let anonymous = LinkToken::generate().unwrap();
    f.shares
        .create(
            f.caller.account.id,
            SESSION,
            choices(preview.revision, false, false),
            anonymous.digest(),
        )
        .await
        .unwrap();
    let (status, _, html) = public(f.router(), &format!("/s/{}", anonymous.text())).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!html.contains("Public hero"));
    f.finish().await;
}

#[tokio::test]
async fn share_expiry_revocation_and_preview_edit_all_invalidate_old_links() {
    let f = Fixture::open().await;
    seed_session(&f).await;
    let preview = f
        .shares
        .save_preview(f.caller.account.id, SESSION, draft(false))
        .await
        .unwrap();
    let token = LinkToken::generate().unwrap();
    let link = f
        .shares
        .create(
            f.caller.account.id,
            SESSION,
            choices(preview.revision.clone(), false, false),
            token.digest(),
        )
        .await
        .unwrap();
    let path = format!("/s/{}", token.text());
    assert_eq!(public(f.router(), &path).await.0, StatusCode::OK);
    f.shares
        .revoke(f.caller.account.id, SESSION, &link.link_id)
        .await
        .unwrap();
    assert_eq!(public(f.router(), &path).await.0, StatusCode::NOT_FOUND);
    let expired = LinkToken::generate().unwrap();
    f.shares
        .create(
            f.caller.account.id,
            SESSION,
            choices(preview.revision.clone(), false, false),
            expired.digest(),
        )
        .await
        .unwrap();
    f.data.client.execute("UPDATE campaigns.share_link SET created_at_ms=(extract(epoch FROM \
                    clock_timestamp())*1000)::bigint-8*86400000::bigint,expires_at_ms=(extract(epoch FROM \
                    clock_timestamp())*1000)::bigint-86400000 WHERE token_digest=$1",&[&expired.digest().as_slice()]).await.unwrap();
    assert_eq!(
        public(f.router(), &format!("/s/{}", expired.text()))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let old = LinkToken::generate().unwrap();
    f.shares
        .create(
            f.caller.account.id,
            SESSION,
            choices(preview.revision, false, false),
            old.digest(),
        )
        .await
        .unwrap();
    f.shares
        .save_preview(f.caller.account.id, SESSION, draft(false))
        .await
        .unwrap();
    assert_eq!(
        public(f.router(), &format!("/s/{}", old.text())).await.0,
        StatusCode::NOT_FOUND
    );
    f.finish().await;
}

#[tokio::test]
async fn share_embed_headers_and_indexing_require_opt_in_and_html_is_escaped() {
    let f = Fixture::open().await;
    seed_session(&f).await;
    let mut source = draft(false);
    source.title = "<script>private-code</script> & public title".into();
    let preview = f
        .shares
        .save_preview(f.caller.account.id, SESSION, source)
        .await
        .unwrap();
    let disabled = LinkToken::generate().unwrap();
    f.shares
        .create(
            f.caller.account.id,
            SESSION,
            choices(preview.revision.clone(), false, false),
            disabled.digest(),
        )
        .await
        .unwrap();
    let (_, headers, html) = public(f.router(), &format!("/s/{}", disabled.text())).await;
    assert_eq!(headers["x-robots-tag"], "noindex, nofollow, noarchive");
    assert!(
        headers[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .contains("frame-ancestors 'self'")
    );
    assert!(!html.contains("<script>"));
    assert!(html.contains("&lt;script&gt;"));
    assert_eq!(
        public(f.router(), &format!("/embed/s/{}", disabled.text()))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let enabled = LinkToken::generate().unwrap();
    f.shares
        .create(
            f.caller.account.id,
            SESSION,
            choices(preview.revision, true, true),
            enabled.digest(),
        )
        .await
        .unwrap();
    let (status, headers, html) = public(
        f.router(),
        &format!("/embed/s/{}?theme=dark", enabled.text()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .contains("frame-ancestors https:")
    );
    assert_eq!(headers[header::REFERRER_POLICY], "no-referrer");
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert!(!headers.contains_key(header::SET_COOKIE));
    assert!(!html.contains("<script"));
    assert!(html.contains("class=\"dark\""));
    let (_, headers, _) = public(f.router(), &format!("/s/{}", enabled.text())).await;
    assert_eq!(headers["x-robots-tag"], "index, follow");
    f.finish().await;
}

#[tokio::test]
async fn share_publishing_requires_current_host_and_an_ended_session() {
    let f = Fixture::open().await;
    seed_session(&f).await;
    let player = AccountId::from_bytes(&[2; 16]).unwrap();
    assert!(matches!(
        f.shares.save_preview(player, SESSION, draft(false)).await,
        Err(ShareFailure::NotFound)
    ));
    f.data
        .client
        .execute(
            "UPDATE df_runtime.table_sessions SET phase='active',ended_at=NULL WHERE session_id=$1",
            &[&[9_u8; 16].as_slice()],
        )
        .await
        .unwrap();
    assert!(matches!(
        f.shares
            .save_preview(f.caller.account.id, SESSION, draft(false))
            .await,
        Err(ShareFailure::NotFound)
    ));
    f.finish().await;
}

#[tokio::test]
async fn share_cleanup_is_bounded_and_purges_old_public_fields_only_after_the_last_link(){
    let f=Fixture::open().await;seed_session(&f).await;let preview=f.shares.save_preview(f.caller.account.id,SESSION,draft(false)).await.unwrap();
    f.data.client.execute("INSERT INTO campaigns.share_link(link_id,token_digest,session_id,creator_account_id,revision,created_at_ms,expires_at_ms) \
        SELECT uuid_send(gen_random_uuid()),sha256(uuid_send(gen_random_uuid())),$1,$2,$3,0,604800000 \
        FROM generate_series(1,101)",&[&[9_u8;16].as_slice(),&f.caller.account.id.as_bytes().as_slice(),&preview.revision]).await.unwrap();
    f.data.client.execute("UPDATE campaigns.share_preview SET updated_at_ms=0 WHERE session_id=$1",&[&[9_u8;16].as_slice()]).await.unwrap();
    assert_eq!(f.shares.cleanup(30).await.unwrap(),(100,0));
    assert_eq!(f.shares.cleanup(30).await.unwrap(),(1,1));
    assert_eq!(f.shares.cleanup(30).await.unwrap(),(0,0));f.finish().await;
}
