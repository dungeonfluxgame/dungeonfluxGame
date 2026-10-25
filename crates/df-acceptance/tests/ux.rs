//! Plan section 17, UX-01 to UX-04: the customer-facing flows (plan 9.6 to 9.10).
//!
//! These need real pages in a real browser (desktop and a 375 px phone) against the test-mode
//! backend, so they are blocked scenarios until P, B, S1 and F mount the account, billing and
//! lobby surfaces. The
//! browser run, not a rendering unit test, is the evidence.

use df_acceptance::blocked_scenario;

blocked_scenario! {
    fn ux_01_a_sponsored_player_joins_without_a_card_and_completes_a_supported_session,
    blocked_on: "blocked on S1, P and F API: invitation join, lobby, sponsor allocation (F) and the play path in one hosted journey",
    setup: [
        "host H funds an allocation; player G has a verified account, no card and no credit; invitation link for G",
    ],
    steps: [
        "G opens the link, signs in, picks a character, readies up, and the host starts the session",
        "assert: no payment prompt ever appears for G and the lobby shows No charge until the host starts",
        "play the supported adventure to its end, then end the session",
        "assert: the session settles against H's allocation only, G's statement shows no charge, and G can leave and later resume with the same character",
    ],
    outcome: "Complete supported session",
}

blocked_scenario! {
    fn ux_02_payment_pending_failed_and_succeeded_states_use_accurate_words_and_safe_retries,
    blocked_on: "blocked on B, P and F API: the add-credit and payment return pages, the read-only purchase status with backoff polling (B), and F's PurchaseStatusView",
    setup: [
        "fake gateway scripted to produce, in turn: pending then succeeded, failed authentication, a hard decline, and a duplicate click on Add credit",
    ],
    steps: [
        "run each outcome through the real pages in a browser",
        "assert: the return page says Confirming payment while pending and never shows spendable credit before the webhook is processed",
        "assert: failed authentication offers retry of the same purchase without creating a second order; a decline says the card was declined and no credit was added",
        "assert: a duplicate click creates one purchase, and a refund pending never looks completed",
    ],
    outcome: "Accurate wording and safe retry behavior",
}

blocked_scenario! {
    fn ux_03_the_billing_flow_works_by_keyboard_and_with_a_screen_reader,
    blocked_on: "blocked on P and F API: the billing dashboard, add-credit and refund pages (P) over F's views, in a browser harness with an accessibility tree reader",
    setup: [
        "the five billing pages: dashboard, add credit, payment return, session funding and refund request",
    ],
    steps: [
        "complete each page's primary task using the keyboard alone, in a visible focus order",
        "read each page's accessibility tree",
        "assert: every control has an accessible name and role; statuses (pending, held, refunded) are announced as text, not only by colour",
        "assert: amounts read as currency with the unit, errors are tied to their fields, and contrast and 375 px reflow meet the stated bar",
    ],
    outcome: "All controls and statuses understandable",
}

blocked_scenario! {
    fn ux_04_a_zero_balance_preserves_state_and_keeps_read_only_and_export_available,
    blocked_on: "blocked on F, O1 and P API: session_funding_state FundingRequired with the TableGate pause and the Resume-when-funded prompt (finance design 10, 18.8)",
    setup: [
        "session S Active with a pot reaching $0.00 available while a paid cinematic is already admitted and one prefetch request is pending",
    ],
    steps: [
        "let the pot reach zero",
        "assert: the admitted cinematic completes and is not interrupted, the pending optional request is refused with the optional-pictures-paused message, and no checkout overlay appears on any player's device",
        "assert: at zero available the session reads FundingRequired and admission refuses new paid work (at_zero_available_the_session_reads_funding_required_and_admission_refuses_new_paid_work)",
        "assert: game state is preserved, saving and viewing work, and campaign export succeeds",
        "add funding",
        "assert: the table sees Resume, and resuming continues from the preserved state with no lost decision",
    ],
    outcome: "State preserved and read-only/export remains available",
}
