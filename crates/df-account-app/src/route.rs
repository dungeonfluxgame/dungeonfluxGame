//! The app's URLs: the only place a path is parsed or built.
//!
//! Emailed tokens arrive in the fragment (`#token=`, never sent to a server or a `Referer`) or, from
//! older links, the query. `path` never writes a token back, so the app replaces the address with
//! the token-free path before it posts the token.

/// Where every app path lives (B mounts the bundle here).
pub const APP_BASE: &str = "/app";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Route {
    Landing,
    SignUp,
    SignIn,
    VerifyEmail {
        token: Option<String>,
    },
    ForgotPassword,
    ResetPassword {
        token: Option<String>,
    },
    SignedOut,
    Dashboard,
    GoogleWelcome,
    AddCredit,
    CreditReturn {
        checkout: Option<String>,
    },
    Funding,
    Statements,
    PlaySessions,
    Assets,
    Settings,
    DeveloperTokens,
    Share {
        session_id: String,
    },
    Security,
    Campaign {
        campaign_id: String,
    },
    Lobby {
        session_id: String,
    },
    /// The pre-game launcher: opens the started session's table screen and player screen.
    Play {
        session_id: String,
    },
    /// An invitation link (token in the fragment) or, without a token, the room-code form.
    Join {
        token: Option<String>,
    },

    Admin(AdminRoute),
    NotFound,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdminRoute {
    Summary,
    Accounts,
    Account { account_id: String },
    Disputes,
    Reconciliation,
    Providers,
    Review,
    Audit,
}

/// Who a route is for; the server decides access, this only picks the screen to show.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Audience {
    Anyone,
    /// Screens for people who are not signed in (a signed-in person goes to the dashboard).
    SignedOut,
    Customer,
    Operator,
}

impl Route {
    /// Parses a location's path, query (without `?`) and fragment (without `#`).
    pub fn parse(path: &str, query: &str, fragment: &str) -> Self {
        let Some(rest) = path.strip_prefix(APP_BASE) else {
            return Self::NotFound;
        };
        let rest = rest.trim_matches('/');
        let token = || parameter(fragment, "token").or_else(|| parameter(query, "token"));
        match rest.split('/').collect::<Vec<_>>().as_slice() {
            [""] => Self::Landing,
            ["sign-up"] => Self::SignUp,
            ["sign-in"] => Self::SignIn,
            ["verify-email"] => Self::VerifyEmail { token: token() },
            ["forgot-password"] => Self::ForgotPassword,
            ["reset-password"] => Self::ResetPassword { token: token() },
            ["signed-out"] => Self::SignedOut,
            ["dashboard"] if parameter(query, "welcome").as_deref() == Some("1") => {
                Self::GoogleWelcome
            }
            ["dashboard"] | ["campaigns"] => Self::Dashboard,
            ["credit"] => Self::AddCredit,
            ["credit", "return"] => Self::CreditReturn {
                checkout: parameter(query, "checkout"),
            },
            ["funding"] => Self::Funding,
            ["statements"] => Self::Statements,
            ["sessions"] => Self::PlaySessions,
            ["assets"] => Self::Assets,
            ["settings"] => Self::Settings,
            ["settings", "developer-tokens"] => Self::DeveloperTokens,
            ["sessions", id, "share"] if is_identifier(id) => Self::Share {
                session_id: (*id).to_owned(),
            },
            ["security"] => Self::Security,
            ["campaigns", "new"] => Self::Dashboard,
            ["campaigns", id] if is_identifier(id) => Self::Campaign {
                campaign_id: (*id).to_owned(),
            },
            ["sessions", id, "lobby"] if is_identifier(id) => Self::Lobby {
                session_id: (*id).to_owned(),
            },
            ["sessions", id, "play"] if is_identifier(id) => Self::Play {
                session_id: (*id).to_owned(),
            },
            ["join"] => Self::Join { token: token() },
            ["display"] => Self::NotFound,
            ["admin"] => Self::Admin(AdminRoute::Summary),
            ["admin", "accounts"] => Self::Admin(AdminRoute::Accounts),
            ["admin", "accounts", id] if is_identifier(id) => Self::Admin(AdminRoute::Account {
                account_id: (*id).to_owned(),
            }),
            ["admin", "disputes"] => Self::Admin(AdminRoute::Disputes),
            ["admin", "reconciliation"] => Self::Admin(AdminRoute::Reconciliation),
            ["admin", "providers"] => Self::Admin(AdminRoute::Providers),
            ["admin", "review"] => Self::Admin(AdminRoute::Review),
            ["admin", "audit"] => Self::Admin(AdminRoute::Audit),
            _ => Self::NotFound,
        }
    }

