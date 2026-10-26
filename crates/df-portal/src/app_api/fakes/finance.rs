//! Fake finance (TODO(F): replaced by the finance service's customer ledger reads). Keeps the
//! five balances per account, funding lots, session pots, statement lines and the operator
//! journal in memory. Balances are unsigned and every debit is checked, so none goes below zero.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use df_account_app::{
    api::ProviderSwitch,
    wire::{
        AccountStats, ActionOutcome, BalanceView, CampaignFunding, CampaignSpend, ClosureBlocker,
        Direction, DisputeRow, FundingView, FundsState, LedgerEntry, LedgerKind, LineStatus,
        MonthlySummary, OverviewView, PacePreset, PresetOffer, ProviderRow, Quantity,
        ReconciliationView, RefundableLot, Refusal, SessionFunding, StatementCategory,
        StatementFilter, StatementLine, StatementTotals, StatementsView, TrialView,
    },
};
use df_customer::Clock;
use df_types::{AccountId, CampaignId, OperationId, SessionId};

use super::{Receipts, lock};
use crate::app_api::ports::{
    FinancePort, FinanceSummary, FundingTarget, OperatorAct, PortError, PortResult,
};

pub(super) const MICRO_DOLLARS_PER_CENT: u64 = 10_000;
/// The promotional trial every newly verified account receives (decision 3).
pub const TRIAL_MICRO_DOLLARS: u64 = 10_000_000;
const STATEMENT_PAGE_LINES: usize = 20;
const CUSTOM_MINIMUM_CENTS_PER_HOUR: u32 = 100;
const CUSTOM_MAXIMUM_CENTS_PER_HOUR: u32 = 5_000;

const fn preset_ceiling(preset: PacePreset) -> Option<u32> {
    match preset {
        PacePreset::Story => Some(300),
        PacePreset::Immersive => Some(800),
        PacePreset::Cinematic => Some(2_000),
        PacePreset::Custom => None,
    }
}

#[derive(Default)]
pub(super) struct Wallet {
    pub purchased_micro: u64,
    pub promotional_micro: u64,
    pub allocated_micro: u64,
    pub reserved_micro: u64,
    pub consumed_this_month_micro: u64,
    pub trial_granted_micro: Option<u64>,
    pub lots: Vec<RefundableLot>,
    /// Newest first.
    pub lines: Vec<StatementLine>,
    /// Newest first.
    pub ledger: Vec<LedgerEntry>,
}

pub(super) struct Pot {
    pub pot_micro: u64,
    pub holds_micro: u64,
    pub spent_micro: u64,
    pub allocations: HashMap<AccountId, u64>,
}

#[derive(Clone, Copy)]
pub(super) struct Pace {
    preset: PacePreset,
    custom_ceiling_cents_per_hour: Option<u32>,
}

pub(super) struct State {
    cancelled_host_sessions: HashSet<SessionId>,
    pub host_sessions:
        HashMap<SessionId, (AccountId, df_account_app::wire_campaign::HostCreditView)>,
    pub wallets: HashMap<AccountId, Wallet>,
    pub paces: HashMap<CampaignId, Pace>,
    pub pots: HashMap<SessionId, Pot>,
    pub paid_checkouts: HashSet<String>,
    pub receipts: Receipts<ActionOutcome>,
    pub disputes: Vec<DisputeRow>,
    pub providers: Vec<ProviderRow>,
    pub reconciliation: ReconciliationView,
    pub sequence: u64,
}

/// Cloning shares one state, so the checkout fake's webhook credits the same wallets.
#[derive(Clone)]
pub struct FakeFinance {
    pub(super) clock: Arc<dyn Clock>,
    pub(super) state: Arc<Mutex<State>>,
}

impl State {
    pub(super) fn next_id(&mut self, prefix: &str) -> String {
        self.sequence += 1;
        format!("{prefix}-{}", self.sequence)
    }

    pub(super) fn wallet(&mut self, account: AccountId) -> &mut Wallet {
        self.wallets.entry(account).or_default()
    }
}

