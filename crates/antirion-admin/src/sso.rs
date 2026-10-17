//! How a failed single sign-on answers. A browser is sent back to the console with a stable code
//! and a short message: `<console>/login?error=<code>&message=<text>`, or, for a test sign-in, the
//! page it was started from (its `return_to`, else the sign-in settings). A client that asks for
//! JSON (`Accept: application/json`, without `text/html`) gets the Admin API error instead.
//!
//! The codes:
//! - `sso_state_invalid`: the sign-in is unknown, used, expired or was started by another browser
//!   (an OIDC or OAuth `state`, or the SAML request a response answers), or the assertion was
//!   used already;
//! - `sso_denied`: the identity provider or the gateway refused this user (cancelled at the
//!   provider, email domain, organization, role mappings, provisioning, user limit);
//! - `sso_response_invalid`: what the identity provider sent does not verify or cannot be read;
//! - `sso_provider_error`: the identity provider could not be reached or answered with an error;
//! - `sso_provider_unknown`: no such identity provider is configured;
//! - `sso_rate_limited`: too many attempts;
//! - `sso_error`: the gateway failed (store, internal error).

use antirion_runtime::Runtime;
use axum::response::Response;
use http::header::ACCEPT;
use http::{HeaderMap, StatusCode};

use crate::error::AdminError;
use crate::oidc::{form_encode, redirect};

/// The longest message carried back, in characters.
const MESSAGE_MAX: usize = 200;

/// Where a failed sign-in sends the browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Landing {
    /// The console's sign-in page.
    SignIn,
    /// A test sign-in: its `return_to` (`/` when none was given).
    Test(String),
}

impl Landing {
    /// The landing of a sign-in started with `test` and `return_to`.
    pub(crate) fn of(test: bool, return_to: &str) -> Landing {
        if test {
            Landing::Test(return_to.to_owned())
        } else {
            Landing::SignIn
        }
    }

    /// The path the browser goes to, without the failure's parameters.
    fn path(&self, ui: &str) -> String {
        match self {
            Landing::SignIn => format!("{ui}/login"),
            Landing::Test(r) if r != "/" => r.clone(),
            Landing::Test(_) => format!("{ui}/settings?section=sign-in"),
        }
    }
}

/// Whether the caller asked for JSON rather than a page.
pub(crate) fn wants_json(headers: &HeaderMap) -> bool {
    let ranges: Vec<String> = headers
        .get_all(ACCEPT)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|r| {
            r.split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
        })
        .collect();
    ranges.iter().any(|r| r == "application/json") && !ranges.iter().any(|r| r == "text/html")
}

/// The stable code a failed sign-in carries back.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    StateInvalid,
    Denied,
    ResponseInvalid,
    ProviderError,
    ProviderUnknown,
    RateLimited,
    Error,
}

impl Failure {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Failure::StateInvalid => "sso_state_invalid",
            Failure::Denied => "sso_denied",
            Failure::ResponseInvalid => "sso_response_invalid",
            Failure::ProviderError => "sso_provider_error",
            Failure::ProviderUnknown => "sso_provider_unknown",
            Failure::RateLimited => "sso_rate_limited",
            Failure::Error => "sso_error",
        }
    }
}

/// The stable code of a failure: the one it was given, else the one its status implies.
pub(crate) fn code(e: &AdminError) -> &'static str {
    let failure = e.sso.unwrap_or(match e.status {
        StatusCode::FORBIDDEN => Failure::Denied,
        StatusCode::NOT_FOUND => Failure::ProviderUnknown,
        StatusCode::TOO_MANY_REQUESTS => Failure::RateLimited,
        StatusCode::BAD_GATEWAY | StatusCode::GATEWAY_TIMEOUT => Failure::ProviderError,
        s if s.is_client_error() => Failure::ResponseInvalid,
        _ => Failure::Error,
    });
    failure.as_str()
}

/// The address a failure sends the browser to.
pub(crate) fn location(ui: &str, landing: &Landing, e: &AdminError) -> String {
    let message: String = e.message.chars().take(MESSAGE_MAX).collect();
    let path = landing.path(ui);
    let (path, fragment) = match path.split_once('#') {
        Some((p, f)) => (p.to_owned(), format!("#{f}")),
        None => (path, String::new()),
    };
    let sep = if path.contains('?') { '&' } else { '?' };
    format!(
        "{path}{sep}error={}&message={}{fragment}",
        code(e),
        form_encode(&message)
    )
}

