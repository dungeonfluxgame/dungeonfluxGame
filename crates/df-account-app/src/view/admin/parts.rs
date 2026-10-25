//! Pieces the operator screens share: tables, pills, number formats and reason forms.
//!
//! Operator forms refuse with amount and reason problems that `copy::refusal_field` cannot place
//! (the same refusal belongs to a different field on each form), so the error summary and the
//! inline messages here resolve the field from the form they sit in.

use crate::{
    copy,
    node::{Element, Node, el, text},
    state::{Field, Form, Intent, ReasonAction, Remote},
    view::{ButtonKind, InputKind, button, failed, form, submit_button},
    wire::{
        ActionOutcome, AdminActionView, ApiError, MAXIMUM_REASON_CHARS, OperatorCapability,
        OperatorView, Refusal,
    },
};

const MICRO_DOLLARS_PER_DOLLAR: u64 = 1_000_000;
const MINIMUM_FRACTION_DIGITS: usize = 2;

/// Hiding a control is presentation only: the server checks the capability on every call.
pub(super) fn can(operator: &OperatorView, capability: OperatorCapability) -> bool {
    operator.capabilities.contains(&capability)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Tone {
    Plain,
    Ok,
    Wait,
    Warn,
    Bad,
}

pub(super) fn pill(tone: Tone, words: impl Into<String>) -> Node {
    let class = match tone {
        Tone::Plain => "pill",
        Tone::Ok => "pill pill-ok",
        Tone::Wait => "pill pill-wait",
        Tone::Warn => "pill pill-warn",
        Tone::Bad => "pill pill-bad",
    };
    el("span").class(class).text(words).into()
}

/// Gives a control a name that says what it acts on; the visible text stays inside the name.
pub(super) fn named(control: Node, name: String) -> Node {
    match control {
        Node::Element(element) => element.attr("aria-label", name).into(),
        other @ Node::Text(_) => other,
    }
}

pub(super) fn plural(count: u64, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// Exact dollars from micro-dollars with at least cents: `$12.34`, `$0.0042`.
pub(super) fn exact_dollars(micro_dollars: u64) -> String {
    let mut fraction = format!("{:06}", micro_dollars % MICRO_DOLLARS_PER_DOLLAR);
    while fraction.len() > MINIMUM_FRACTION_DIGITS && fraction.ends_with('0') {
        fraction.pop();
    }
    format!("${}.{fraction}", micro_dollars / MICRO_DOLLARS_PER_DOLLAR)
}

/// A price multiplier from permille: `1.50x`, or `1.505x` when the third digit matters.
pub(super) fn multiplier(permille: u32) -> String {
    let (whole, fraction) = (permille / 1000, permille % 1000);
    if fraction % 10 == 0 {
        format!("{whole}.{:02}x", fraction / 10)
    } else {
        format!("{whole}.{fraction:03}x")
    }
}

/// A rate from permille: `2.5%`.
pub(super) fn rate(permille: u32) -> String {
    format!("{}.{}%", permille / 10, permille % 10)
}

pub(super) struct Column {
    pub label: &'static str,
    pub is_number: bool,
}

pub(super) const fn column(label: &'static str) -> Column {
    Column {
        label,
        is_number: false,
    }
}

pub(super) const fn number_column(label: &'static str) -> Column {
    Column {
        label,
        is_number: true,
    }
}

fn numeric(cell: Element, is_number: bool) -> Element {
    if is_number { cell.class("num") } else { cell }
}

/// A captioned table whose first cell of each row is its row header. Each cell carries its column
/// name so the table stacks on a narrow screen.
pub(super) fn data_table(caption: &str, columns: &[Column], rows: Vec<Vec<Node>>) -> Node {
    let header = el("tr").children(columns.iter().map(|column| {
        numeric(el("th").attr("scope", "col"), column.is_number)
            .text(column.label)
            .into()
    }));
    let body = rows.into_iter().map(|cells| {
        let mut row = el("tr");
        for (index, (cell, column)) in cells.into_iter().zip(columns).enumerate() {
            row = row.child(if index == 0 {
                el("th").attr("scope", "row").child(cell)
            } else {
                numeric(el("td").attr("data-label", column.label), column.is_number).child(cell)
            });
        }
        row.into()
    });
    el("div")
        .class("data-wrap")
        .child(
            el("table")
                .class("data data-stack")
                .child(el("caption").text(caption))
                .child(el("thead").child(header))
                .child(el("tbody").children(body)),
        )
        .into()
}

/// The fields a form's amount and reason problems belong to.
#[derive(Clone, Copy, Debug)]
pub(super) struct FormFields {
    pub amount: Option<Field>,
    pub reason: Field,
}

pub(super) const REFUND_FIELDS: FormFields = FormFields {
    amount: Some(Field::RefundAmount),
    reason: Field::RefundReason,
};
pub(super) const GOODWILL_FIELDS: FormFields = FormFields {
    amount: Some(Field::CreditAmount),
    reason: Field::CreditReason,
};
const ACTION_FIELDS: FormFields = FormFields {
    amount: None,
    reason: Field::ActionReason,
};

/// A form's refusal, placed on the field it is about.
pub(super) struct Refused {
    error: Option<ApiError>,
    fields: FormFields,
}

impl Refused {
    pub(super) const fn new(error: Option<ApiError>, fields: FormFields) -> Self {
        Self { error, fields }
    }

    fn target(&self) -> Option<(Refusal, Field)> {
        let refusal = self.error?.refusal?;
        let field = match refusal {
            Refusal::AmountOutOfRange => self.fields.amount,
            Refusal::ReasonRequired | Refusal::ReasonTooLong => Some(self.fields.reason),
            Refusal::SupportCaseRequired => Some(Field::SupportCase),
            _ => None,
        }?;
        Some((refusal, field))
    }

    fn message_for(&self, field: Field) -> Option<&'static str> {
        self.target()
            .filter(|(_, target)| *target == field)
            .map(|(refusal, _)| copy::refusal_message(refusal))
    }

    /// The summary the runtime focuses after a refusal, linked to the field when there is one.
    pub(super) fn summary(&self) -> Node {
        let Some(error) = self.error else {
            return el("div").class("error-slot").into();
        };
        let problem = copy::problem(error);
        let mut summary = el("div")
            .class("error-summary")
            .attr("role", "alert")
            .attr("tabindex", "-1")
            .attr("data-df-focus", "error-summary")
            .child(el("h2").text("There is a problem"));
        summary = match self.target() {
            Some((_, field)) => summary.child(
                el("p").child(
                    el("a")
                        .attr("href", format!("#{}", field.name()))
                        .text(problem.what),
                ),
            ),
            None => summary.child(el("p").text(problem.what)),
        };
        if !problem.fix.is_empty() {
            summary = summary.child(el("p").text(problem.fix));
        }
        summary.into()
    }
}

pub(super) struct Input<'a> {
    pub field: Field,
    pub label: &'a str,
    pub value: &'a str,
    pub kind: InputKind,
    pub hint: String,
}

