DUNGEONFLUX TRANSACTIONAL NOTIFICATION TEMPLATES
================================================

Folder: content/notifications/
Written by: launch agent H3, 2026-10-24, for agent B's email port.
Sources: artifacts/tmp/launch-plan/master-launch-plan.txt sections 6.7, 9.3, 9.8, 9.10, 10 (J01 to J14), 11, 13.11;
planning/account-service-research.md (sections 2, 7 and the decisions on trial credit).

Marketing email is separate consent and is not in this set.


FILE FORMAT
-----------

Each <event>.txt holds four parts, in this order:

  SUBJECT:       one line, sentence case.
  BODY:          paragraphs, one per line, separated by blank lines. Second person. Plain text.
  PLACEHOLDERS:  one line per placeholder: {{name}}: one-line description.
  NOTE:          which plan section requires the message, the research reference, and send rules.

Placeholders use double braces with lowercase names and underscores, for example {{display_name}}.
Files are ASCII, UTF-8, LF line endings. No emoji.

A renderer must fail when a placeholder has no value. Never send a message that still shows a {{...}} marker.
Subject and every other single-line value must have CR and LF removed before rendering (header injection).


VALUE CONVENTIONS
-----------------

Money:         US dollars, a leading $ and two decimal places, for example $20.00. Format from integer cents or
               exact micro-dollars using the rounding rule in plan 6.4. Never format from binary floats.
Times:         the recipient's time zone, for example 24 Oct 2026, 19:05.
Dates:         for example 24 Nov 2026.
*_url:         plain page address. No token and no query string.
*_link:        short-lived, credential-bearing link. Only three exist: verification_link, invite_link,
               download_link. Each one is marked in its template's NOTE.
payment_method_label: card brand and last four digits only, for example "Visa ending 6411".
Campaign names:  shown to members and invitees. Campaign names are not story content.
Email addresses: the recipient's own address only. Show any other address masked, for example a***@e***.test.


WHAT NO TEMPLATE CONTAINS
-------------------------

Full card numbers, bank details, billing addresses, processor IDs or checkout session IDs; passwords or password
reset tokens; session tokens or cookies; provider credentials or keys; signed asset URLs; IP addresses; private
character secrets; story notes; NPC text; other payers' balances.

The three *_link placeholders are the only credentials. Each is short-lived and each is required by the plan:
verification_link (9.3, research 7.1), invite_link (9.6), download_link (13.11, J14).


TEMPLATE LIST
-------------

File                                 Subject (as written)                                      Placeholders
------------------------------------ --------------------------------------------------------- ----------------------------------------------
email-verification.txt               Verify your DungeonFlux email                             display_name, email_address, verification_link,
                                                                                               link_expires_at
security-change.txt                  Security change on your DungeonFlux account               display_name, change_summary, event_time,
                                                                                               device_label, account_security_url,
                                                                                               support_address
campaign-invitation.txt              {{host_display_name}} invited you to {{campaign_name}}    host_display_name, campaign_name,
                                                                                               scheduled_time_line, content_rating,
                                                                                               funding_sentence, invite_link,
                                                                                               link_expires_at, join_url, room_code
payment-confirmed.txt                Your payment is confirmed                                 display_name, purchase_date, receipt_id,
                                                                                               credit_amount, tax_amount,
                                                                                               payment_method_label, total_charged,
                                                                                               available_credit_after, terms_url,
                                                                                               receipt_url
payment-failed.txt                   Your payment did not go through                           display_name, attempted_amount,
                                                                                               failure_reason, add_credit_url
contribution-accepted.txt            Your contribution to {{campaign_name}} is active          display_name, campaign_name, host_display_name,
                                                                                               contribution_amount, payer_hourly_maximum,
                                                                                               payer_session_maximum,
                                                                                               campaign_funding_url
contribution-released.txt            Unused credit released from {{campaign_name}}             display_name, campaign_name, session_name,
                                                                                               session_end_time, released_amount,
                                                                                               unresolved_amount, billing_url
budget-threshold.txt                 Funding for {{campaign_name}} is running low              display_name, campaign_name, pot_available,
                                                                                               pot_held, hourly_maximum, current_action,
                                                                                               funding_url
table-paused-for-funds.txt           {{campaign_name}} is paused until it is funded            display_name, campaign_name, last_save_time,
                                                                                               funding_url
refund-requested.txt                 We received your refund request                           display_name, refund_amount, receipt_id,
                                                                                               released_allocation_amount,
                                                                                               pending_hold_amount,
                                                                                               payment_method_label, refund_reference,
                                                                                               refund_status_url
