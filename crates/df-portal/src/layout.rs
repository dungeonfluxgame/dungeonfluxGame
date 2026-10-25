//! Page chrome and form parts shared by every page module (including campaign modules mounted in
//! this crate). Labels are visible text on every control; errors are summarized at the top and
//! linked to their fields (plan 11). Everything returns `Markup`.

use crate::html::{Markup, markup, text};

/// Which navigation the header shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Navigation {
    Visitor,
    SignedIn,
}

fn links(navigation: Navigation) -> Markup {
    match navigation {
        Navigation::Visitor => markup!(
            "<a href=\"/how-it-plays\">How it plays</a><a href=\"/pricing\">Pricing</a>",
            "<a href=\"/faq\">FAQ</a><a class=\"nav-action\" href=\"/login\">Log in</a>",
            "<a class=\"nav-action primary\" href=\"/signup\">Create account</a>"
        ),
        Navigation::SignedIn => markup!(
            "<a href=\"/account\">Account</a><a href=\"/account/security\">Security</a>",
            "<a href=\"/account/sessions\">Sessions</a><a href=\"/account/data\">Your data</a>"
        ),
    }
}

/// A whole page: header, the main content, and footer.
pub fn page(title: &str, navigation: Navigation, main: Markup) -> Markup {
    markup!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n",
        "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>",
        text(title),
        " - DungeonFlux</title>\n<link rel=\"stylesheet\" href=\"/portal/portal.css\">\n",
        "</head>\n<body>\n<a class=\"skip\" href=\"#main\">Skip to content</a>\n",
        "<header class=\"site-header\"><a class=\"brand\" href=\"/\">DungeonFlux</a>",
        "<nav aria-label=\"Main\">",
        links(navigation),
        "</nav></header>\n<main id=\"main\" tabindex=\"-1\">\n",
        main,
        "\n</main>\n<footer class=\"site-footer\"><a href=\"/terms\">Terms (draft)</a>",
        "<a href=\"/privacy\">Privacy (draft)</a></footer>\n</body>\n</html>\n"
    )
}

/// One form field's error, keyed by the field's `name`.
pub struct FieldError {
    pub field: &'static str,
    pub message: &'static str,
}

/// The error summary at the top of a form; each entry links to its field.
pub fn error_summary(errors: &[FieldError]) -> Markup {
    if errors.is_empty() {
        return Markup::default();
    }
    let items = Markup::concat(errors.iter().map(|error| {
        markup!(
            "<li><a href=\"#",
            error.field,
            "\">",
            text(error.message),
            "</a></li>"
        )
    }));
    markup!(
        "<div class=\"error-summary\" role=\"alert\"><h2>Please fix this</h2><ul>",
        items,
        "</ul></div>"
    )
}

/// The style of a notice above the content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NoticeKind {
    Success,
    Warning,
}

pub fn notice(kind: NoticeKind, message: &str) -> Markup {
    let class = match kind {
        NoticeKind::Success => "notice success",
        NoticeKind::Warning => "notice warning",
    };
    markup!(
        "<div class=\"",
        class,
        "\" role=\"status\">",
        text(message),
        "</div>"
    )
}

/// An input's type; the closed set the portal uses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputKind {
    Email,
    Password,
    Text,
}

pub struct Field<'a> {
    pub name: &'static str,
    pub label: &'static str,
    pub kind: InputKind,
    pub autocomplete: &'static str,
    pub value: &'a str,
    pub hint: Option<&'static str>,
    pub required: bool,
}

/// A labelled input. Its hint and error, if any, are announced through `aria-describedby`.
pub fn field(field: &Field<'_>, errors: &[FieldError]) -> Markup {
    let error = errors.iter().find(|error| error.field == field.name);
    let kind = match field.kind {
        InputKind::Email => "email",
        InputKind::Password => "password",
        InputKind::Text => "text",
    };
    let hint = field.hint.map_or_else(Markup::default, |hint| {
        markup!(
            "<p class=\"hint\" id=\"",
            field.name,
            "-hint\">",
            text(hint),
            "</p>"
        )
    });
    let error_text = error.map_or_else(Markup::default, |error| {
        markup!(
            "<p class=\"field-error\" id=\"",
            field.name,
            "-error\">",
            text(error.message),
            "</p>"
        )
    });
    let described = match (field.hint.is_some(), error.is_some()) {
        (false, false) => Markup::default(),
        (true, false) => markup!(" aria-describedby=\"", field.name, "-hint\""),
        (false, true) => markup!(" aria-describedby=\"", field.name, "-error\""),
        (true, true) => markup!(
            " aria-describedby=\"",
            field.name,
            "-hint ",
            field.name,
            "-error\""
        ),
    };
    markup!(
        "<div class=\"field",
        if error.is_some() { " invalid" } else { "" },
        "\"><label for=\"",
        field.name,
        "\">",
        text(field.label),
        "</label>",
        hint,
        error_text,
        "<input id=\"",
        field.name,
        "\" name=\"",
        field.name,
        "\" type=\"",
        kind,
        "\" autocomplete=\"",
        field.autocomplete,
        "\" value=\"",
        text(field.value),
        "\"",
        if field.required { " required" } else { "" },
        if error.is_some() {
            " aria-invalid=\"true\""
        } else {
            ""
        },
        described,
        "></div>"
    )
}

/// The opening of a `POST` form with its CSRF token and, when given, its operation id.
pub fn form_start(action: &'static str, csrf_token: &str, operation: Option<&str>) -> Markup {
    let operation = operation.map_or_else(Markup::default, |operation| {
        markup!(
            "<input type=\"hidden\" name=\"operation\" value=\"",
            text(operation),
            "\">"
        )
    });
    markup!(
        "<form method=\"post\" action=\"",
        action,
        "\" novalidate><input type=\"hidden\" name=\"csrf\" value=\"",
        text(csrf_token),
        "\">",
        operation
    )
}

/// A hidden field.
pub fn hidden(name: &'static str, value: &str) -> Markup {
    markup!(
        "<input type=\"hidden\" name=\"",
        name,
        "\" value=\"",
        text(value),
        "\">"
    )
}

/// What a submit button does, which decides how it looks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ButtonStyle {
    Primary,
    Secondary,
    Danger,
}

pub fn button(label: &'static str, style: ButtonStyle) -> Markup {
    let class = match style {
        ButtonStyle::Primary => "button primary",
        ButtonStyle::Secondary => "button secondary",
        ButtonStyle::Danger => "button danger",
    };
    markup!(
        "<button type=\"submit\" class=\"",
        class,
        "\">",
        text(label),
        "</button>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_field_with_an_error_is_marked_invalid_and_described_by_its_message() {
        let built = field(
            &Field {
                name: "email",
                label: "Email address",
                kind: InputKind::Email,
                autocomplete: "email",
                value: "\"><script>",
                hint: None,
                required: true,
            },
            &[FieldError {
                field: "email",
                message: "Enter an email address like name@example.com.",
            }],
        );
        let markup = built.as_str();
        assert!(markup.contains("aria-invalid=\"true\""));
        assert!(markup.contains("aria-describedby=\"email-error\""));
        assert!(markup.contains("value=\"&quot;&gt;&lt;script&gt;\""));
        assert!(!markup.contains("<script>"));
    }
}
