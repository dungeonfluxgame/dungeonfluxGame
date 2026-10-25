#![cfg(not(target_arch = "wasm32"))]

mod support;
use support::*;

#[test]
fn the_migrations_apply_to_a_scratch_database() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let version: String = client
            .query_one("SHOW server_version", &[])
            .await
            .unwrap()
            .get(0);
        eprintln!("postgres {version}");
        let accounts: i64 = client
            .query_one("SELECT count(*) FROM finance.ledger_account", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(accounts, 30);
        scratch.finish().await;
    });
}