impl FakeFinance {
    /// Installs a customer-safe host quote in a local fixture, without moving credit.
    pub fn seed_host_credit(
        &self,
        host: AccountId,
        session: SessionId,
        quote: df_account_app::wire_campaign::HostCreditView,
    ) {
        lock(&self.state)
            .host_sessions
            .insert(session, (host, quote));
    }
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            state: Arc::new(Mutex::new(State {
                cancelled_host_sessions: HashSet::new(),
                host_sessions: HashMap::new(),
                wallets: HashMap::new(),
                paces: HashMap::new(),
                pots: HashMap::new(),
                paid_checkouts: HashSet::new(),
                receipts: Receipts::default(),
                disputes: Vec::new(),
                providers: Vec::new(),
                reconciliation: ReconciliationView {
                    last_run_at_ms: None,
                    differences: Vec::new(),
                },
                sequence: 0,
            })),
        }
    }

    pub(super) fn now_ms(&self) -> u64 {
        self.clock.now_ms()
    }

    /// Posts a statement line and, for the operator, its journal entry; `entry` is the journal
    /// kind and breakdown when the line has one.
    pub(super) fn post_line(
        &self,
        state: &mut State,
        account: AccountId,
        mut line: StatementLine,
        entry: Option<LedgerEntry>,
    ) {
        line.line_id = state.next_id("line");
        let entry = entry.map(|mut entry| {
            entry.entry_id = state.next_id("entry");
            entry
        });
        let wallet = state.wallet(account);
        wallet.lines.insert(0, line);
        if let Some(entry) = entry {
            wallet.ledger.insert(0, entry);
        }
    }

    pub(super) fn entry_for(&self, kind: LedgerKind, retail_micro_dollars: u64) -> LedgerEntry {
        LedgerEntry {
            entry_id: String::new(),
            at_ms: self.now_ms(),
            kind,
            campaign_name: None,
            retail_micro_dollars,
            table_fee_micro_dollars: 0,
            multiplier_permille: 1_000,
            provider: None,
            provider_cost_micro_dollars: 0,
            price_version: "pilot-1".to_owned(),
            status: LineStatus::Settled,
        }
    }

    pub(super) fn line_for(
        &self,
        category: StatementCategory,
        direction: Direction,
        amount_micro_dollars: u64,
    ) -> StatementLine {
        StatementLine {
            line_id: String::new(),
            at_ms: self.now_ms(),
            category,
            campaign_name: None,
            quantity: Quantity::None,
            direction,
            amount_micro_dollars,
            status: LineStatus::Settled,
        }
    }

    /// Credits a paid checkout once: a lot, purchased credit, a statement line and a journal
    /// entry. A repeated webhook for the same checkout changes nothing.
    pub fn credit_purchase(&self, account: AccountId, cents: u32, checkout_ref: &str) {
        let mut state = lock(&self.state);
        if !state.paid_checkouts.insert(checkout_ref.to_owned()) {
            return;
        }
        let micro = u64::from(cents) * MICRO_DOLLARS_PER_CENT;
        let lot_id = state.next_id("lot");
        let now_ms = self.now_ms();
        let wallet = state.wallet(account);
        wallet.purchased_micro += micro;
        wallet.lots.push(RefundableLot {
            lot_id,
            purchased_at_ms: now_ms,
            refundable_cents: cents,
            card_suffix: "4242".to_owned(),
        });
        let line = self.line_for(StatementCategory::Purchase, Direction::Credit, micro);
        let entry = self.entry_for(LedgerKind::Purchase, micro);
        self.post_line(&mut state, account, line, Some(entry));
    }

    /// Seeds a settled charge for a table, with the operator's full breakdown behind it.
    pub fn seed_charge(
        &self,
        account: AccountId,
        category: StatementCategory,
        campaign_name: &str,
        quantity: Quantity,
        retail_micro_dollars: u64,
    ) {
        let mut state = lock(&self.state);
        let wallet = state.wallet(account);
        let from_promotional = retail_micro_dollars.min(wallet.promotional_micro);
        wallet.promotional_micro -= from_promotional;
        wallet.purchased_micro = wallet
            .purchased_micro
            .saturating_sub(retail_micro_dollars - from_promotional);
        wallet.consumed_this_month_micro += retail_micro_dollars;
        let mut line = self.line_for(category, Direction::Charge, retail_micro_dollars);
        line.campaign_name = Some(campaign_name.to_owned());
        line.quantity = quantity;
        let mut entry = self.entry_for(LedgerKind::Usage, retail_micro_dollars);
        entry.campaign_name = Some(campaign_name.to_owned());
        entry.table_fee_micro_dollars = retail_micro_dollars / 5;
        entry.multiplier_permille = 1_500;
        entry.provider = Some("Aurora Speech".to_owned());
        entry.provider_cost_micro_dollars = retail_micro_dollars / 3;
        self.post_line(&mut state, account, line, Some(entry));
    }

    fn grant_trial_once(&self, state: &mut State, account: AccountId) {
        if state.wallet(account).trial_granted_micro.is_some() {
            return;
        }
        let wallet = state.wallet(account);
        wallet.trial_granted_micro = Some(TRIAL_MICRO_DOLLARS);
        wallet.promotional_micro += TRIAL_MICRO_DOLLARS;
        let line = self.line_for(
            StatementCategory::TrialCredit,
            Direction::Credit,
            TRIAL_MICRO_DOLLARS,
        );
        let entry = self.entry_for(LedgerKind::TrialGrant, TRIAL_MICRO_DOLLARS);
        self.post_line(state, account, line, Some(entry));
    }

    fn overview_now(&self, account: AccountId, is_verified: bool) -> OverviewView {
        let mut state = lock(&self.state);
        if is_verified {
            self.grant_trial_once(&mut state, account);
        }
        let wallet = state.wallet(account);
        let trial = match wallet.trial_granted_micro {
            None => TrialView::AwaitingVerification {
                amount_micro_dollars: TRIAL_MICRO_DOLLARS,
            },
            Some(granted) if wallet.promotional_micro == 0 => TrialView::Spent {
                granted_micro_dollars: granted,
            },
            Some(granted) => TrialView::Active {
                granted_micro_dollars: granted,
                remaining_micro_dollars: wallet.promotional_micro,
            },
        };
        OverviewView {
            balance: BalanceView {
                purchased_available_micro_dollars: wallet.purchased_micro,
                promotional_available_micro_dollars: wallet.promotional_micro,
                allocated_micro_dollars: wallet.allocated_micro,
                reserved_micro_dollars: wallet.reserved_micro,
                consumed_this_month_micro_dollars: wallet.consumed_this_month_micro,
            },
            trial,
        }
    }

    fn pace_of(state: &State, campaign: CampaignId) -> (PacePreset, u32) {
        let pace = state.paces.get(&campaign).copied().unwrap_or(Pace {
            preset: PacePreset::Story,
            custom_ceiling_cents_per_hour: None,
        });
        let ceiling = pace
            .custom_ceiling_cents_per_hour
            .or_else(|| preset_ceiling(pace.preset))
            .unwrap_or(CUSTOM_MINIMUM_CENTS_PER_HOUR);
        (pace.preset, ceiling)
    }

    fn session_funding(
        state: &State,
        account: AccountId,
        session: SessionId,
        ceiling_cents_per_hour: u32,
    ) -> SessionFunding {
        let (pot, holds, spent, own) = state.pots.get(&session).map_or((0, 0, 0, 0), |pot| {
            (
                pot.pot_micro,
                pot.holds_micro,
                pot.spent_micro,
                pot.allocations.get(&account).copied().unwrap_or(0),
            )
        });
        let hourly_micro = u64::from(ceiling_cents_per_hour) * MICRO_DOLLARS_PER_CENT;
        let runway_minutes = (pot * 60)
            .checked_div(hourly_micro)
            .and_then(|minutes| u32::try_from(minutes).ok());
        let funds = match runway_minutes {
            Some(minutes) if minutes >= 60 => FundsState::Funded,
            Some(minutes) if minutes > 0 => FundsState::Low,
            _ => FundsState::FundingRequired,
        };
        SessionFunding {
            session_id: crate::app_api::ids::hex(session.as_bytes()),
            pot_available_micro_dollars: pot,
            open_holds_micro_dollars: holds,
            spent_micro_dollars: spent,
            own_allocation_micro_dollars: own,
            own_remaining_micro_dollars: own,
            runway_at_ceiling_minutes: runway_minutes,
            runway_at_recent_rate_minutes: None,
            funds,
            contributions_visible: true,
        }
    }

    fn funding_now(&self, account: AccountId, targets: &[FundingTarget]) -> FundingView {
        let state = lock(&self.state);
        let campaigns = targets
            .iter()
            .map(|target| {
                let (preset, ceiling) = Self::pace_of(&state, target.campaign);
                CampaignFunding {
                    campaign_id: crate::app_api::ids::hex(target.campaign.as_bytes()),
                    name: target.name.clone(),
                    can_choose_pace: target.can_choose_pace,
                    preset,
                    ceiling_cents_per_hour: ceiling,
                    session: target
                        .live_session
                        .map(|session| Self::session_funding(&state, account, session, ceiling)),
                }
            })
            .collect();
        FundingView {
            presets: [
                PacePreset::Story,
                PacePreset::Immersive,
                PacePreset::Cinematic,
                PacePreset::Custom,
            ]
            .map(|preset| PresetOffer {
                preset,
                ceiling_cents_per_hour: preset_ceiling(preset),
            })
            .to_vec(),
            is_proposed_pricing: true,
            custom_minimum_cents_per_hour: CUSTOM_MINIMUM_CENTS_PER_HOUR,
            custom_maximum_cents_per_hour: CUSTOM_MAXIMUM_CENTS_PER_HOUR,
            campaigns,
        }
    }

    fn set_pace_now(
        &self,
        account: AccountId,
        campaign: CampaignId,
        preset: PacePreset,
        custom: Option<u32>,
        operation: OperationId,
    ) -> PortResult<()> {
        if let Some(ceiling) = custom
            && !(CUSTOM_MINIMUM_CENTS_PER_HOUR..=CUSTOM_MAXIMUM_CENTS_PER_HOUR).contains(&ceiling)
        {
            return Err(PortError::Refused(Refusal::CeilingOutOfRange));
        }
        let mut state = lock(&self.state);
        if state.receipts.replay(account, operation).is_some() {
            return Ok(());
        }
        state.paces.insert(
            campaign,
            Pace {
                preset,
                custom_ceiling_cents_per_hour: custom,
            },
        );
        state
            .receipts
            .record(account, operation, ActionOutcome::Done);
        Ok(())
    }

    fn allocate_now(
        &self,
        account: AccountId,
        session: SessionId,
        amount_cents: u32,
        operation: OperationId,
    ) -> PortResult<()> {
        let mut state = lock(&self.state);
        if state.receipts.replay(account, operation).is_some() {
            return Ok(());
        }
        let micro = u64::from(amount_cents) * MICRO_DOLLARS_PER_CENT;
        let wallet = state.wallet(account);
        if micro == 0 || wallet.promotional_micro + wallet.purchased_micro < micro {
            return Err(PortError::Refused(Refusal::AmountOutOfRange));
        }
        let from_promotional = micro.min(wallet.promotional_micro);
        wallet.promotional_micro -= from_promotional;
        wallet.purchased_micro -= micro - from_promotional;
        wallet.allocated_micro += micro;
        let pot = state.pots.entry(session).or_insert(Pot {
            pot_micro: 0,
            holds_micro: 0,
            spent_micro: 0,
            allocations: HashMap::new(),
        });
        pot.pot_micro += micro;
        *pot.allocations.entry(account).or_insert(0) += micro;
        state
            .receipts
            .record(account, operation, ActionOutcome::Done);
        Ok(())
    }

    fn statements_now(
        &self,
        account: AccountId,
        cursor: Option<&str>,
    ) -> PortResult<StatementsView> {
        self.statements_filtered_now(account, StatementFilter::All, None, None, cursor)
    }

    fn statements_filtered_now(
        &self,
        account: AccountId,
        filter: StatementFilter,
        from_ms: Option<u64>,
        to_ms: Option<u64>,
        cursor: Option<&str>,
    ) -> PortResult<StatementsView> {
        let start = match cursor {
            None => 0,
            Some(text) => text.parse::<usize>().map_err(|_| PortError::NotFound)?,
        };
        let mut state = lock(&self.state);
        let all = &state.wallet(account).lines;
        let filtered: Vec<StatementLine> = all
            .iter()
            .filter(|line| {
                let in_range = from_ms.is_none_or(|from| line.at_ms >= from)
                    && to_ms.is_none_or(|to| line.at_ms <= to);
                let in_filter = match filter {
                    StatementFilter::All => true,
                    StatementFilter::TopUps => line.direction == Direction::Credit,
                    StatementFilter::Spend => line.direction == Direction::Charge,
                };
                in_range && in_filter
            })
            .cloned()
            .collect();
        let totals = totals_of(&filtered);
        let monthly = monthly_of(&filtered);
        let per_campaign = per_campaign_of(&filtered);
        let page: Vec<StatementLine> = filtered
            .iter()
            .skip(start)
            .take(STATEMENT_PAGE_LINES)
            .cloned()
            .collect();
        let end = start + page.len();
        Ok(StatementsView {
            next_cursor: (end < filtered.len()).then(|| end.to_string()),
            lines: page,
            totals,
            monthly,
            per_campaign,
        })
    }

    /// Seeds a charge at a chosen time, for preview history across months.
    pub fn seed_charge_at(
        &self,
        account: AccountId,
        at_ms: u64,
        category: StatementCategory,
        campaign_name: &str,
        quantity: Quantity,
        retail_micro_dollars: u64,
    ) {
        let mut state = lock(&self.state);
        let mut line = self.line_for(category, Direction::Charge, retail_micro_dollars);
        line.at_ms = at_ms;
        line.campaign_name = Some(campaign_name.to_owned());
        line.quantity = quantity;
        let mut entry = self.entry_for(LedgerKind::Usage, retail_micro_dollars);
        entry.at_ms = at_ms;
        entry.campaign_name = Some(campaign_name.to_owned());
        self.post_line(&mut state, account, line, Some(entry));
        // Keep newest first after inserting a dated line.
        let wallet = state.wallet(account);
        wallet
            .lines
            .sort_by_key(|line| std::cmp::Reverse(line.at_ms));
        wallet
            .ledger
            .sort_by_key(|entry| std::cmp::Reverse(entry.at_ms));
    }

    /// Seeds a credit (top-up, promotional or refund) at a chosen time.
    pub fn seed_credit_at(
        &self,
        account: AccountId,
        at_ms: u64,
        category: StatementCategory,
        amount_micro_dollars: u64,
    ) {
        let mut state = lock(&self.state);
        let mut line = self.line_for(category, Direction::Credit, amount_micro_dollars);
        line.at_ms = at_ms;
        if category == StatementCategory::Refund {
            line.direction = Direction::Credit;
            line.status = LineStatus::Settled;
        }
        let kind = match category {
            StatementCategory::Purchase => LedgerKind::Purchase,
            StatementCategory::Refund => LedgerKind::Refund,
            _ => LedgerKind::GoodwillCredit,
        };
        let mut entry = self.entry_for(kind, amount_micro_dollars);
        entry.at_ms = at_ms;
        self.post_line(&mut state, account, line, Some(entry));
        let wallet = state.wallet(account);
        wallet
            .lines
            .sort_by_key(|line| std::cmp::Reverse(line.at_ms));
        wallet
            .ledger
            .sort_by_key(|entry| std::cmp::Reverse(entry.at_ms));
    }

    fn closure_blockers_now(&self, account: AccountId) -> Vec<ClosureBlocker> {
        let mut state = lock(&self.state);
        let has_dispute = !state.disputes.is_empty()
            && state
                .disputes
                .iter()
                .any(|dispute| dispute.account_id == crate::app_api::ids::account_text(account));
        let wallet = state.wallet(account);
        let mut blockers = Vec::new();
        if wallet.purchased_micro > 0 {
            blockers.push(ClosureBlocker::PurchasedBalance {
                amount_micro_dollars: wallet.purchased_micro,
            });
        }
        if wallet.reserved_micro > 0 {
            blockers.push(ClosureBlocker::PendingHolds {
                amount_micro_dollars: wallet.reserved_micro,
            });
        }
        if has_dispute {
            blockers.push(ClosureBlocker::OpenDispute);
        }
        blockers
    }
}

