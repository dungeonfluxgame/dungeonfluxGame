//! The operator console (plan 9.13, 7.3): account lookup, the full ledger, refunds, goodwill
//! credits, disputes, reconciliation, provider health, the review queue and the audit log.
//!
//! Every mutation carries a reason and an operation id; the server checks the capability, records
//! the audit row and refuses what the account may not do. Hiding a control here is presentation.

use super::{
    Effect, Field, FocusTarget, Form, Intent, Model, OperationSlot, Remote, Screen, Submit,
};
use crate::{
    api::{ApiCall, ProviderSwitch, Reply},
    money::parse_dollars,
    route::AdminRoute,
    wire::{
        AdminAccountDetail, AdminAccountRow, AdminActionView, AdminSummary, ApiError, AuditRow,
        DisputeRow, GoodwillCreditRequest, MAXIMUM_REASON_CHARS, ProviderRow, ReasonRequest,
        ReconciliationView, RefundRequest, Refusal, ReviewDecision, ReviewDecisionRequest,
        ReviewItem,
    },
};

#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(
    clippy::large_enum_variant,
    reason = "the model holds one screen at a time, so boxing the account screen only adds an indirection"
)]
pub enum AdminScreen {
    Summary(Remote<AdminSummary>),
    Accounts(SearchScreen),
    Account(AccountScreen),
    Disputes(RowsScreen<DisputeRow>),
    Reconciliation(Remote<ReconciliationView>),
    Providers(RowsScreen<ProviderRow>),
    Review(RowsScreen<ReviewItem>),
    Audit(Remote<Vec<AuditRow>>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchScreen {
    pub query: String,
    pub results: Option<Remote<Vec<AdminAccountRow>>>,
    pub submit: Submit,
    pub standing: String,
    pub verified: String,
    pub has_balance: String,
    pub sort: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RefundForm {
    pub lot_id: Option<String>,
    pub amount: String,
    pub reason: String,
    pub operation: Remote<String>,
    pub submit: Submit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditGrantForm {
    pub amount: String,
    pub reason: String,
    pub support_case: String,
    pub operation: Remote<String>,
    pub submit: Submit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountScreen {
    pub account_id: String,
    pub detail: Remote<AdminAccountDetail>,
    pub refund: RefundForm,
    pub credit: CreditGrantForm,
    pub last: Option<AdminActionView>,
    pub tab: crate::wire::AdminTab,
}

/// A list with at most one open reason form for one of its rows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RowsScreen<T> {
    pub rows: Remote<Vec<T>>,
    pub action: Option<ReasonAction>,
    pub last: Option<AdminActionView>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionTarget {
    DisputeEvidence {
        dispute_id: String,
    },
    Provider {
        provider_id: String,
        switch: ProviderSwitch,
    },
    Review {
        item_id: String,
        decision: ReviewDecision,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReasonAction {
    pub target: ActionTarget,
    pub reason: String,
    pub operation: Remote<String>,
    pub submit: Submit,
}

fn rows<T>() -> RowsScreen<T> {
    RowsScreen {
        rows: Remote::Loading,
        action: None,
        last: None,
    }
}

/// Refuses an empty or overlong reason (B plan 10.1: 1..=500 characters).
fn reason_refusal(reason: &str) -> Option<Refusal> {
    let length = reason.trim().chars().count();
    if length == 0 {
        Some(Refusal::ReasonRequired)
    } else if length > MAXIMUM_REASON_CHARS {
        Some(Refusal::ReasonTooLong)
    } else {
        None
    }
}

impl AdminScreen {
    pub(super) fn operation_issued(&mut self, slot: OperationSlot, operation: Remote<String>) {
        match (self, slot) {
            (Self::Account(screen), OperationSlot::Primary) => screen.refund.operation = operation,
            (Self::Account(screen), OperationSlot::Secondary) => {
                screen.credit.operation = operation;
            }
            (Self::Disputes(screen), _) => set_action_operation(&mut screen.action, operation),
            (Self::Providers(screen), _) => set_action_operation(&mut screen.action, operation),
            (Self::Review(screen), _) => set_action_operation(&mut screen.action, operation),
            _ => {}
        }
    }

    fn action_mut(&mut self) -> Option<&mut ReasonAction> {
        match self {
            Self::Disputes(screen) => screen.action.as_mut(),
            Self::Providers(screen) => screen.action.as_mut(),
            Self::Review(screen) => screen.action.as_mut(),
            _ => None,
        }
    }
}

fn set_action_operation(action: &mut Option<ReasonAction>, operation: Remote<String>) {
    if let Some(action) = action.as_mut() {
        action.operation = operation;
    }
}

impl Model {
    pub(super) fn enter_admin(&mut self, route: AdminRoute) -> Vec<Effect> {
        let (screen, call) = match route {
            AdminRoute::Summary => (
                AdminScreen::Summary(Remote::Loading),
                Some(ApiCall::AdminSummary),
            ),
            AdminRoute::Accounts => (
                AdminScreen::Accounts(SearchScreen {
                    query: String::new(),
                    results: None,
                    submit: Submit::Idle,
                    standing: String::new(),
                    verified: String::new(),
                    has_balance: String::new(),
                    sort: "last_activity".to_owned(),
                }),
                None,
            ),
            AdminRoute::Account { account_id } => (
                AdminScreen::Account(AccountScreen {
                    account_id: account_id.clone(),
                    detail: Remote::Loading,
                    refund: RefundForm {
                        lot_id: None,
                        amount: String::new(),
                        reason: String::new(),
                        operation: Remote::Loading,
                        submit: Submit::Idle,
                    },
                    credit: CreditGrantForm {
                        amount: String::new(),
                        reason: String::new(),
                        support_case: String::new(),
                        operation: Remote::Loading,
                        submit: Submit::Idle,
                    },
                    last: None,
                    tab: crate::wire::AdminTab::Details,
                }),
                Some(ApiCall::AdminAccount { account_id }),
            ),
            AdminRoute::Disputes => (AdminScreen::Disputes(rows()), Some(ApiCall::Disputes)),
            AdminRoute::Reconciliation => (
                AdminScreen::Reconciliation(Remote::Loading),
                Some(ApiCall::Reconciliation),
            ),
            AdminRoute::Providers => (AdminScreen::Providers(rows()), Some(ApiCall::Providers)),
            AdminRoute::Review => (AdminScreen::Review(rows()), Some(ApiCall::ReviewQueue)),
            AdminRoute::Audit => (AdminScreen::Audit(Remote::Loading), Some(ApiCall::Audit)),
        };
        let is_account = matches!(screen, AdminScreen::Account(_));
        self.screen = Screen::Admin(screen);
        let mut effects: Vec<Effect> = call.into_iter().map(|call| self.api(call)).collect();
        if is_account {
            effects.push(self.issue_operation(OperationSlot::Primary));
            effects.push(self.issue_operation(OperationSlot::Secondary));
        }
        effects
    }

    pub(super) fn admin_intent(&mut self, intent: Intent) -> Vec<Effect> {
        let Screen::Admin(screen) = &mut self.screen else {
            return Vec::new();
        };
        match (intent, screen) {
            (Intent::SelectAdminTab(tab), AdminScreen::Account(screen)) => {
                screen.tab = tab;
                Vec::new()
            }
            (Intent::SetAdminStanding(value), AdminScreen::Accounts(screen)) => {
                screen.standing = value;
                Vec::new()
            }
            (Intent::SetAdminVerified(value), AdminScreen::Accounts(screen)) => {
                screen.verified = value;
                Vec::new()
            }
            (Intent::SetAdminHasBalance(value), AdminScreen::Accounts(screen)) => {
                screen.has_balance = value;
                Vec::new()
            }
            (Intent::SetAdminSort(value), AdminScreen::Accounts(screen)) => {
                screen.sort = value;
                Vec::new()
            }
            (Intent::ChooseLot { lot_id }, AdminScreen::Account(screen)) => {
                screen.refund.lot_id = Some(lot_id);
                screen.refund.submit = Submit::Idle;
                Vec::new()
            }
            (Intent::OpenAction(target), screen) => {
                let action = Some(ReasonAction {
                    target,
                    reason: String::new(),
                    operation: Remote::Loading,
                    submit: Submit::Idle,
                });
                match screen {
                    AdminScreen::Disputes(screen) => screen.action = action,
                    AdminScreen::Providers(screen) => screen.action = action,
                    AdminScreen::Review(screen) => screen.action = action,
                    _ => return Vec::new(),
                }
                vec![self.issue_operation(OperationSlot::Primary)]
            }
            (Intent::CancelEdit, screen) => {
                match screen {
                    AdminScreen::Disputes(screen) => screen.action = None,
                    AdminScreen::Providers(screen) => screen.action = None,
                    AdminScreen::Review(screen) => screen.action = None,
                    _ => {}
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    pub(super) fn edit_admin(&mut self, field: Field, value: String) {
        let Screen::Admin(screen) = &mut self.screen else {
            return;
        };
        let slot = match (screen, field) {
            (AdminScreen::Accounts(screen), Field::SearchQuery) => &mut screen.query,
            (AdminScreen::Account(screen), Field::RefundAmount) => &mut screen.refund.amount,
            (AdminScreen::Account(screen), Field::RefundReason) => &mut screen.refund.reason,
            (AdminScreen::Account(screen), Field::CreditAmount) => &mut screen.credit.amount,
            (AdminScreen::Account(screen), Field::CreditReason) => &mut screen.credit.reason,
            (AdminScreen::Account(screen), Field::SupportCase) => &mut screen.credit.support_case,
            (screen, Field::ActionReason) => match screen.action_mut() {
                Some(action) => &mut action.reason,
                None => return,
            },
            _ => return,
        };
        *slot = value;
    }

    pub(super) fn submit_admin(&mut self, form: Form) -> Vec<Effect> {
        let generation = self.generation;
        let Screen::Admin(screen) = &mut self.screen else {
            return Vec::new();
        };
        let call = match (screen, form) {
            (AdminScreen::Accounts(screen), Form::Search) => {
                if screen.query.trim().is_empty() || screen.submit.is_sending() {
                    return Vec::new();
                }
                screen.submit = Submit::Sending;
                screen.results = Some(Remote::Loading);
                ApiCall::AdminAccounts {
                    query: screen.query.trim().to_owned(),
                    standing: (!screen.standing.is_empty()).then(|| screen.standing.clone()),
                    verified: (!screen.verified.is_empty()).then(|| screen.verified.clone()),
                    has_balance: (!screen.has_balance.is_empty())
                        .then(|| screen.has_balance.clone()),
                    sort: Some(screen.sort.clone()),
                }
            }
            (AdminScreen::Account(screen), Form::Refund) => {
                let form = &mut screen.refund;
                let Some(lot) = screen.detail.ready().and_then(|detail| {
                    let lot_id = form.lot_id.as_ref()?;
                    detail
                        .refundable_lots
                        .iter()
                        .find(|lot| &lot.lot_id == lot_id)
                }) else {
                    form.submit = Submit::Refused(ApiError::refused(Refusal::AmountOutOfRange));
                    return Vec::new();
                };
                let amount = parse_dollars(&form.amount)
                    .ok()
                    .filter(|cents| (1..=lot.refundable_cents).contains(cents));
                let refusal = match (amount, reason_refusal(&form.reason)) {
                    (None, _) => Some(Refusal::AmountOutOfRange),
                    (Some(_), refusal) => refusal,
                };
                if let Some(refusal) = refusal {
                    form.submit = Submit::Refused(ApiError::refused(refusal));
                    return Vec::new();
                }
                let (Some(amount_cents), Some(operation)) =
                    (amount, form.operation.ready().cloned())
                else {
                    return Vec::new();
                };
                if form.submit.is_sending() {
                    return Vec::new();
                }
                form.submit = Submit::Sending;
                ApiCall::Refund(RefundRequest {
                    account_id: screen.account_id.clone(),
                    lot_id: lot.lot_id.clone(),
                    amount_cents,
                    reason: form.reason.trim().to_owned(),
                    operation,
                })
            }
            (AdminScreen::Account(screen), Form::Goodwill) => {
                let form = &mut screen.credit;
                let amount = parse_dollars(&form.amount).ok().filter(|cents| *cents > 0);
                let refusal = if amount.is_none() {
                    Some(Refusal::AmountOutOfRange)
                } else if form.support_case.trim().is_empty() {
                    Some(Refusal::SupportCaseRequired)
                } else {
                    reason_refusal(&form.reason)
                };
                if let Some(refusal) = refusal {
                    form.submit = Submit::Refused(ApiError::refused(refusal));
                    return Vec::new();
                }
                let (Some(amount_cents), Some(operation)) =
                    (amount, form.operation.ready().cloned())
                else {
                    return Vec::new();
                };
                if form.submit.is_sending() {
                    return Vec::new();
                }
                form.submit = Submit::Sending;
                ApiCall::GoodwillCredit(GoodwillCreditRequest {
                    account_id: screen.account_id.clone(),
                    amount_cents,
                    reason: form.reason.trim().to_owned(),
                    support_case: form.support_case.trim().to_owned(),
                    operation,
                })
            }
            (screen, Form::Action) => {
                let Some(action) = screen.action_mut() else {
                    return Vec::new();
                };
                if let Some(refusal) = reason_refusal(&action.reason) {
                    action.submit = Submit::Refused(ApiError::refused(refusal));
                    return Vec::new();
                }
                let Some(operation) = action.operation.ready().cloned() else {
                    return Vec::new();
                };
                if action.submit.is_sending() {
                    return Vec::new();
                }
                action.submit = Submit::Sending;
                let request = ReasonRequest {
                    reason: action.reason.trim().to_owned(),
                    operation,
                };
                match action.target.clone() {
                    ActionTarget::DisputeEvidence { dispute_id } => ApiCall::DisputeEvidence {
                        dispute_id,
                        request,
                    },
                    ActionTarget::Provider {
                        provider_id,
                        switch,
                    } => ApiCall::SwitchProvider {
                        provider_id,
                        switch,
                        request,
                    },
                    ActionTarget::Review { item_id, decision } => ApiCall::DecideReview {
                        item_id,
                        request: ReviewDecisionRequest {
                            decision,
                            reason: request.reason,
                            operation: request.operation,
                        },
                    },
                }
            }
            _ => return Vec::new(),
        };
        vec![Effect::Api { generation, call }]
    }

    pub(super) fn admin_reply(&mut self, reply: Reply) -> Vec<Effect> {
        let generation = self.generation;
        let Screen::Admin(screen) = &mut self.screen else {
            return Vec::new();
        };
        match (reply, screen) {
            (Reply::AdminSummary(result), AdminScreen::Summary(summary)) => {
                *summary = Remote::from_result(result);
                Vec::new()
            }
            (Reply::AdminAccounts(result), AdminScreen::Accounts(screen)) => {
                screen.submit = Submit::Idle;
                screen.results = Some(Remote::from_result(result));
                Vec::new()
            }
            (Reply::AdminAccount(result), AdminScreen::Account(screen)) => {
                screen.detail = Remote::from_result(result);
                Vec::new()
            }
            (Reply::AdminAction(result), AdminScreen::Account(screen)) => {
                let slot = if screen.refund.submit.is_sending() {
                    OperationSlot::Primary
                } else {
                    OperationSlot::Secondary
                };
                let submit = match slot {
                    OperationSlot::Primary => &mut screen.refund.submit,
                    OperationSlot::Secondary => &mut screen.credit.submit,
                };
                match result {
                    Ok(view) => {
                        *submit = Submit::Idle;
                        screen.last = Some(view);
                        match slot {
                            OperationSlot::Primary => {
                                screen.refund.amount.clear();
                                screen.refund.reason.clear();
                                screen.refund.lot_id = None;
                                screen.refund.operation = Remote::Loading;
                            }
                            OperationSlot::Secondary => {
                                screen.credit.amount.clear();
                                screen.credit.reason.clear();
                                screen.credit.support_case.clear();
                                screen.credit.operation = Remote::Loading;
                            }
                        }
                        let account_id = screen.account_id.clone();
                        vec![
                            Effect::IssueOperation { generation, slot },
                            Effect::Api {
                                generation,
                                call: ApiCall::AdminAccount { account_id },
                            },
                            Effect::Focus(FocusTarget::Heading),
                        ]
                    }
                    Err(error) => {
                        *submit = Submit::Refused(error);
                        vec![Effect::Focus(FocusTarget::ErrorSummary)]
                    }
                }
            }
            (Reply::AdminAction(result), screen) => {
                let reload = match screen {
                    AdminScreen::Disputes(_) => ApiCall::Disputes,
                    AdminScreen::Providers(_) => ApiCall::Providers,
                    AdminScreen::Review(_) => ApiCall::ReviewQueue,
                    _ => return Vec::new(),
                };
                match result {
                    Ok(view) => {
                        match screen {
                            AdminScreen::Disputes(rows) => finish_action(rows, view),
                            AdminScreen::Providers(rows) => finish_action(rows, view),
                            AdminScreen::Review(rows) => finish_action(rows, view),
                            _ => {}
                        }
                        vec![
                            Effect::Api {
                                generation,
                                call: reload,
                            },
                            Effect::Focus(FocusTarget::Heading),
                        ]
                    }
                    Err(error) => {
                        if let Some(action) = screen.action_mut() {
                            action.submit = Submit::Refused(error);
                        }
                        vec![Effect::Focus(FocusTarget::ErrorSummary)]
                    }
                }
            }
            (Reply::Disputes(result), AdminScreen::Disputes(screen)) => {
                screen.rows = Remote::from_result(result);
                Vec::new()
            }
            (Reply::Reconciliation(result), AdminScreen::Reconciliation(view)) => {
                *view = Remote::from_result(result);
                Vec::new()
            }
            (Reply::Providers(result), AdminScreen::Providers(screen)) => {
                screen.rows = Remote::from_result(result);
                Vec::new()
            }
            (Reply::ReviewQueue(result), AdminScreen::Review(screen)) => {
                screen.rows = Remote::from_result(result);
                Vec::new()
            }
            (Reply::Audit(result), AdminScreen::Audit(view)) => {
                *view = Remote::from_result(result);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }
}

fn finish_action<T>(screen: &mut RowsScreen<T>, view: AdminActionView) {
    screen.action = None;
    screen.last = Some(view);
}
