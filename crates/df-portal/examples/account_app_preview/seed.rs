//! The preview's seed data: three verified accounts made through the real account service, and a
//! world for them in the fake ports (credit, a hosted campaign with a lobby, statement lines, an
//! invitation, a dispute, reconciliation differences, providers and review items).

use std::{io, time::Duration};

use df_account_app::wire::{
    AssetHoldState, AssetKind, AssetRow, PlaySessionRow, ProviderHealth, Quantity,
    ReconciliationKind, ReviewFlag, StatementCategory,
};
use df_account_app::wire_campaign::{CampaignRole, InvitationKind};
use df_customer::{
    AccountStore, Client, CustomerService, DevOutbox, DeviceLabel, EmailSender, LoginOutcome,
    SignupForm, SignupOutcome, VerifyWithPasswordOutcome,
};
use df_portal::app_api::{
    fakes::FakePorts,
    ports::{Actor, CampaignDirectory, FinancePort},
};
use df_types::AccountId;

pub const CUSTOMER_EMAIL: &str = "cam@example.test";
pub const OPERATOR_EMAIL: &str = "ops@example.test";
pub const HOST_EMAIL: &str = "robin@example.test";
pub const DEV_CUSTOMER_EMAIL: &str = "dev@example.test";
pub const DEV_OPERATOR_EMAIL: &str = "dev-admin@example.test";

pub struct Seeded {
    /// The once-shown link to Robin's campaign, for a signed-in Cam to open.
    pub invitation_link: String,
    pub room_code: Option<String>,
}

fn failed(what: &str) -> io::Error {
    io::Error::other(format!("seed failed: {what}"))
}

fn client() -> Client {
    Client {
        ip: None,
        device: DeviceLabel::Desktop,
    }
}