pub(super) fn month_of(at_ms: u64) -> String {
    // Civil date from days since the Unix epoch (Howard Hinnant's algorithm).
    let days = (at_ms / 86_400_000) as i64;
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    if month <= 2 {
        year += 1;
    }
    format!("{year:04}-{month:02}")
}

pub(super) fn totals_of(lines: &[StatementLine]) -> StatementTotals {
    let mut totals = StatementTotals::default();
    for line in lines {
        match line.category {
            StatementCategory::Refund => totals.refunded_micro_dollars += line.amount_micro_dollars,
            StatementCategory::Purchase if line.direction == Direction::Credit => {
                totals.topped_up_micro_dollars += line.amount_micro_dollars;
            }
            StatementCategory::TrialCredit
            | StatementCategory::GoodwillCredit
            | StatementCategory::TechnicalCredit
                if line.direction == Direction::Credit =>
            {
                totals.promotional_granted_micro_dollars += line.amount_micro_dollars;
            }
            _ if line.direction == Direction::Charge => {
                totals.spent_micro_dollars += line.amount_micro_dollars;
            }
            _ => {}
        }
    }
    totals
}

fn monthly_of(lines: &[StatementLine]) -> Vec<MonthlySummary> {
    let mut months: Vec<MonthlySummary> = Vec::new();
    for line in lines {
        let month = month_of(line.at_ms);
        let position = months.iter().position(|entry| entry.month == month);
        let index = match position {
            Some(index) => index,
            None => {
                months.push(MonthlySummary {
                    month,
                    topped_up_micro_dollars: 0,
                    promotional_granted_micro_dollars: 0,
                    spent_micro_dollars: 0,
                    refunded_micro_dollars: 0,
                });
                months.len() - 1
            }
        };
        let entry = &mut months[index];
        let single = totals_of(std::slice::from_ref(line));
        entry.topped_up_micro_dollars += single.topped_up_micro_dollars;
        entry.promotional_granted_micro_dollars += single.promotional_granted_micro_dollars;
        entry.spent_micro_dollars += single.spent_micro_dollars;
        entry.refunded_micro_dollars += single.refunded_micro_dollars;
    }
    months.sort_by(|a, b| b.month.cmp(&a.month));
    months
}