    /// The address bar form of this route. Tokens are never written back.
    pub fn path(&self) -> String {
        let tail = match self {
            Self::Landing | Self::NotFound => "",
            Self::SignUp => "sign-up",
            Self::SignIn => "sign-in",
            Self::VerifyEmail { .. } => "verify-email",
            Self::ForgotPassword => "forgot-password",
            Self::ResetPassword { .. } => "reset-password",
            Self::SignedOut => "signed-out",
            Self::Dashboard | Self::GoogleWelcome => "dashboard",
            Self::AddCredit => "credit",
            Self::CreditReturn { checkout: Some(id) } => {
                return format!("{APP_BASE}/credit/return?checkout={id}");
            }
            Self::CreditReturn { checkout: None } => "credit/return",
            Self::Funding => "funding",
            Self::Statements => "statements",
            Self::PlaySessions => "sessions",
            Self::Assets => "assets",
            Self::Settings => "settings",
            Self::DeveloperTokens => "settings/developer-tokens",
            Self::Share { session_id } => return format!("{APP_BASE}/sessions/{session_id}/share"),
            Self::Security => "security",
            Self::Campaign { campaign_id } => return format!("{APP_BASE}/campaigns/{campaign_id}"),
            Self::Lobby { session_id } => return format!("{APP_BASE}/sessions/{session_id}/lobby"),
            Self::Play { session_id } => return format!("{APP_BASE}/sessions/{session_id}/play"),
            Self::Join { .. } => "join",
            Self::Admin(admin) => return format!("{APP_BASE}/admin{}", admin.tail()),
        };
        if tail.is_empty() {
            format!("{APP_BASE}/")
        } else {
            format!("{APP_BASE}/{tail}")
        }
    }

    /// A fixed name for log lines: never the path, which can carry an id, and never a token.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Landing => "landing",
            Self::SignUp => "sign_up",
            Self::SignIn => "sign_in",
            Self::VerifyEmail { .. } => "verify_email",
            Self::ForgotPassword => "forgot_password",
            Self::ResetPassword { .. } => "reset_password",
            Self::SignedOut => "signed_out",
            Self::Dashboard | Self::GoogleWelcome => "dashboard",
            Self::AddCredit => "add_credit",
            Self::CreditReturn { .. } => "credit_return",
            Self::Funding => "funding",
            Self::Statements => "statements",
            Self::PlaySessions => "play_sessions",
            Self::Assets => "assets",
            Self::Settings => "settings",
            Self::Security => "security",
            Self::DeveloperTokens => "developer_tokens",
            Self::Share { .. } => "share",
            Self::Campaign { .. } => "campaign",
            Self::Lobby { .. } => "lobby",
            Self::Play { .. } => "play",
            Self::Join { .. } => "join",
            Self::Admin(_) => "admin",
            Self::NotFound => "not_found",
        }
    }

    /// Whether the address bar holds a secret that must be removed before anything else runs.
    pub fn carries_token(&self) -> bool {
        matches!(
            self,
            Self::VerifyEmail { token: Some(_) }
                | Self::ResetPassword { token: Some(_) }
                | Self::Join { token: Some(_) }
        )
    }

    pub fn audience(&self) -> Audience {
        match self {
            Self::Landing
            | Self::VerifyEmail { .. }
            | Self::ResetPassword { .. }
            | Self::Join { .. }
            | Self::SignedOut
            | Self::NotFound => Audience::Anyone,
            Self::SignUp | Self::SignIn | Self::ForgotPassword => Audience::SignedOut,
            Self::Dashboard
            | Self::GoogleWelcome
            | Self::AddCredit
            | Self::CreditReturn { .. }
            | Self::Funding
            | Self::Statements
            | Self::PlaySessions
            | Self::Assets
            | Self::Settings
            | Self::DeveloperTokens
            | Self::Share { .. }
            | Self::Security
            | Self::Campaign { .. }
            | Self::Lobby { .. }
            | Self::Play { .. } => Audience::Customer,
            Self::Admin(_) => Audience::Operator,
        }
    }
}