pub(super) fn input(spec: Input<'_>, refused: &Refused) -> Node {
    let id = spec.field.name();
    let message = refused.message_for(spec.field);
    let mut described = vec![format!("{id}-hint")];
    let mut wrapper = el("div")
        .class(if message.is_some() {
            "field has-error"
        } else {
            "field"
        })
        .child(el("label").attr("for", id).text(spec.label))
        .child(
            el("p")
                .class("hint")
                .attr("id", format!("{id}-hint"))
                .text(spec.hint),
        );
    if let Some(message) = message {
        described.push(format!("{id}-error"));
        wrapper = wrapper.child(
            el("p")
                .class("field-error")
                .attr("id", format!("{id}-error"))
                .text(message),
        );
    }
    let mut control = el("input")
        .attr("id", id)
        .attr("name", id)
        .attr("type", "text")
        .attr("autocomplete", "off")
        .attr("value", spec.value)
        .attr("aria-describedby", described.join(" "))
        .bind(spec.field);
    if spec.kind == InputKind::Dollars {
        control = control.attr("inputmode", "decimal");
    }
    if message.is_some() {
        control = control.attr("aria-invalid", "true");
    }
    wrapper.child(control).into()
}

/// The reason rule and how much room is left: `1 to 500 characters, kept in the audit log. 480 left.`
pub(super) fn reason_hint(reason: &str) -> String {
    let used = reason.trim().chars().count();
    let rule = format!("1 to {MAXIMUM_REASON_CHARS} characters, kept in the audit log.");
    match MAXIMUM_REASON_CHARS.checked_sub(used) {
        Some(left) => format!("{rule} {left} left."),
        None => format!("{rule} {} too many.", used - MAXIMUM_REASON_CHARS),
    }
}

pub(super) fn reason_input(field: Field, value: &str, refused: &Refused) -> Node {
    input(
        Input {
            field,
            label: "Reason",
            value,
            kind: InputKind::Text,
            hint: reason_hint(value),
        },
        refused,
    )
}

/// Why a form cannot be sent yet: its idempotency key could not be issued.
pub(super) fn operation_problem(operation: &Remote<String>) -> Option<Node> {
    match operation {
        Remote::Failed(error) => Some(failed(*error)),
        Remote::Loading | Remote::Ready(_) => None,
    }
}

/// What an operator action did, in words and a pill.
pub(super) fn last_result(action: &AdminActionView) -> Node {
    let (tone, state, detail) = match action.outcome {
        ActionOutcome::Done => (Tone::Ok, "Done", ""),
        ActionOutcome::Pending => (
            Tone::Wait,
            "Pending",
            " Sent to the processor; the final status arrives by webhook.",
        ),
    };
    el("div")
        .class("panel admin-result")
        .attr("role", "status")
        .child(pill(tone, state))
        .child(text(format!(
            " The action is recorded in the audit log as {}.{detail}",
            action.audit_id
        )))
        .into()
}

/// The open reason form of a list row.
pub(super) struct ReasonPanel<'a> {
    pub title: String,
    pub detail: &'a str,
    pub submit_label: &'a str,
    pub sending_label: &'a str,
    pub action: &'a ReasonAction,
}

pub(super) fn reason_panel(spec: ReasonPanel<'_>) -> Node {
    let action = spec.action;
    let refused = Refused::new(action.submit.error(), ACTION_FIELDS);
    el("div")
        .class("panel panel-warn admin-action")
        .child(el("h2").text(spec.title.clone()))
        .child(el("p").text(spec.detail))
        .children(operation_problem(&action.operation))
        .child(
            form(Form::Action, &spec.title)
                .child(refused.summary())
                .child(reason_input(Field::ActionReason, &action.reason, &refused))
                .child(
                    el("div")
                        .class("actions")
                        .child(submit_button(
                            spec.submit_label,
                            spec.sending_label,
                            action.submit.is_sending(),
                        ))
                        .child(button(
                            "Cancel",
                            Some(Intent::CancelEdit),
                            ButtonKind::Secondary,
                            false,
                        )),
                ),
        )
        .into()
}

pub(super) fn empty(words: &str) -> Node {
    el("p").class("empty").text(words).into()
}