fn per_campaign_of(lines: &[StatementLine]) -> Vec<CampaignSpend> {
    let mut campaigns: Vec<CampaignSpend> = Vec::new();
    for line in lines
        .iter()
        .filter(|line| line.direction == Direction::Charge)
    {
        let Some(name) = &line.campaign_name else {
            continue;
        };
        if let Some(entry) = campaigns
            .iter_mut()
            .find(|entry| &entry.campaign_name == name)
        {
            entry.spent_micro_dollars += line.amount_micro_dollars;
        } else {
            campaigns.push(CampaignSpend {
                campaign_name: name.clone(),
                spent_micro_dollars: line.amount_micro_dollars,
            });
        }
    }
    campaigns.sort_by_key(|entry| std::cmp::Reverse(entry.spent_micro_dollars));
    campaigns
}

impl FinancePort for FakeFinance {
    async fn cancel_host_lobby(&self, account: AccountId, session: SessionId) -> PortResult<()> {
        let mut state = lock(&self.state);
        let (host, _) = state
            .host_sessions
            .get(&session)
            .ok_or(PortError::NotFound)?;
        if *host != account {
            return Err(PortError::NotFound);
        }
        state.cancelled_host_sessions.insert(session);
        Ok(())
    }

    async fn host_credit(
        &self,
        account: AccountId,
        session: SessionId,
    ) -> PortResult<Option<df_account_app::wire_campaign::HostCreditView>> {
        let state = lock(&self.state);
        if state.cancelled_host_sessions.contains(&session) {
            return Ok(None);
        }
        Ok(state
            .host_sessions
            .get(&session)
            .filter(|(host, _)| *host == account)
            .map(|(_, quote)| *quote))
    }

