//! Actual canonical revoke/restore races distinguish locked negatives from missing evidence.
use super::*;
use imagery_identity::{AuthorityEvidence, HostedAuthorityCheck};
fn hosted_check() -> HostedAuthorityCheck {
    HostedAuthorityCheck {
        tenant: id(0x51),
        campaign: id(0x52),
        hosted_session: id(0x53),
        member: id(0x62),
        generation: id(1),
        web_session: id(0x81),
        gameplay_credential: [0x91; 32],
    }
}
async fn row_locked(contender: &mut Client, table: &str) {
    let other = contender.transaction().await.unwrap();
    let error = other
        .query(&format!("SELECT 1 FROM {table} FOR UPDATE NOWAIT"), &[])
        .await
        .unwrap_err();
    assert_eq!(error.as_db_error().unwrap().code().code(), "55P03");
    other.rollback().await.unwrap();
}
async fn wait_for_row_lock(observer: &Client, pid: i32) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if observer.query_one("SELECT coalesce((SELECT wait_event_type='Lock' FROM pg_stat_activity WHERE pid=$1),false)",&[&pid]).await.unwrap().get::<_,bool>(0) {break;}
            tokio::task::yield_now().await;
        }
    }).await.expect("actual authority query reached row-lock wait");
}
async fn null_sql_arguments_are_unproven(client: &Client) {
    let run = run_binding();
    for (function, values) in [
        (
            "lock_run_authority_evidence",
            vec![
                Some(run.tenant.to_vec()),
                Some(run.campaign.to_vec()),
                Some(run.hosted_session.to_vec()),
                Some(run.gameplay_tenant.to_vec()),
                Some(run.gameplay_session.to_vec()),
                Some(run.run.to_vec()),
                Some(run.fence.to_vec()),
            ],
        ),
        (
            "lock_hosted_authority_evidence",
            vec![
                Some(id(0x51).to_vec()),
                Some(id(0x52).to_vec()),
                Some(id(0x53).to_vec()),
                Some(id(0x62).to_vec()),
                Some(id(1).to_vec()),
                Some(id(0x81).to_vec()),
                Some(vec![0x91; 32]),
            ],
        ),
    ] {
        let sql = format!(
            "SELECT imagery.{function}($1::bytea,$2::bytea,$3::bytea,$4::bytea,$5::bytea,$6::bytea,$7::bytea)"
        );
        for index in 0..7 {
            for bad in [None, Some(vec![1; 15])] {
                let mut arguments = values.clone();
                arguments[index] = bad;
                assert_eq!(
                    client
                        .query_one(
                            &sql,
                            &[
                                &arguments[0],
                                &arguments[1],
                                &arguments[2],
                                &arguments[3],
                                &arguments[4],
                                &arguments[5],
                                &arguments[6]
                            ]
                        )
                        .await
                        .unwrap()
                        .get::<_, i16>(0),
                    0
                );
            }
        }
    }
}
#[test]
fn hosted_evidence_uses_read_lock_definer_without_caller_mutation_and_serializes_restore() {
    runtime().block_on(async {
        let Some(scratch)=Scratch::open().await else {return};
        bound(&scratch).await;
        let admin=scratch.connect().await;
        let mut authority=scratch.connect().await;
        let mut contender=scratch.connect().await;
        authority.batch_execute("SET ROLE df_imagery_admission").await.unwrap();
        null_sql_arguments_are_unproven(&authority).await;
        for table in ["accounts.account","accounts.web_session","campaigns.campaign_member",
            "imagery.image_host_run_binding","imagery.image_host_seat_binding","df_local_demo.grants"] {
            assert!(!admin.query_one("SELECT has_any_column_privilege('df_imagery_admission',$1,'UPDATE')",&[&table]).await.unwrap().get::<_,bool>(0));
        }
        let denial=authority.execute("UPDATE accounts.web_session SET revoked_at_ms=1,revoke_reason='logout'",&[]).await.unwrap_err();
        assert_eq!(denial.as_db_error().unwrap().code().code(),"42501");
        let tx=authority.transaction().await.unwrap();
        assert_eq!(imagery_identity::lock_hosted_authority_evidence(&tx,&hosted_check()).await.unwrap(),AuthorityEvidence::Current);
        for table in ["accounts.account","accounts.web_session","campaigns.campaign_member","df_game.sessions","df_runtime.table_sessions","df_local_demo.grants","imagery.image_host_run_binding","imagery.image_host_seat_binding"] {
            row_locked(&mut contender,table).await;
        }
        tx.commit().await.unwrap();
        admin.execute("UPDATE accounts.web_session SET revoked_at_ms=1,revoke_reason='logout'",&[]).await.unwrap();
        let tx=authority.transaction().await.unwrap();
        assert_eq!(imagery_identity::lock_hosted_authority_evidence(&tx,&hosted_check()).await.unwrap(),AuthorityEvidence::Refused);
        row_locked(&mut contender,"accounts.web_session").await;
        tx.commit().await.unwrap();
        admin.execute("UPDATE accounts.web_session SET revoked_at_ms=NULL,revoke_reason=NULL",&[]).await.unwrap();
        let tx=authority.transaction().await.unwrap();
        assert_eq!(imagery_identity::lock_hosted_authority_evidence(&tx,&hosted_check()).await.unwrap(),AuthorityEvidence::Current);
        tx.commit().await.unwrap();
        let before: String=admin.query_one("SELECT jsonb_build_object('seat',(SELECT jsonb_agg(xmin::text) FROM imagery.image_host_seat_binding),'run',(SELECT jsonb_agg(xmin::text) FROM imagery.image_host_run_binding),'web',(SELECT jsonb_agg(xmin::text) FROM accounts.web_session),'account',(SELECT jsonb_agg(xmin::text) FROM accounts.account),'campaign',(SELECT jsonb_agg(xmin::text) FROM campaigns.campaign),'grant',(SELECT jsonb_agg(xmin::text) FROM df_local_demo.grants))::text",&[]).await.unwrap().get(0);
        for missing in 0..3 {
            let mut check=hosted_check();
            match missing {0=>check.web_session=id(0xfe),1=>check.member=id(0xfd),_=>check.gameplay_credential=[0xfc;32]};
            let tx=authority.transaction().await.unwrap();
            assert_eq!(imagery_identity::lock_hosted_authority_evidence(&tx,&check).await.unwrap(),AuthorityEvidence::Unproven);
            tx.commit().await.unwrap();
        }
        let after: String=admin.query_one("SELECT jsonb_build_object('seat',(SELECT jsonb_agg(xmin::text) FROM imagery.image_host_seat_binding),'run',(SELECT jsonb_agg(xmin::text) FROM imagery.image_host_run_binding),'web',(SELECT jsonb_agg(xmin::text) FROM accounts.web_session),'account',(SELECT jsonb_agg(xmin::text) FROM accounts.account),'campaign',(SELECT jsonb_agg(xmin::text) FROM campaigns.campaign),'grant',(SELECT jsonb_agg(xmin::text) FROM df_local_demo.grants))::text",&[]).await.unwrap().get(0);
        assert_eq!(after,before,"missing evidence cannot mutate or create canonical authority");
        scratch.finish().await;
    });
}
#[test]
fn local_run_existing_negative_locks_restore_while_missing_rows_remain_unproven() {
    runtime().block_on(async {
        let Some(scratch) = Scratch::open_local().await else {
            return;
        };
        seed_local_run(&scratch).await;
        let admin = scratch.connect().await;
        let mut authority = scratch.connect().await;
        let mut contender = scratch.connect().await;
        let tx = authority.transaction().await.unwrap();
        assert_eq!(
            imagery_identity::lock_local_run_authority_evidence(&tx, &local_run_grant())
                .await
                .unwrap(),
            AuthorityEvidence::Current
        );
        row_locked(&mut contender, "df_game.sessions").await;
        row_locked(&mut contender, "df_local_demo.grants").await;
        tx.commit().await.unwrap();
        admin
            .execute(
                "UPDATE df_game.sessions SET lease_until=clock_timestamp()-interval '1 second'",
                &[],
            )
            .await
            .unwrap();
        admin
            .execute(
                "UPDATE df_local_demo.grants SET active=false WHERE credential=$1",
                &[&[0x94u8; 32].as_slice()],
            )
            .await
            .unwrap();
        let tx = authority.transaction().await.unwrap();
        assert_eq!(
            imagery_identity::lock_local_run_authority_evidence(&tx, &local_run_grant())
                .await
                .unwrap(),
            AuthorityEvidence::Refused
        );
        row_locked(&mut contender, "df_game.sessions").await;
        row_locked(&mut contender, "df_local_demo.grants").await;
        tx.commit().await.unwrap();
        admin
            .execute(
                "UPDATE df_game.sessions SET lease_until=clock_timestamp()+interval '1 hour'",
                &[],
            )
            .await
            .unwrap();
        admin
            .execute(
                "UPDATE df_local_demo.grants SET active=true WHERE credential=$1",
                &[&[0x94u8; 32].as_slice()],
            )
            .await
            .unwrap();
        let tx = authority.transaction().await.unwrap();
        assert_eq!(
            imagery_identity::lock_local_run_authority_evidence(&tx, &local_run_grant())
                .await
                .unwrap(),
            AuthorityEvidence::Current
        );
        tx.commit().await.unwrap();
        for missing in 0..2 {
            let mut grant = local_run_grant();
            if missing == 0 {
                grant.session = id(0xfe)
            } else {
                grant.bootstrap_credential = [0xfd; 32]
            };
            let tx = authority.transaction().await.unwrap();
            assert_eq!(
                imagery_identity::lock_local_run_authority_evidence(&tx, &grant)
                    .await
                    .unwrap(),
                AuthorityEvidence::Unproven
            );
            tx.commit().await.unwrap();
        }
        scratch.finish().await;
    });
}
#[test]
fn local_guest_negative_locks_actual_credential_without_fabricating_missing_mapping() {
    runtime().block_on(async {
        let Some(scratch) = Scratch::open_local().await else {
            return;
        };
        seed_local_run(&scratch).await;
        let mut authority = scratch.connect().await;
        let mut contender = scratch.connect().await;
        let command = local_command(0x63, 0x91, 0x6a, 0x20, 0x21);
        let tx = authority.transaction().await.unwrap();
        imagery_identity::bootstrap_local_guest(&tx, &command, NOW_MS)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        authority
            .execute(
                "UPDATE df_local_demo.grants SET active=false WHERE credential=$1",
                &[&[0x91u8; 32].as_slice()],
            )
            .await
            .unwrap();
        let tx = authority.transaction().await.unwrap();
        assert_eq!(
            imagery_identity::lock_local_guest_authority_evidence(
                &tx,
                &id(0x63),
                &id(0x54),
                &[0x91; 32]
            )
            .await
            .unwrap(),
            AuthorityEvidence::Refused
        );
        row_locked(&mut contender, "df_local_demo.grants").await;
        row_locked(&mut contender, "df_game.sessions").await;
        tx.commit().await.unwrap();
        authority
            .execute(
                "UPDATE df_local_demo.grants SET active=true WHERE credential=$1",
                &[&[0x91u8; 32].as_slice()],
            )
            .await
            .unwrap();
        let tx = authority.transaction().await.unwrap();
        assert_eq!(
            imagery_identity::lock_local_guest_authority_evidence(
                &tx,
                &id(0x63),
                &id(0x54),
                &[0x91; 32]
            )
            .await
            .unwrap(),
            AuthorityEvidence::Current
        );
        assert_eq!(
            imagery_identity::lock_local_guest_authority_evidence(
                &tx,
                &id(0xfe),
                &id(0x54),
                &[0x91; 32]
            )
            .await
            .unwrap(),
            AuthorityEvidence::Unproven
        );
        tx.commit().await.unwrap();
        assert_eq!(
            authority
                .query_one("SELECT count(*) FROM imagery.image_local_guest", &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            1
        );
        scratch.finish().await;
    });
}

#[test]
fn hosted_evidence_waits_on_account_before_archived_campaign_writer_lock() {
    runtime().block_on(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        bound(&scratch).await;
        let observer = scratch.connect().await;
        observer
            .execute(
                "UPDATE campaigns.campaign SET status='archived',archived_at_ms=1",
                &[],
            )
            .await
            .unwrap();
        let mut writer = scratch.connect().await;
        let mut authority = scratch.connect().await;
        authority
            .batch_execute("SET ROLE df_imagery_admission")
            .await
            .unwrap();
        let pid: i32 = authority
            .query_one("SELECT pg_backend_pid()", &[])
            .await
            .unwrap()
            .get(0);
        let closing = writer.transaction().await.unwrap();
        closing
            .query("SELECT 1 FROM accounts.account FOR UPDATE", &[])
            .await
            .unwrap();
        let pending = tokio::spawn(async move {
            let tx = authority.transaction().await.unwrap();
            let evidence = imagery_identity::lock_hosted_authority_evidence(&tx, &hosted_check())
                .await
                .unwrap();
            tx.commit().await.unwrap();
            evidence
        });
        wait_for_row_lock(&observer, pid).await;
        closing
            .query("SELECT 1 FROM campaigns.campaign FOR UPDATE NOWAIT", &[])
            .await
            .unwrap();
        closing
            .execute(
                "UPDATE campaigns.campaign SET scheduled_delete_at_ms=1 WHERE status='archived'",
                &[],
            )
            .await
            .unwrap();
        closing.commit().await.unwrap();
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(5), pending)
                .await
                .unwrap()
                .unwrap(),
            AuthorityEvidence::Refused
        );
        scratch.finish().await;
    });
}
#[test]
fn local_guest_evidence_waits_on_session_before_startup_reissues_grants() {
    runtime().block_on(async {
        let Some(scratch)=Scratch::open_local().await else {return};
        seed_local_run(&scratch).await;
        let observer=scratch.connect().await;
        let mut writer=scratch.connect().await;
        let mut authority=scratch.connect().await;
        let tx=authority.transaction().await.unwrap();
        imagery_identity::bootstrap_local_guest(&tx,&local_command(0x63,0x91,0x6a,0x20,0x21),NOW_MS).await.unwrap();
        tx.commit().await.unwrap();
        let pid:i32=authority.query_one("SELECT pg_backend_pid()",&[]).await.unwrap().get(0);
        let restoring=writer.transaction().await.unwrap();
        restoring.query("SELECT 1 FROM df_game.sessions FOR UPDATE",&[]).await.unwrap();
        let pending=tokio::spawn(async move {
            let tx=authority.transaction().await.unwrap();
            let evidence=imagery_identity::lock_local_guest_authority_evidence(&tx,&id(0x63),&id(0x54),&[0x91;32]).await.unwrap();
            tx.commit().await.unwrap(); evidence
        });
        wait_for_row_lock(&observer,pid).await;
        restoring.query("SELECT 1 FROM df_local_demo.grants FOR UPDATE NOWAIT",&[]).await.unwrap();
        restoring.execute("UPDATE df_local_demo.grants SET expires_at=clock_timestamp()+interval '1 hour' WHERE session_id=$1 AND active",&[&id(0x54).as_slice()]).await.unwrap();
        restoring.commit().await.unwrap();
        assert_eq!(tokio::time::timeout(std::time::Duration::from_secs(5),pending).await.unwrap().unwrap(),AuthorityEvidence::Current);
        scratch.finish().await;
    });
}
