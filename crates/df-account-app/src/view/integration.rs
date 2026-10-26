//! Settings token controls and the exact curated share preview, using the existing Rust DOM.
use super::*;
use crate::{
    state::{IntegrationField as F, IntegrationIntent as I, IntegrationScreen, SharingScreen, TokensScreen},
    wire_integration::SecretReveal,
    wire_share::SharePreview,
};
fn action(label: &str, intent: I, disabled: bool) -> Node {
    button(label, Some(Intent::Integration(intent)), ButtonKind::Secondary, disabled)
}
fn field(field: F, label: &str, value: &str, password: bool) -> Node {
    if field == F::Highlights {
        let field = Field::Integration(field);
        return el("div").class("field")
            .child(el("label").attr("for",field.name()).text(label))
            .child(el("textarea").attr("id",field.name()).attr("name",field.name())
                .attr("rows","5").attr("value",value).bind(field)).into();
    }
    text_field(TextField { field: Field::Integration(field),label,value,
        kind: if password {InputKind::Password} else {InputKind::Text},
        autocomplete: if password {"current-password"} else {"off"},hint:None },None)
}
fn reveal(secret: &Option<SecretReveal>) -> Node {
    match secret {
        Some(secret) => el("section").class("section")
            .child(el("h2").text("Shown once"))
            .child(el("p").text("Copy this now. It cannot be retrieved later."))
            .child(el("code").class("secret-reveal").text(&secret.0))
            .child(action("Copy",I::Copy,false)).child(action("Dismiss",I::Dismiss,false)).into(),
        None => el("div").into(),
    }
}
pub(super) fn view(screen: &IntegrationScreen) -> Node {
    match screen { IntegrationScreen::Tokens(s) => tokens(s), IntegrationScreen::Share(s) => sharing(s) }
}
fn tokens(s:&TokensScreen)->Node {
    let mut page=page("Developer tokens",Some("Secrets for your trusted tools. Never put a token in public or browser code. Up to 10 active tokens."))
        .child(reveal(&s.reveal));
    if let Some(error)=s.submit.error(){page=page.child(failed(error));}
    page.child(token_controls(s)).child(remote(&s.rows,"Loading tokens",|rows|{
        if rows.is_empty(){return el("p").text("No tokens yet.").into();}
        el("ul").class("rows").children(rows.iter().map(|r|{
            let used=r.last_used_at_ms.map_or("never".to_owned(),|at|format!("{} UTC",crate::money::date_time(at,0)));
            el("li").child(el("strong").text(&r.label))
                .child(el("p").text(format!("{} | expires {} UTC | last used {used}",r.scopes.join(", "),crate::money::date_time(r.expires_at_ms,0))))
                .child(action(if r.revoked_at_ms.is_some(){"Revoked"}else{"Revoke"},I::RevokeToken(r.token_id.clone()),s.submit.is_sending()||r.revoked_at_ms.is_some())).into()
        })).into()
    })).into()
}
fn token_controls(s:&TokensScreen)->Node {
    let mut controls=el("fieldset").class("form integration-controls").flag("disabled",s.submit.is_sending())
        .child(el("legend").text("Create a token"))
        .child(field(F::Label,"Label",&s.label,false))
        .child(field(F::Expiry,"Expiry in days: 7, 30, 90, 180 or 365",&s.expiry,false))
        .child(field(F::Password,"Confirm password, or sign in again within five minutes",&s.password.0,true))
        .child(el("p").text("Choose exact permissions. Email access also requires account access."));
    for scope in ["account:read","account:email:read","auth_sessions:read","play_sessions:read","funding:read"] {
        controls=controls.child(el("button").attr("type","button")
            .attr("aria-pressed",s.scopes.iter().any(|v|v==scope).to_string()).text(scope)
            .on_click(Intent::Integration(I::ToggleScope(scope.to_owned()))));
    }
    controls.child(action("Create token",I::CreateToken,s.submit.is_sending())).into()
}
fn sharing(s:&SharingScreen)->Node {
    let mut page=page("Share session",Some("Publish only this curated summary. Each included person must consent to this exact preview. Omit anyone who does not consent."))
        .child(reveal(&s.reveal));
    if let Some(error)=s.submit.error(){page=page.child(failed(error));}
    let host=s.preview.ready().is_some_and(|p|p.can_publish);
    if host {page=page.child(draft_controls(s));}
    page=page.child(remote(&s.preview,"Loading preview",|p|preview(s,p)));
    if !host {return page.into();}
    page.child(publish_controls(s)).child(remote(&s.links,"Loading shared links",|links|{
        if links.is_empty(){return el("p").text("No active or recent share links.").into();}
        el("ul").class("rows").children(links.iter().map(|l|el("li")
            .child(el("span").text(format!("Expires {} UTC",crate::money::date_time(l.expires_at_ms,0))))
            .child(action(if l.revoked{"Revoked"}else{"Revoke link"},I::RevokeShare(l.link_id.clone()),l.revoked||s.submit.is_sending())).into())).into()
    })).into()
}
fn draft_controls(s:&SharingScreen)->Node {
    el("fieldset").class("form integration-controls").flag("disabled",s.submit.is_sending())
        .child(el("legend").text("Curate the public summary"))
        .child(field(F::Title,"Public title",&s.title,false))
        .child(field(F::Highlights,"Public highlights (one per line, at most five)",&s.highlights,false))
        .child(action("Save exact preview",I::SavePreview,s.submit.is_sending())).into()
}
fn preview(s:&SharingScreen,p:&SharePreview)->Node {
    let mut card=el("section").class("section").child(el("h2").text(&p.draft.title))
        .child(el("p").text(format!("{} | about {} minutes",p.date,p.duration_minutes)));
    if s.dirty {card=card.child(el("p").text("Save your changes, then review the new exact preview before publishing or consenting."));}
    for highlight in &p.draft.highlights {card=card.child(el("p").text(highlight));}
    for person in &p.draft.people {card=card.child(el("p").text(format!("{}: {}",person.public_name,
        if p.consented_accounts.contains(&person.account_id){"consented"}else{"consent pending"})));}
    if p.can_publish {
        for person in &p.candidates {
            let verb=if p.draft.people.iter().any(|v|v.account_id==person.account_id){"Omit"}else{"Include"};
            card=card.child(action(&format!("{verb} {}",person.public_name),I::Person(person.account_id.clone()),s.submit.is_sending()));
        }
    }
    if p.can_consent && !p.revision.is_empty() {
        card=card.child(action("I consent to this preview",I::Consent(true),s.dirty||s.submit.is_sending()))
            .child(action("Withdraw my consent",I::Consent(false),s.submit.is_sending()));
    }
    card.into()
}
fn publish_controls(s:&SharingScreen)->Node {
    let ready=s.preview.ready().is_some_and(|p|!p.revision.is_empty()&&p.draft.people.len()==p.consented_accounts.len());
    el("fieldset").class("form integration-controls").flag("disabled",s.submit.is_sending())
        .child(el("legend").text("Publish this preview"))
        .child(field(F::ShareExpiry,"Link expiry in days: 7, 30 or 90",&s.expiry,false))
        .child(checkbox(Field::Integration(F::Indexing),"Allow search engine indexing",s.indexing,None))
        .child(checkbox(Field::Integration(F::Embeds),"Allow embeds on HTTPS sites",s.embeds,None))
        .child(action("Create share link",I::CreateShare,s.dirty||s.submit.is_sending()||!ready)).into()
}