    async fn consent_host_credit(
        &self,
        account: AccountId,
        session: SessionId,
        request: df_account_app::wire_campaign::HostConsentRequest,
        _operation: OperationId,
    ) -> PortResult<()> {
        let mut state = lock(&self.state);
        if state.cancelled_host_sessions.contains(&session) {
            return Err(PortError::Forbidden);
        }

        let (host, quote) = state
            .host_sessions
            .get_mut(&session)
            .ok_or(PortError::NotFound)?;
        if *host != account {
            return Err(PortError::Forbidden);
        }
        if request.price_version != quote.price_version
            || request.terms_version != quote.terms_version
            || request.cap_micro_dollars != quote.cap_micro_dollars
        {
            return Err(PortError::Forbidden);
        }
        if quote.consented_at_ms.is_none() {
            quote.consented_at_ms =
                Some(i64::try_from(self.now_ms()).map_err(|_| PortError::Unavailable)?);
        }
        quote.is_fundable = quote.available_micro_dollars >= quote.cap_micro_dollars;
        Ok(())
    }

    async fn overview(&self, account: AccountId, is_verified: bool) -> PortResult<OverviewView> {
        Ok(self.overview_now(account, is_verified))
    }

    async fn funding(
        &self,
        account: AccountId,
        targets: &[FundingTarget],
    ) -> PortResult<FundingView> {
        Ok(self.funding_now(account, targets))
    }