/// The verification token mailed to `email`, once the mail worker has written it.
async fn mailed_token(outbox: &DevOutbox, email: &str) -> io::Result<String> {
    for _ in 0..100 {
        for entry in outbox.index().await? {
            let Some(text) = outbox.message(entry.sequence).await? else {
                continue;
            };
            let is_verification = text.contains("X-DungeonFlux-Purpose: verify_email")
                && text.contains(&format!("To: {email}\n"));
            if let (true, Some((_, rest))) = (is_verification, text.split_once("#token=")) {
                return Ok(rest.chars().take_while(char::is_ascii_hexdigit).collect());
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(failed("the verification mail never arrived"))
}

async fn verified_account<S: AccountStore, M: EmailSender>(
    service: &CustomerService<S, M>,
    outbox: &DevOutbox,
    email: &str,
    password: &str,
) -> io::Result<AccountId> {
    let operation = service
        .new_operation()
        .map_err(|_| failed("operation id"))?;
    let signup = service
        .signup(
            SignupForm {
                email,
                password,
                operation,
                adult_attested: true,
                terms_version: df_account_app::wire::CURRENT_TERMS_VERSION.to_owned(),
            },
            client(),
        )
        .await
        .map_err(|_| failed("sign up"))?;
    if signup != SignupOutcome::CheckYourEmail {
        return Err(failed("sign up was refused"));
    }
    let token = mailed_token(outbox, email).await?;
    match service
        .verify_email_and_set_password(&token, password, password, client())
        .await
    {
        Ok(VerifyWithPasswordOutcome::Verified(_)) => {}
        _ => return Err(failed("verification")),
    }
    let LoginOutcome::SignedIn(issued) = service
        .login(email, password, client())
        .await
        .map_err(|_| failed("sign in"))?
    else {
        return Err(failed("sign in was refused"));
    };
    let caller = service
        .authenticate(&issued.token.text())
        .await
        .map_err(|_| failed("session"))?
        .ok_or_else(|| failed("session"))?;
    service
        .logout(&caller, client())
        .await
        .map_err(|_| failed("sign out"))?;
    Ok(caller.account.id)
}

fn actor(account: AccountId, name: &str) -> Actor {
    Actor {
        account,
        display_name: name.to_owned(),
        is_verified: true,
    }
}

fn seed_statement_lines(ports: &FakePorts, cam: AccountId) {
    let finance = &ports.finance;
    for index in 0..14_u64 {
        finance.seed_charge(
            cam,
            StatementCategory::TableTime,
            "The Lantern Road",
            Quantity::Seconds {
                seconds: 1_800 + index * 60,
            },
            12_000 + index * 500,
        );
    }
    finance.seed_charge(
        cam,
        StatementCategory::Narration,
        "The Lantern Road",
        Quantity::AudioTenthsOfMinute { tenths: 420 },
        180_000,
    );
    finance.seed_charge(
        cam,
        StatementCategory::SceneImage,
        "The Lantern Road",
        Quantity::Images { count: 6 },
        240_000,
    );
    finance.seed_charge(
        cam,
        StatementCategory::Portrait,
        "Moonfall Keep",
        Quantity::Images { count: 2 },
        90_000,
    );
    finance.seed_charge(
        cam,
        StatementCategory::Music,
        "The Lantern Road",
        Quantity::Seconds { seconds: 600 },
        45_000,
    );
}

const DAY_MS: u64 = 24 * 60 * 60 * 1000;

fn hex_id(bytes: &[u8; 16]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Dated funding history (three months, two campaigns, one refund), play sessions and
/// generated assets of every kind for one account.
pub fn seed_history(ports: &FakePorts, account: AccountId, lantern: &str, moonfall: &str) {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64);
    let finance = &ports.finance;
    finance.seed_credit_at(
        account,
        now_ms - 70 * DAY_MS,
        StatementCategory::Purchase,
        20_000_000,
    );
    finance.seed_credit_at(
        account,
        now_ms - 40 * DAY_MS,
        StatementCategory::Purchase,
        10_000_000,
    );
    finance.seed_credit_at(
        account,
        now_ms - 5 * DAY_MS,
        StatementCategory::Purchase,
        25_000_000,
    );
    finance.seed_credit_at(
        account,
        now_ms - 60 * DAY_MS,
        StatementCategory::GoodwillCredit,
        5_000_000,
    );
    finance.seed_credit_at(
        account,
        now_ms - 10 * DAY_MS,
        StatementCategory::Refund,
        2_000_000,
    );
    finance.seed_charge_at(
        account,
        now_ms - 65 * DAY_MS,
        StatementCategory::TableTime,
        "The Lantern Road",
        Quantity::Seconds { seconds: 7_200 },
        3_600_000,
    );
    finance.seed_charge_at(
        account,
        now_ms - 35 * DAY_MS,
        StatementCategory::Narration,
        "Moonfall Keep",
        Quantity::AudioTenthsOfMinute { tenths: 300 },
        2_400_000,
    );
    finance.seed_charge_at(
        account,
        now_ms - 3 * DAY_MS,
        StatementCategory::SceneImage,
        "The Lantern Road",
        Quantity::Images { count: 4 },
        1_800_000,
    );
    for (index, days_ago) in [70_u64, 45, 20, 6, 1].into_iter().enumerate() {
        let started = now_ms - days_ago * DAY_MS;
        let campaign = if index % 2 == 0 { lantern } else { moonfall };
        let name = if index % 2 == 0 {
            "The Lantern Road"
        } else {
            "Moonfall Keep"
        };
        ports.campaigns.seed_play_session(
            account,
            PlaySessionRow {
                session_id: format!("{index:032x}"),
                campaign_id: campaign.to_owned(),
                campaign_name: name.to_owned(),
                started_at_ms: started,
                ended_at_ms: Some(started + 7_200_000),
                duration_seconds: 7_200,
                players: 4,
                spent_micro_dollars: 2_000_000 + index as u64 * 250_000,
            },
        );
    }
    let kinds = [
        (AssetKind::Portrait, "portrait", 240_000_u64),
        (AssetKind::Scene, "scene", 1_800_000),
        (AssetKind::Music, "music", 3_200_000),
        (AssetKind::Video, "video", 18_000_000),
    ];
    for (index, (kind, label, size)) in kinds.into_iter().enumerate() {
        ports.assets.seed_asset(
            account,
            AssetRow {
                asset_id: format!("asset-{label}-1"),
                kind,
                campaign_id: Some(lantern.to_owned()),
                campaign_name: Some("The Lantern Road".to_owned()),
                created_at_ms: now_ms - (index as u64 + 1) * DAY_MS,
                size_bytes: size,
                thumbnail_url: Some(format!("/app/media/thumb/asset-{label}-1")),
                hold_state: AssetHoldState::Ready,
            },
        );
    }
    ports.assets.seed_asset(
        account,
        AssetRow {
            asset_id: "asset-scene-held".to_owned(),
            kind: AssetKind::Scene,
            campaign_id: Some(moonfall.to_owned()),
            campaign_name: Some("Moonfall Keep".to_owned()),
            created_at_ms: now_ms - 2 * DAY_MS,
            size_bytes: 1_500_000,
            thumbnail_url: Some("/app/media/thumb/asset-scene-held".to_owned()),
            hold_state: AssetHoldState::Held,
        },
    );
}

async fn seed_world(ports: &FakePorts, ids: [AccountId; 3]) -> io::Result<Seeded> {
    let [cam, ops, robin] = ids;
    ports
        .finance
        .overview(cam, true)
        .await
        .map_err(|_| failed("trial"))?;
    ports.finance.credit_purchase(cam, 2_000, "seed-purchase");
    seed_statement_lines(ports, cam);
    ports.operators.grant_all(ops, true);

    let hosting = actor(cam, "Cam");
    let lantern = ports.campaigns.seed_campaign(&hosting, "The Lantern Road");
    ports
        .campaigns
        .seed_member(lantern, &actor(robin, "Robin"), CampaignRole::Player);
    ports
        .campaigns
        .seed_lobby(lantern)
        .ok_or_else(|| failed("lobby"))?;
    ports
        .campaigns
        .rotate_code(&hosting, lantern)
        .await
        .map_err(|_| failed("room code"))?;
    let room_code = ports.campaigns.room_code(lantern);
    ports
        .campaigns
        .seed_join_request(lantern, &actor(ids_for_guest(), "Ivy"));

    let moonfall = ports
        .campaigns
        .seed_campaign(&actor(robin, "Robin"), "Moonfall Keep");
    let invitation_link = ports
        .campaigns
        .seed_invitation(moonfall, InvitationKind::Player)
        .ok_or_else(|| failed("invitation"))?;

    ports.finance.seed_dispute(cam, 1_500);
    ports.finance.seed_reconciliation_difference(
        ReconciliationKind::ProcessorCapture,
        "ch_test_0001",
        20_000,
    );
    ports.finance.seed_reconciliation_difference(
        ReconciliationKind::AgedReservation,
        "res_test_0042",
        5_000,
    );
    ports.finance.seed_provider(
        "voice-a",
        "Aurora Speech",
        ProviderHealth::Healthy,
        4,
        380_000,
    );
    ports.finance.seed_provider(
        "image-a",
        "Lumen Images",
        ProviderHealth::Degraded,
        61,
        1_250_000,
    );
    seed_history(
        ports,
        cam,
        &hex_id(lantern.as_bytes()),
        &hex_id(moonfall.as_bytes()),
    );
    ports
        .finance
        .seed_provider("music-a", "Tessera Music", ProviderHealth::Paused, 0, 0);
    ports.review.seed_item(
        "portrait-0001",
        "The Lantern Road",
        ReviewFlag::PossibleRealPerson,
    );
    ports.review.seed_item(
        "portrait-0002",
        "Moonfall Keep",
        ReviewFlag::ScreenUnavailable,
    );
    Ok(Seeded {
        invitation_link,
        room_code,
    })
}

/// A stand-in account for a person who is not signed up (a pending room-code request).
fn ids_for_guest() -> AccountId {
    AccountId::from_bytes(&[7; 16]).unwrap_or_else(|_| unreachable!("sixteen nonzero bytes"))
}

pub async fn seed<S: AccountStore, M: EmailSender>(
    service: &CustomerService<S, M>,
    outbox: &DevOutbox,
    ports: &FakePorts,
    password: &str,
) -> io::Result<Seeded> {
    let cam = verified_account(service, outbox, CUSTOMER_EMAIL, password).await?;
    let ops = verified_account(service, outbox, OPERATOR_EMAIL, password).await?;
    let robin = verified_account(service, outbox, HOST_EMAIL, password).await?;
    seed_world(ports, [cam, ops, robin]).await
}

pub async fn seed_dev_accounts<S: AccountStore, M: EmailSender>(
    service: &CustomerService<S, M>,
    outbox: &DevOutbox,
    ports: &FakePorts,
    password: &str,
) -> io::Result<()> {
    let customer = verified_account(service, outbox, DEV_CUSTOMER_EMAIL, password).await?;
    let operator = verified_account(service, outbox, DEV_OPERATOR_EMAIL, password).await?;
    ports.operators.grant_all(operator, true);
    ports
        .finance
        .overview(customer, true)
        .await
        .map_err(|_| failed("trial"))?;
    ports
        .finance
        .credit_purchase(customer, 2_500, "dev-purchase");
    let hosting = actor(customer, "Dev");
    let lantern = ports.campaigns.seed_campaign(&hosting, "The Lantern Road");
    let moonfall = ports
        .campaigns
        .seed_campaign(&actor(operator, "Admin"), "Moonfall Keep");
    seed_history(
        ports,
        customer,
        &hex_id(lantern.as_bytes()),
        &hex_id(moonfall.as_bytes()),
    );
    Ok(())
}