refund-completed.txt                 Your refund of {{refund_amount}} is complete              display_name, refund_amount, receipt_id,
                                                                                               completed_time, payment_method_label,
                                                                                               refund_reference, available_credit_after
refund-failed.txt                    Your refund of {{refund_amount}} needs review             display_name, refund_amount, receipt_id,
                                                                                               refund_reference, support_address
session-statement-provisional.txt    Your provisional statement for {{session_name}} is ready  display_name, session_name, campaign_name,
                                                                                               active_time, ai_work_summary,
                                                                                               technical_credits, total_consumed,
                                                                                               own_share, released_amount,
                                                                                               unsettled_amount, statement_url
session-statement-final.txt          Your final statement for {{session_name}} is ready        display_name, session_name, campaign_name,
                                                                                               active_time, ai_work_summary,
                                                                                               technical_credits, total_consumed,
                                                                                               own_share, released_amount,
                                                                                               statement_url
export-ready.txt                     Your data export is ready                                 display_name, export_scope, campaign_name, prepared_time,
                                                                                               file_size, download_link,
                                                                                               link_expires_at, account_security_url,
                                                                                               support_address
account-deletion-scheduled.txt       Your DungeonFlux account is scheduled for deletion        display_name, deletion_date,
                                                                                               settlement_status, account_url,
                                                                                               privacy_url, account_security_url
account-deletion-cancelled.txt       Your account deletion is cancelled                        display_name, cancelled_time, cancel_reason,
                                                                                               account_security_url, support_address
account-deletion-completed.txt       Your DungeonFlux account has been deleted                 display_name, deleted_at,
                                                                                               billing_retention_end,
                                                                                               campaign_handover_summary,
                                                                                               support_address
material-incident-notice.txt         Important: {{incident_title}}                             display_name, incident_title, incident_date,
                                                                                               affected_scope, incident_summary,
                                                                                               data_categories, actions_taken,
                                                                                               customer_actions, financial_impact,
                                                                                               next_update_time, support_address
trial-credit-granted.txt             Your trial credit is ready                                display_name, trial_amount, account_page_url

The per-placeholder descriptions are in each file's PLACEHOLDERS block.


PLAN SECTION MAP
----------------

email-verification         9.3, 11                    security-change             9.3, 11
campaign-invitation        9.6, 11, J02               payment-confirmed           9.7, 9.8, 7.2, 11, J01
payment-failed             9.8, 6.3, 11               contribution-accepted       5.3, 5.5, 9.7, 9.9, 11, J03
contribution-released      5.4, 9.11, 11, J07, J12    budget-threshold            9.10, 11, J05
table-paused-for-funds     9.10, 11, J05              refund-requested            6.7, 9.7, 11, J13
refund-completed           6.7, 9.7, 11, J13          refund-failed               6.3, 6.7, 9.7, 11
session-statement-*        7.2, 9.11, 11, J12         export-ready                13.11, 11, J14
account-deletion-*         13.11, 11, J14             material-incident-notice    11, 16
trial-credit-granted       owner requirement, research decisions 19 to 23, 6.5, 9.7


NOT COVERED HERE
----------------

These messages exist in the research or the plan but have no template in this folder:

- Password reset request and confirmation (research 7.3; 15-minute token).
- Email change confirmation links to the old and new addresses (research 7.3; they carry tokens).
- "Someone tried to sign up with your address" and "already registered" messages (research 6.3 and 7.1).
- Unknown-result or pending payment states. Plan 11 requires them, and this set treats them as in-app status only.
- Operator and admin mail, including enrolment links (research section 8 and the operator sections).
- Marketing email (separate consent; out of scope).
- Auto-reload consent and notices (plan 5.6; later release).


OPEN DECISIONS FOR THE COORDINATOR
----------------------------------

1. Budget threshold: plan 11 says in-app first. Confirm when email is sent (see budget-threshold.txt).
2. Paused for funds: confirm the delay before email (see table-paused-for-funds.txt).
3. Trial credit: plan 11 does not list it. Confirm it belongs in the transactional set (see trial-credit-granted.txt).
4. Account deletion cancelled and completed: plan 11 lists only the deletion schedule. Confirm both messages.
5. Trial expiry wording: the research says no expiry for now. Legal terms are a paid-launch gate.
6. Tax wording on receipts and statements ("Not a tax invoice", the tax line): the tax specialist decides (plan 6.7).
7. Which sign-ins count as new for security-change.txt.
8. Link lifetimes: verification 24 hours (research 7.1); export 24 hours (research 7.5); invitation lifetime is not stated in the plan or the research.
9. Material incident notice: counsel approves each incident's wording before send.