    async fn set_pace(
        &self,
        account: AccountId,
        campaign: CampaignId,
        preset: PacePreset,
        custom_ceiling_cents_per_hour: Option<u32>,
        operation: OperationId,
    ) -> PortResult<()> {
        self.set_pace_now(
            account,
            campaign,
            preset,
            custom_ceiling_cents_per_hour,
            operation,
        )
    }

    async fn allocate(
        &self,
        account: AccountId,
        session: SessionId,
        amount_cents: u32,
        operation: OperationId,
    ) -> PortResult<()> {
        self.allocate_now(account, session, amount_cents, operation)
    }

    async fn statements(
        &self,
        account: AccountId,
        cursor: Option<&str>,
    ) -> PortResult<StatementsView> {
        self.statements_now(account, cursor)
    }

    async fn statements_filtered(
        &self,
        account: AccountId,
        filter: StatementFilter,
        from_ms: Option<u64>,
        to_ms: Option<u64>,
        cursor: Option<&str>,
    ) -> PortResult<StatementsView> {
        self.statements_filtered_now(account, filter, from_ms, to_ms, cursor)
    }

    async fn account_stats(&self, account: AccountId, now_ms: u64) -> PortResult<AccountStats> {
        let mut state = lock(&self.state);
        let wallet = state.wallet(account);
        let totals = totals_of(&wallet.lines);
        let month = month_of(now_ms);
        let spent_this_month: u64 = wallet
            .lines
            .iter()
            .filter(|line| month_of(line.at_ms) == month)
            .map(|line| totals_of(std::slice::from_ref(line)).spent_micro_dollars)
            .sum();
        Ok(AccountStats {
            spent_this_month_micro_dollars: spent_this_month,
            spent_lifetime_micro_dollars: totals.spent_micro_dollars,
            ..AccountStats::default()
        })
    }