/// `result`, with a failure turned into a redirect to `landing` unless the caller asked for JSON.
pub(crate) fn settle(
    rt: &Runtime,
    headers: &HeaderMap,
    landing: &Landing,
    result: Result<Response, AdminError>,
) -> Result<Response, AdminError> {
    match result {
        Err(e) if !wants_json(headers) => {
            tracing::info!(
                code = code(&e),
                status = e.status.as_u16(),
                "a single sign-on failed"
            );
            Ok(redirect(&location(&rt.statics.ui.path, landing, &e)))
        }
        other => other,
    }
}

/// The failure an identity provider reported on the callback (`error=…`): a refusal for
/// `access_denied`, a provider error otherwise. Only a plain error code is repeated, never the
/// provider's description.
pub(crate) fn provider_said(error: &str) -> AdminError {
    let plain =
        !error.is_empty() && error.len() <= 64 && error.bytes().all(|b| b.is_ascii_lowercase() || b == b'_');
    let shown = if plain { error } else { "unknown" };
    if error == "access_denied" {
        AdminError::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            "The identity provider did not sign you in (access_denied).",
        )
        .with_sso(Failure::Denied)
    } else {
        AdminError::new(
            StatusCode::BAD_GATEWAY,
            "idp_unavailable",
            format!("The identity provider answered with an error ({shown})."),
        )
        .with_sso(Failure::ProviderError)
    }
}

#[cfg(test)]
mod tests {
    use http::HeaderValue;

    use super::*;
    use crate::oidc::{bad_state, refused, unavailable};

    fn accept(v: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            ACCEPT,
            HeaderValue::from_str(v).unwrap_or_else(|_| unreachable!()),
        );
        h
    }

    #[test]
    fn only_a_client_asking_for_json_gets_json() {
        assert!(wants_json(&accept("application/json")));
        assert!(wants_json(&accept("application/json; charset=utf-8, */*;q=0.1")));
        assert!(!wants_json(&accept(
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
        )));
        assert!(!wants_json(&accept("text/html, application/json")));
        assert!(!wants_json(&accept("*/*")));
        assert!(!wants_json(&HeaderMap::new()));
    }

    #[test]
    fn failures_have_stable_codes() {
        assert_eq!(code(&bad_state()), "sso_state_invalid");
        assert_eq!(code(&refused("This email domain may not sign in.")), "sso_denied");
        assert_eq!(code(&unavailable()), "sso_provider_error");
        assert_eq!(code(&AdminError::not_found()), "sso_provider_unknown");
        assert_eq!(
            code(&AdminError::unauthenticated("The ID token is not valid.")),
            "sso_response_invalid"
        );
        assert_eq!(code(&AdminError::store_unavailable()), "sso_error");
        assert_eq!(code(&provider_said("access_denied")), "sso_denied");
        let other = provider_said("server_error\"><script>");
        assert_eq!(code(&other), "sso_provider_error");
        assert!(other.message.contains("(unknown)"), "{}", other.message);
    }

    #[test]
    fn a_failure_lands_on_the_sign_in_page_or_where_a_test_started() {
        let e = bad_state();
        assert_eq!(
            location("/ui", &Landing::SignIn, &e),
            "/ui/login?error=sso_state_invalid&message=The%20login%20state%20is%20unknown%2C%20used%20or%20expired."
        );
        assert!(location("", &Landing::SignIn, &e).starts_with("/login?error="));
        let test = Landing::of(true, "/");
        assert!(
            location("/ui", &test, &e)
                .starts_with("/ui/settings?section=sign-in&error=sso_state_invalid&message=")
        );
        let back = Landing::of(true, "/ui/settings?section=sign-in#providers");
        assert_eq!(
            location("/ui", &back, &unavailable()),
            "/ui/settings?section=sign-in&error=sso_provider_error\
             &message=The%20identity%20provider%20could%20not%20be%20reached.#providers"
        );
        let long = AdminError::invalid_parameter("x".repeat(500));
        let to = location("/ui", &Landing::SignIn, &long);
        assert_eq!(to.matches('x').count(), MESSAGE_MAX);
    }
}
