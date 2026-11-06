use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};

use crate::guard::{GuardError, check};

fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.insert(
            header::HeaderName::from_bytes(name.as_bytes()).expect("a valid header name"),
            HeaderValue::from_str(value).expect("a valid header value"),
        );
    }
    map
}

#[test]
fn a_read_from_a_loopback_host_name_is_allowed_with_or_without_a_port() {
    for host in [
        "127.0.0.1",
        "127.0.0.1:7878",
        "localhost",
        "localhost:9",
        "[::1]",
        "[::1]:7878",
    ] {
        assert_eq!(
            check(&Method::GET, &headers(&[("host", host)])),
            Ok(()),
            "{host}"
        );
    }
}

#[test]
fn a_read_with_another_host_name_or_none_is_refused() {
    for host in [
        "example.com",
        "example.com:7878",
        "127.0.0.1.example.com",
        "localhost.evil.test",
        "10.0.0.5",
    ] {
        assert_eq!(
            check(&Method::GET, &headers(&[("host", host)])),
            Err(GuardError::ForeignHost),
            "{host}"
        );
    }
    assert_eq!(
        check(&Method::GET, &HeaderMap::new()),
        Err(GuardError::ForeignHost)
    );
}

#[test]
fn a_change_must_be_json() {
    let local = [("host", "127.0.0.1")];
    assert_eq!(
        check(&Method::POST, &headers(&local)),
        Err(GuardError::NotJson)
    );
    let form = [
        ("host", "127.0.0.1"),
        ("content-type", "application/x-www-form-urlencoded"),
    ];
    assert_eq!(
        check(&Method::POST, &headers(&form)),
        Err(GuardError::NotJson)
    );
    let json = [("host", "127.0.0.1"), ("content-type", "application/json")];
    assert_eq!(check(&Method::POST, &headers(&json)), Ok(()));
    let json_with_charset = [
        ("host", "localhost"),
        ("content-type", "Application/JSON; charset=utf-8"),
    ];
    assert_eq!(check(&Method::POST, &headers(&json_with_charset)), Ok(()));
}

#[test]
fn a_change_from_a_page_on_another_site_is_refused() {
    let base = [("host", "127.0.0.1"), ("content-type", "application/json")];
    let with = |name: &'static str, value: &'static str| {
        let mut all = base.to_vec();
        all.push((name, value));
        headers(&all)
    };
    assert_eq!(
        check(&Method::POST, &with("origin", "http://127.0.0.1:7878")),
        Ok(())
    );
    assert_eq!(
        check(&Method::POST, &with("origin", "http://localhost:3000")),
        Ok(())
    );
    for origin in [
        "http://evil.example",
        "null",
        "not a url",
        "http://127.0.0.1.evil.example",
    ] {
        assert_eq!(
            check(&Method::POST, &with("origin", origin)),
            Err(GuardError::ForeignOrigin),
            "{origin}"
        );
    }
    assert_eq!(
        check(&Method::POST, &with("sec-fetch-site", "same-origin")),
        Ok(())
    );
    assert_eq!(
        check(&Method::POST, &with("sec-fetch-site", "none")),
        Ok(())
    );
    assert_eq!(
        check(&Method::POST, &with("sec-fetch-site", "cross-site")),
        Err(GuardError::CrossSiteRequest)
    );
    assert_eq!(
        check(&Method::POST, &with("sec-fetch-site", "same-site")),
        Err(GuardError::CrossSiteRequest)
    );
}

#[test]
fn a_refusal_names_its_status_and_says_why() {
    assert_eq!(
        GuardError::NotJson.status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    assert_eq!(GuardError::ForeignHost.status(), StatusCode::FORBIDDEN);
    for error in GuardError::ALL {
        assert!(!error.message().is_empty());
        assert_eq!(error.to_string(), error.message());
    }
}