    async fn operator_account_totals(
        &self,
        account: AccountId,
    ) -> PortResult<(u64, u64, Option<u64>)> {
        let mut state = lock(&self.state);
        let wallet = state.wallet(account);
        let totals = totals_of(&wallet.lines);
        let last = wallet.lines.iter().map(|line| line.at_ms).max();
        Ok((
            totals.topped_up_micro_dollars,
            totals.spent_micro_dollars,
            last,
        ))
    }

    async fn closure_blockers(&self, account: AccountId) -> PortResult<Vec<ClosureBlocker>> {
        Ok(self.closure_blockers_now(account))
    }

    async fn operator_summary(&self) -> PortResult<FinanceSummary> {
        Ok(self.summary_now())
    }

    async fn operator_balance(&self, account: AccountId) -> PortResult<u64> {
        Ok(self.balance_now(account))
    }

    async fn operator_ledger(
        &self,
        account: AccountId,
    ) -> PortResult<(Vec<LedgerEntry>, Vec<RefundableLot>)> {
        Ok(self.ledger_now(account))
    }

    async fn refund(
        &self,
        act: OperatorAct,
        account: AccountId,
        lot_id: &str,
        amount_cents: u32,
    ) -> PortResult<ActionOutcome> {
        self.refund_now(&act, account, lot_id, amount_cents)
    }

    async fn goodwill_credit(
        &self,
        act: OperatorAct,
        account: AccountId,
        amount_cents: u32,
        _support_case: &str,
    ) -> PortResult<ActionOutcome> {
        self.goodwill_now(&act, account, amount_cents)
    }

    async fn disputes(&self) -> PortResult<Vec<DisputeRow>> {
        Ok(lock(&self.state).disputes.clone())
    }

    async fn dispute_evidence(
        &self,
        act: OperatorAct,
        dispute_id: &str,
    ) -> PortResult<ActionOutcome> {
        self.evidence_now(&act, dispute_id)
    }

    async fn reconciliation(&self) -> PortResult<ReconciliationView> {
        Ok(lock(&self.state).reconciliation.clone())
    }

    async fn providers(&self) -> PortResult<Vec<ProviderRow>> {
        Ok(lock(&self.state).providers.clone())
    }

    async fn switch_provider(
        &self,
        act: OperatorAct,
        provider_id: &str,
        switch: ProviderSwitch,
    ) -> PortResult<ActionOutcome> {
        self.switch_now(&act, provider_id, switch)
    }
}
