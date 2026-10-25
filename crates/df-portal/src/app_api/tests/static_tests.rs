use axum::http::{StatusCode, header};

use super::fixtures::{Harness, Reply};

const SHELL_MARKER: &str = "<div id=\"app\">";
const EXPECTED_CSP: &str = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; \
    style-src 'self' https://fonts.googleapis.com; font-src https://fonts.gstatic.com; \
    img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; \
    form-action 'self'";

async fn fetch(harness: &Harness, path: &str) -> Reply {
    harness.browser().get(path).await
}

fn assert_hardened(reply: &Reply, path: &str) {
    let headers = &reply.headers;
    assert_eq!(
        headers[header::CONTENT_SECURITY_POLICY],
        EXPECTED_CSP,
        "{path}"
    );
    assert_eq!(headers[header::REFERRER_POLICY], "no-referrer", "{path}");
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff", "{path}");
    assert_eq!(headers[header::X_FRAME_OPTIONS], "DENY", "{path}");
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache", "{path}");
}

#[tokio::test]
async fn the_shell_answers_every_app_page_directly_with_no_redirect_of_any_kind() {
    let harness = Harness::new().await;
    for path in [
        "/app",
        "/app/",
        "/app/join",
        "/app/join/",
        "/app/sign-in",
        "/app/verify-email",
        "/app/reset-password",
        "/app/campaigns/abc/def",
        "/app/dashboard",
        "/app/anything.else",
    ] {
        let reply = fetch(&harness, path).await;
        assert_eq!(reply.status, StatusCode::OK, "{path}");
        assert!(reply.headers.get(header::LOCATION).is_none(), "{path}");
        assert!(reply.body.contains(SHELL_MARKER), "{path}");
        assert_eq!(
            reply.headers[header::CONTENT_TYPE],
            "text/html; charset=utf-8",
            "{path}"
        );
        assert_hardened(&reply, path);
    }
}

#[tokio::test]
async fn assets_are_served_with_their_types_and_the_same_hardening() {
    let harness = Harness::new().await;
    for (path, content_type) in [
        (
            "/app/assets/account_app.js",
            "text/javascript; charset=utf-8",
        ),
        ("/app/assets/account_app_bg.wasm", "application/wasm"),
        ("/app/assets/app.css", "text/css; charset=utf-8"),
        ("/app/assets/hero.webp", "image/webp"),
    ] {
        let reply = fetch(&harness, path).await;
        assert_eq!(reply.status, StatusCode::OK, "{path}");
        assert_eq!(reply.headers[header::CONTENT_TYPE], content_type, "{path}");
        assert_hardened(&reply, path);
    }
}

#[tokio::test]
async fn a_missing_asset_is_not_found_and_never_the_shell() {
    let harness = Harness::new().await;
    for path in [
        "/app/assets/missing.js",
        "/app/assets/",
        "/app/assets/nothing",
        "/app/missing.wasm",
    ] {
        let reply = fetch(&harness, path).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{path}");
        assert!(!reply.body.contains(SHELL_MARKER), "{path}");
    }
}

#[tokio::test]
async fn paths_that_leave_the_bundle_are_never_read() {
    let harness = Harness::new().await;
    for path in [
        "/app/assets/../secret.txt",
        "/app/assets/..%2fsecret.txt",
        "/app/assets/%2e%2e/secret.txt",
        "/app/assets/..%5csecret.txt",
        "/app/assets/a%00b.js",
        "/app/..%2fsecret.txt",
        "/app//etc/passwd",
        "/app/assets//absolute.js",
        "/app/C:/windows/win.ini",
        "/app/assets/%2fetc%2fpasswd",
    ] {
        let reply = fetch(&harness, path).await;
        assert_ne!(reply.status, StatusCode::OK, "{path}");
        assert!(!reply.body.contains("outside the assets"), "{path}");
    }
    let sibling = fetch(&harness, "/app/secret.txt").await;
    assert!(!sibling.body.contains("outside the assets"));
}

#[tokio::test]
async fn an_unknown_api_path_is_a_json_not_found_and_never_the_shell() {
    let harness = Harness::new().await;
    for path in [
        "/app/api/nothing",
        "/app/api",
        "/app/api/credit/unknown/deep",
    ] {
        let reply = fetch(&harness, path).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(reply.code(), "not_found", "{path}");
    }
}

#[tokio::test]
async fn an_unbuilt_bundle_answers_service_unavailable_not_an_empty_page() {
    let harness = Harness::new().await;
    std::fs::remove_file(harness.bundle.0.join("index.html")).unwrap();
    let reply = fetch(&harness, "/app/join").await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(reply.headers.get(header::LOCATION).is_none());
}