impl AdminRoute {
    fn tail(&self) -> String {
        match self {
            Self::Summary => String::new(),
            Self::Accounts => "/accounts".to_owned(),
            Self::Account { account_id } => format!("/accounts/{account_id}"),
            Self::Disputes => "/disputes".to_owned(),
            Self::Reconciliation => "/reconciliation".to_owned(),
            Self::Providers => "/providers".to_owned(),
            Self::Review => "/review".to_owned(),
            Self::Audit => "/audit".to_owned(),
        }
    }
}

/// Server ids and tokens are lowercase hex; anything else in a path segment is not ours.
pub(crate) fn is_identifier(text: &str) -> bool {
    !text.is_empty() && text.len() <= 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// One `key=value` from a `&`-separated list, accepted only when the value is an identifier.
fn parameter(list: &str, key: &str) -> Option<String> {
    list.split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(name, _)| *name == key)
        .map(|(_, value)| value)
        .filter(|value| is_identifier(value))
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_route_survives_a_round_trip_through_its_path() {
        let routes = [
            Route::Landing,
            Route::SignUp,
            Route::SignIn,
            Route::VerifyEmail { token: None },
            Route::ForgotPassword,
            Route::ResetPassword { token: None },
            Route::SignedOut,
            Route::Dashboard,
            Route::AddCredit,
            Route::CreditReturn {
                checkout: Some("0a1b".to_owned()),
            },
            Route::Funding,
            Route::Statements,
            Route::PlaySessions,
            Route::Assets,
            Route::Settings,
            Route::Security,
            Route::Campaign {
                campaign_id: "0c".to_owned(),
            },
            Route::Lobby {
                session_id: "5e".to_owned(),
            },
            Route::Join { token: None },
            Route::Admin(AdminRoute::Summary),
            Route::Admin(AdminRoute::Accounts),
            Route::Admin(AdminRoute::Account {
                account_id: "00ff".to_owned(),
            }),
            Route::Admin(AdminRoute::Disputes),
            Route::Admin(AdminRoute::Reconciliation),
            Route::Admin(AdminRoute::Providers),
            Route::Admin(AdminRoute::Review),
            Route::Admin(AdminRoute::Audit),
        ];
        for route in routes {
            let path = route.path();
            let (path_only, query) = path.split_once('?').unwrap_or((&path, ""));
            assert_eq!(Route::parse(path_only, query, ""), route, "{path}");
        }
    }

    #[test]
    fn old_campaign_creation_links_open_the_dashboard() {
        assert_eq!(Route::parse("/app/campaigns/new", "", ""), Route::Dashboard);
    }

    #[test]
    fn a_fragment_token_wins_over_a_query_token_and_is_never_written_back() {
        let route = Route::parse("/app/verify-email", "token=aa", "token=bb");
        assert_eq!(
            route,
            Route::VerifyEmail {
                token: Some("bb".to_owned())
            }
        );
        assert!(route.carries_token());
        assert_eq!(route.path(), "/app/verify-email");
    }

    #[test]
    fn a_token_or_id_that_is_not_hex_is_ignored() {
        assert_eq!(
            Route::parse("/app/reset-password", "", "token=<script>"),
            Route::ResetPassword { token: None }
        );
        assert_eq!(
            Route::parse("/app/admin/accounts/../../x", "", ""),
            Route::NotFound
        );
    }

    #[test]
    fn paths_outside_the_app_or_unknown_inside_it_are_not_found() {
        assert_eq!(Route::parse("/account", "", ""), Route::NotFound);
        assert_eq!(Route::parse("/app/nowhere", "", ""), Route::NotFound);
        assert_eq!(Route::parse("/app", "", ""), Route::Landing);
        assert_eq!(Route::parse("/app/dashboard/", "", ""), Route::Dashboard);
    }
}
