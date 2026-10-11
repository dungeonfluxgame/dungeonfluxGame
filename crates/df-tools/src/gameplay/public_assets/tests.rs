use super::*;
use axum::{
    body::to_bytes,
    http::{HeaderValue, Request},
};
use futures::StreamExt;
use tonic::codegen::Service;

fn fixture(inn: Option<Bytes>) -> Router {
    router(PublicAssets {
        art: Bytes::from_static(b"0123456789"),
        campfire: Bytes::from_static(b"campfire"),
        portrait: Bytes::from_static(b"portrait"),
        inn,
    })
}

async fn request(router: &mut Router, method: Method, path: &str, headers: HeaderMap) -> Response {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .unwrap();
    *request.headers_mut() = headers;
    Service::call(router, request).await.unwrap()
}

fn range(value: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::RANGE, value.parse().unwrap());
    headers
}

async fn etag(router: &mut Router, path: &str) -> HeaderValue {
    let response = request(router, Method::GET, path, HeaderMap::new()).await;
    assert_eq!(response.status(), StatusCode::OK);
    response.headers()[header::ETAG].clone()
}

fn if_none_match(value: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::IF_NONE_MATCH, value.parse().unwrap());
    headers
}

#[tokio::test]
async fn allowlisted_routes_return_exact_public_bytes_mime_and_length() {
    let mut router = fixture(Some(Bytes::from_static(b"inn")));
    for (path, mime, expected) in [
        (ART_PATH, "image/png", b"0123456789".as_slice()),
        (CAMPFIRE_PATH, "image/webp", b"campfire".as_slice()),
        (PORTRAIT_PATH, "image/webp", b"portrait".as_slice()),
        (INN_PATH, "image/webp", b"inn".as_slice()),
    ] {
        let response = request(&mut router, Method::GET, path, HeaderMap::new()).await;
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(response.headers()[header::CONTENT_TYPE], mime);
        assert_eq!(response.headers()[header::ACCEPT_RANGES], "bytes");
        assert_eq!(
            response.headers()[header::CONTENT_LENGTH],
            expected.len().to_string()
        );
        assert!(!response.headers().contains_key(header::CONTENT_RANGE));
        assert_eq!(to_bytes(response.into_body(), 10).await.unwrap(), expected);
    }
}

#[tokio::test]
async fn get_ranges_deliver_only_selected_bytes_with_exact_interval_headers() {
    let mut router = fixture(Some(Bytes::from_static(b"inn")));
    for (range_value, expected, interval) in [
        ("bytes=0-0", "0", "bytes 0-0/10"),
        ("bytes=2-5", "2345", "bytes 2-5/10"),
        ("bytes=7-999", "789", "bytes 7-9/10"),
        ("bytes=7-", "789", "bytes 7-9/10"),
        ("bytes=-3", "789", "bytes 7-9/10"),
        ("bytes=-20", "0123456789", "bytes 0-9/10"),
        ("bytes=0002-0005", "2345", "bytes 2-5/10"),
        ("Bytes=2-5", "2345", "bytes 2-5/10"),
        (
            "bytes=2-99999999999999999999999999999",
            "23456789",
            "bytes 2-9/10",
        ),
        (
            "bytes=-99999999999999999999999999999",
            "0123456789",
            "bytes 0-9/10",
        ),
    ] {
        let response = request(&mut router, Method::GET, ART_PATH, range(range_value)).await;
        assert_eq!(
            response.status(),
            StatusCode::PARTIAL_CONTENT,
            "{range_value}"
        );
        assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
        assert_eq!(response.headers()[header::ACCEPT_RANGES], "bytes");
        assert_eq!(response.headers()[header::CONTENT_RANGE], interval);
        assert_eq!(
            response.headers()[header::CONTENT_LENGTH],
            expected.len().to_string()
        );
        assert_eq!(
            to_bytes(response.into_body(), 10).await.unwrap(),
            expected.as_bytes()
        );
    }
    // The actual concept-art handlers also accept ranges, including the optional inn.
    for path in [CAMPFIRE_PATH, PORTRAIT_PATH, INN_PATH] {
        let response = request(&mut router, Method::GET, path, range("bytes=1-1")).await;
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "image/webp");
        let expected = match path {
            INN_PATH => b"n",
            PORTRAIT_PATH => b"o",
            _ => b"a",
        };
        assert_eq!(
            to_bytes(response.into_body(), 1).await.unwrap(),
            expected.as_slice()
        );
    }
}

#[tokio::test]
async fn unsatisfiable_ranges_return_416_and_public_representation_length() {
    let mut router = fixture(Some(Bytes::new()));
    for range_value in [
        "bytes=10-",
        "bytes=11-30",
        "bytes=-0",
        "bytes=999999999999999999999999-",
        "bytes=0-0",
    ] {
        let path = if range_value == "bytes=0-0" {
            INN_PATH
        } else {
            ART_PATH
        };
        let length = if path == INN_PATH { 0 } else { 10 };
        let response = request(&mut router, Method::GET, path, range(range_value)).await;
        assert_eq!(
            response.status(),
            StatusCode::RANGE_NOT_SATISFIABLE,
            "{range_value}"
        );
        assert_eq!(
            response.headers()[header::CONTENT_RANGE],
            format!("bytes */{length}")
        );
        assert_eq!(response.headers()[header::CONTENT_LENGTH], "0");
        assert!(to_bytes(response.into_body(), 0).await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn invalid_unsupported_and_unvalidated_conditional_ranges_use_full_get() {
    let mut router = fixture(None);
    for range_value in [
        "bytes=5-2",
        "bytes=",
        "bytes=-",
        "bytes=+1-2",
        "bytes=1 -2",
        "bytes=0-a",
        "items=0-1",
        "bytes=0-1,3-4",
        "bytes=0-1,",
        "bytes=0-1-2",
        "bytes=999999999999999999999999-99999999999999999999999",
    ] {
        let response = request(&mut router, Method::GET, ART_PATH, range(range_value)).await;
        assert_eq!(response.status(), StatusCode::OK, "{range_value}");
        assert!(!response.headers().contains_key(header::CONTENT_RANGE));
        assert_eq!(
            to_bytes(response.into_body(), 10).await.unwrap(),
            b"0123456789".as_slice()
        );
    }
    let mut repeated = range("bytes=0-1");
    repeated.append(header::RANGE, "bytes=3-4".parse().unwrap());
    let mut conditional = range("bytes=0-1");
    conditional.insert(header::IF_RANGE, "\"unknown-version\"".parse().unwrap());
    for headers in [
        repeated,
        conditional,
        range(&format!("bytes=0-{}", "9".repeat(129))),
    ] {
        let response = request(&mut router, Method::GET, ART_PATH, headers).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(!response.headers().contains_key(header::CONTENT_RANGE));
        assert_eq!(
            to_bytes(response.into_body(), 10).await.unwrap(),
            b"0123456789".as_slice()
        );
    }
}

#[tokio::test]
async fn head_ignores_range_and_describes_full_image_without_bytes() {
    let mut router = fixture(Some(Bytes::from_static(b"inn")));
    for (path, mime, length) in [
        (ART_PATH, "image/png", "10"),
        (CAMPFIRE_PATH, "image/webp", "8"),
        (PORTRAIT_PATH, "image/webp", "8"),
        (INN_PATH, "image/webp", "3"),
    ] {
        for headers in [HeaderMap::new(), range("bytes=0-0"), range("bytes=999-")] {
            let response = request(&mut router, Method::HEAD, path, headers).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CONTENT_TYPE], mime);
            assert_eq!(response.headers()[header::CONTENT_LENGTH], length);
            assert!(!response.headers().contains_key(header::CONTENT_RANGE));
            assert!(to_bytes(response.into_body(), 0).await.unwrap().is_empty());
        }
    }
}

#[tokio::test]
async fn missing_optional_inn_preserves_redirect_and_other_paths_cannot_select_bytes() {
    let mut router = fixture(None);
    for method in [Method::GET, Method::HEAD] {
        let response = request(&mut router, method, INN_PATH, range("bytes=0-0")).await;
        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(response.headers()[header::LOCATION], ART_PATH);
        assert!(!response.headers().contains_key(header::CONTENT_RANGE));
        assert!(to_bytes(response.into_body(), 0).await.unwrap().is_empty());
    }
    for path in [
        "/assets/private.webp",
        "/assets/concept-art/../private.webp",
        "/assets/concept-art/other.webp",
    ] {
        let response = request(&mut router, Method::GET, path, range("bytes=0-0")).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    let response = request(&mut router, Method::POST, ART_PATH, HeaderMap::new()).await;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn large_get_and_range_bodies_are_poll_driven_bounded_chunks() {
    let bytes = Bytes::from(vec![7; MAX_BODY_CHUNK_BYTES * 2 + 3]);
    let mut router = router(PublicAssets {
        art: bytes.clone(),
        campfire: Bytes::new(),
        portrait: Bytes::new(),
        inn: None,
    });
    let validator = etag(&mut router, ART_PATH).await;
    let mut conditional_range = range("bytes=1-");
    conditional_range.insert(header::IF_RANGE, validator);
    for (headers, expected) in [
        (HeaderMap::new(), bytes.clone()),
        (range("bytes=1-"), bytes.slice(1..)),
        (conditional_range, bytes.slice(1..)),
    ] {
        let response = request(&mut router, Method::GET, ART_PATH, headers).await;
        assert_eq!(
            response.headers()[header::CONTENT_LENGTH],
            expected.len().to_string()
        );
        let mut stream = response.into_body().into_data_stream();
        let mut received = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.unwrap();
            assert!(!chunk.is_empty());
            assert!(chunk.len() <= MAX_BODY_CHUNK_BYTES);
            assert_eq!(
                chunk.as_ptr(),
                expected.as_ptr().wrapping_add(received.len())
            );
            received.extend_from_slice(&chunk);
        }
        assert_eq!(received.as_slice(), expected.as_ref());
    }
    let response = request(&mut router, Method::GET, ART_PATH, HeaderMap::new()).await;
    let mut stream = response.into_body().into_data_stream();
    assert_eq!(
        stream.next().await.unwrap().unwrap().len(),
        MAX_BODY_CHUNK_BYTES
    );
    drop(stream);
    let response = request(&mut router, Method::GET, ART_PATH, range("bytes=-3")).await;
    assert_eq!(
        to_bytes(response.into_body(), 3).await.unwrap(),
        bytes.slice(bytes.len() - 3..)
    );
}

#[tokio::test]
async fn etags_bind_exact_immutable_snapshot_bytes_and_change_with_the_selected_snapshot() {
    let assets = PublicAssets {
        art: Bytes::from_static(b"abc"),
        campfire: Bytes::new(),
        portrait: Bytes::from_static(b"abc"),
        inn: Some(Bytes::new()),
    };
    let mut original = router(assets.clone());
    let validator = etag(&mut original, ART_PATH).await;
    assert_eq!(
        validator,
        "\"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\""
    );
    assert_eq!(etag(&mut original, PORTRAIT_PATH).await, validator);
    assert_eq!(
        etag(&mut original, INN_PATH).await,
        "\"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\""
    );
    let empty = request(&mut original, Method::GET, INN_PATH, if_none_match("*")).await;
    assert_eq!(empty.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(empty.headers()[header::CONTENT_LENGTH], "0");
    assert!(to_bytes(empty.into_body(), 0).await.unwrap().is_empty());
    let head = request(&mut original, Method::HEAD, ART_PATH, HeaderMap::new()).await;
    assert_eq!(head.headers()[header::ETAG], validator);
    assert!(to_bytes(head.into_body(), 0).await.unwrap().is_empty());

    let mut replacement = router(PublicAssets {
        art: Bytes::from_static(b"abd"),
        ..assets
    });
    let changed = request(
        &mut replacement,
        Method::GET,
        ART_PATH,
        if_none_match(validator.to_str().unwrap()),
    )
    .await;
    assert_eq!(changed.status(), StatusCode::OK);
    assert_ne!(changed.headers()[header::ETAG], validator);
    let replacement_validator = changed.headers()[header::ETAG].clone();
    assert_eq!(
        to_bytes(changed.into_body(), 3).await.unwrap(),
        b"abd".as_slice()
    );
    let mut stale_range = range("bytes=1-1");
    stale_range.insert(header::IF_RANGE, validator.clone());
    let response = request(&mut replacement, Method::GET, ART_PATH, stale_range).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!response.headers().contains_key(header::CONTENT_RANGE));
    assert_eq!(
        to_bytes(response.into_body(), 3).await.unwrap(),
        b"abd".as_slice()
    );
    let mut current_range = range("bytes=1-1");
    current_range.insert(header::IF_RANGE, replacement_validator);
    let response = request(&mut replacement, Method::GET, ART_PATH, current_range).await;
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes 1-1/3");
    assert_eq!(
        to_bytes(response.into_body(), 1).await.unwrap(),
        b"b".as_slice()
    );
    let unchanged = request(
        &mut original,
        Method::GET,
        ART_PATH,
        if_none_match(validator.to_str().unwrap()),
    )
    .await;
    assert_eq!(unchanged.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(unchanged.headers()[header::ETAG], validator);
    assert!(to_bytes(unchanged.into_body(), 0).await.unwrap().is_empty());
}

#[tokio::test]
async fn weak_list_and_star_if_none_match_work_for_get_and_head_on_every_public_route() {
    let mut router = fixture(Some(Bytes::from_static(b"inn")));
    for (path, mime, length) in [
        (ART_PATH, "image/png", "10"),
        (CAMPFIRE_PATH, "image/webp", "8"),
        (PORTRAIT_PATH, "image/webp", "8"),
        (INN_PATH, "image/webp", "3"),
    ] {
        let validator = etag(&mut router, path).await;
        let tag = validator.to_str().unwrap();
        for value in [
            tag.to_owned(),
            format!("W/{tag}"),
            "*".to_owned(),
            " \t*\t ".to_owned(),
            format!("\"stale\", W/{tag}"),
            format!("W/{tag}, \"stale\""),
            format!("\"opaque,with,commas\", {tag}"),
            format!(" , , W/{tag}, , "),
            format!("\"*\", {tag}"),
            format!("\"\", W/{tag}"),
            format!("\"opaque\\tag\", {tag}"),
        ] {
            for method in [Method::GET, Method::HEAD] {
                let response = request(&mut router, method, path, if_none_match(&value)).await;
                assert_eq!(
                    response.status(),
                    StatusCode::NOT_MODIFIED,
                    "{path} {value}"
                );
                assert_eq!(response.headers()[header::ETAG], validator);
                assert_eq!(response.headers()[header::CONTENT_TYPE], mime);
                assert_eq!(response.headers()[header::ACCEPT_RANGES], "bytes");
                assert_eq!(response.headers()[header::CONTENT_LENGTH], length);
                assert!(!response.headers().contains_key(header::CONTENT_RANGE));
                assert!(to_bytes(response.into_body(), 0).await.unwrap().is_empty());
            }
        }
        let mut repeated = if_none_match("\"stale\"");
        repeated.append(header::IF_NONE_MATCH, format!("W/{tag}").parse().unwrap());
        let response = request(&mut router, Method::GET, path, repeated).await;
        assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
        assert!(to_bytes(response.into_body(), 0).await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn malformed_or_oversized_if_none_match_is_ignored_as_a_whole_even_after_a_matching_tag() {
    let mut router = fixture(None);
    let validator = etag(&mut router, ART_PATH).await;
    let tag = validator.to_str().unwrap();
    for value in [
        "unquoted".to_owned(),
        "\"unfinished".to_owned(),
        "\"*\"".to_owned(),
        format!("w/{tag}"),
        format!("W/ {tag}"),
        format!("{tag} trailing"),
        format!("{tag}, W/"),
        format!("{tag}, \"unfinished"),
        format!("*, {tag}"),
        format!("{tag}, *"),
        format!("{tag}, \"bad\ttag\""),
        format!("{} {tag}", " ".repeat(MAX_CONDITIONAL_HEADER_BYTES)),
    ] {
        let response = request(&mut router, Method::GET, ART_PATH, if_none_match(&value)).await;
        assert_eq!(response.status(), StatusCode::OK, "{value}");
        assert_eq!(response.headers()[header::ETAG], validator);
        assert_eq!(response.headers()[header::CONTENT_LENGTH], "10");
        assert_eq!(
            to_bytes(response.into_body(), 10).await.unwrap(),
            b"0123456789".as_slice()
        );
    }
    for (first, second) in [(tag, "bad"), ("bad", tag), (tag, "*"), ("*", tag)] {
        let mut repeated = if_none_match(first);
        repeated.append(header::IF_NONE_MATCH, second.parse().unwrap());
        let response = request(&mut router, Method::GET, ART_PATH, repeated).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(response.into_body(), 10).await.unwrap(),
            b"0123456789".as_slice()
        );
    }
    // Opaque entity tags admit obs-text bytes, even though HeaderValue::to_str
    // cannot decode them. Validating a list does not require lossy conversion.
    let mut value = b"\"opaque-".to_vec();
    value.push(0x80);
    value.extend_from_slice(format!("\", W/{tag}").as_bytes());
    let mut headers = HeaderMap::new();
    headers.insert(
        header::IF_NONE_MATCH,
        HeaderValue::from_bytes(&value).unwrap(),
    );
    let response = request(&mut router, Method::GET, ART_PATH, headers).await;
    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    assert!(to_bytes(response.into_body(), 0).await.unwrap().is_empty());
}

#[tokio::test]
async fn if_range_requires_one_matching_strong_validator_before_selecting_partial_bytes() {
    let mut router = fixture(Some(Bytes::from_static(b"inn")));
    let validator = etag(&mut router, ART_PATH).await;
    let tag = validator.to_str().unwrap();
    for value in [tag.to_owned(), format!(" \t{tag}\t ")] {
        let mut headers = range("bytes=2-5");
        headers.insert(header::IF_RANGE, value.parse().unwrap());
        let response = request(&mut router, Method::GET, ART_PATH, headers).await;
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.headers()[header::ETAG], validator);
        assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes 2-5/10");
        assert_eq!(response.headers()[header::CONTENT_LENGTH], "4");
        assert_eq!(
            to_bytes(response.into_body(), 4).await.unwrap(),
            b"2345".as_slice()
        );
    }
    for value in [
        "\"stale\"".to_owned(),
        format!("W/{tag}"),
        "*".to_owned(),
        "Tue, 06 Oct 2026 00:00:00 GMT".to_owned(),
        format!("{tag}, {tag}"),
        format!("{tag} trailing"),
        format!("{} {tag}", " ".repeat(MAX_CONDITIONAL_HEADER_BYTES)),
    ] {
        let mut headers = range("bytes=2-5");
        headers.insert(header::IF_RANGE, value.parse().unwrap());
        let response = request(&mut router, Method::GET, ART_PATH, headers).await;
        assert_eq!(response.status(), StatusCode::OK, "{value}");
        assert_eq!(response.headers()[header::ETAG], validator);
        assert_eq!(response.headers()[header::CONTENT_LENGTH], "10");
        assert!(!response.headers().contains_key(header::CONTENT_RANGE));
        assert_eq!(
            to_bytes(response.into_body(), 10).await.unwrap(),
            b"0123456789".as_slice()
        );
    }
    let mut repeated = range("bytes=2-5");
    repeated.append(header::IF_RANGE, validator.clone());
    repeated.append(header::IF_RANGE, validator.clone());
    let response = request(&mut router, Method::GET, ART_PATH, repeated).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), 10).await.unwrap(),
        b"0123456789".as_slice()
    );
}

#[tokio::test]
async fn preconditions_precede_range_errors_and_only_matching_if_range_retains_416() {
    let mut router = fixture(None);
    let validator = etag(&mut router, ART_PATH).await;
    let tag = validator.to_str().unwrap();
    for range_value in ["bytes=999-", "bytes=2-5", "corrupt-range"] {
        for method in [Method::GET, Method::HEAD] {
            let mut headers = range(range_value);
            headers.insert(header::IF_NONE_MATCH, format!("W/{tag}").parse().unwrap());
            headers.insert(header::IF_RANGE, "\"stale\"".parse().unwrap());
            let response = request(&mut router, method, ART_PATH, headers).await;
            assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
            assert_eq!(response.headers()[header::ETAG], validator);
            assert_eq!(response.headers()[header::CONTENT_LENGTH], "10");
            assert!(!response.headers().contains_key(header::CONTENT_RANGE));
            assert!(to_bytes(response.into_body(), 0).await.unwrap().is_empty());
        }
    }
    let mut headers = range("bytes=999-");
    headers.insert(header::IF_RANGE, validator.clone());
    headers.insert(header::IF_NONE_MATCH, "\"stale\"".parse().unwrap());
    let response = request(&mut router, Method::GET, ART_PATH, headers).await;
    assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(response.headers()[header::ETAG], validator);
    assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes */10");
    assert_eq!(response.headers()[header::CONTENT_LENGTH], "0");
    assert!(to_bytes(response.into_body(), 0).await.unwrap().is_empty());
    let mut headers = range("bytes=999-");
    headers.insert(header::IF_RANGE, format!("W/{tag}").parse().unwrap());
    let response = request(&mut router, Method::GET, ART_PATH, headers).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!response.headers().contains_key(header::CONTENT_RANGE));
    assert_eq!(
        to_bytes(response.into_body(), 10).await.unwrap(),
        b"0123456789".as_slice()
    );

    let mut headers = range("bytes=2-5");
    headers.insert(header::IF_RANGE, validator.clone());
    headers.insert(
        header::IF_NONE_MATCH,
        format!("{tag} corrupt").parse().unwrap(),
    );
    let response = request(&mut router, Method::GET, ART_PATH, headers).await;
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        to_bytes(response.into_body(), 4).await.unwrap(),
        b"2345".as_slice()
    );
    let mut headers = range("bytes=2-5");
    headers.insert(header::IF_RANGE, validator);
    let response = request(&mut router, Method::HEAD, ART_PATH, headers).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_LENGTH], "10");
    assert!(!response.headers().contains_key(header::CONTENT_RANGE));
    assert!(to_bytes(response.into_body(), 0).await.unwrap().is_empty());
}

#[tokio::test]
async fn absent_optional_route_and_unallowlisted_paths_do_not_gain_conditional_validators() {
    let mut router = fixture(None);
    for method in [Method::GET, Method::HEAD] {
        let response = request(&mut router, method, INN_PATH, if_none_match("*")).await;
        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(response.headers()[header::LOCATION], ART_PATH);
        assert!(!response.headers().contains_key(header::ETAG));
        assert!(!response.headers().contains_key(header::CONTENT_RANGE));
        assert!(to_bytes(response.into_body(), 0).await.unwrap().is_empty());
    }
    let response = request(
        &mut router,
        Method::GET,
        "/assets/private.webp",
        if_none_match("*"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(!response.headers().contains_key(header::ETAG));
    let response = request(&mut router, Method::POST, ART_PATH, if_none_match("*")).await;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert!(!response.headers().contains_key(header::ETAG));
}

#[tokio::test]
async fn if_match_requires_a_strong_list_or_star_match_for_get_and_head() {
    let mut router = fixture(Some(Bytes::from_static(b"inn")));
    for (path, mime, expected) in [
        (ART_PATH, "image/png", b"0123456789".as_slice()),
        (CAMPFIRE_PATH, "image/webp", b"campfire".as_slice()),
        (PORTRAIT_PATH, "image/webp", b"portrait".as_slice()),
        (INN_PATH, "image/webp", b"inn".as_slice()),
    ] {
        let validator = etag(&mut router, path).await;
        let tag = validator.to_str().unwrap();
        for (value, passes) in [
            (tag.to_owned(), true),
            (format!(" \t{tag}\t "), true),
            ("*".to_owned(), true),
            (" \t*\t ".to_owned(), true),
            (format!("\"stale\", {tag}"), true),
            (format!("W/{tag}, {tag}"), true),
            (format!("\"opaque,with,commas\", {tag}"), true),
            (format!(" , , {tag}, , "), true),
            (format!("W/{tag}"), false),
            ("\"stale\"".to_owned(), false),
            (format!("\"stale\", W/{tag}"), false),
            ("\"*\"".to_owned(), false),
            (" , , ".to_owned(), false),
        ] {
            for method in [Method::GET, Method::HEAD] {
                let mut headers = HeaderMap::new();
                headers.insert(header::IF_MATCH, value.parse().unwrap());
                let response = request(&mut router, method.clone(), path, headers).await;
                assert_eq!(
                    response.status(),
                    if passes {
                        StatusCode::OK
                    } else {
                        StatusCode::PRECONDITION_FAILED
                    },
                    "{path} {method} {value}"
                );
                assert_eq!(response.headers()[header::ETAG], validator);
                assert_eq!(response.headers()[header::CONTENT_TYPE], mime);
                assert_eq!(response.headers()[header::ACCEPT_RANGES], "bytes");
                assert_eq!(
                    response.headers()[header::CONTENT_LENGTH],
                    if passes {
                        expected.len().to_string()
                    } else {
                        "0".to_owned()
                    }
                );
                assert!(!response.headers().contains_key(header::CONTENT_RANGE));
                let body = to_bytes(response.into_body(), expected.len())
                    .await
                    .unwrap();
                if passes && method == Method::GET {
                    assert_eq!(body, expected);
                } else {
                    assert!(body.is_empty());
                }
            }
        }
        for method in [Method::GET, Method::HEAD] {
            let mut headers = HeaderMap::new();
            headers.append(header::IF_MATCH, format!("W/{tag}").parse().unwrap());
            headers.append(header::IF_MATCH, validator.clone());
            let response = request(&mut router, method, path, headers).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::ETAG], validator);
        }
    }
    let mut empty = fixture(Some(Bytes::new()));
    let mut headers = HeaderMap::new();
    headers.insert(header::IF_MATCH, "*".parse().unwrap());
    let response = request(&mut empty, Method::GET, INN_PATH, headers).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_LENGTH], "0");
    assert!(to_bytes(response.into_body(), 0).await.unwrap().is_empty());
}

#[tokio::test]
async fn failed_if_match_precedes_if_none_match_and_every_range_outcome() {
    let mut router = fixture(None);
    let validator = etag(&mut router, ART_PATH).await;
    let tag = validator.to_str().unwrap();
    for value in ["\"stale\"".to_owned(), format!("W/{tag}")] {
        for method in [Method::GET, Method::HEAD] {
            for range_value in ["bytes=2-5", "bytes=999-", "corrupt-range"] {
                for reversed in [false, true] {
                    let mut headers = range(range_value);
                    headers.insert(header::IF_RANGE, validator.clone());
                    if reversed {
                        headers.insert(header::IF_NONE_MATCH, "*".parse().unwrap());
                        headers.insert(header::IF_MATCH, value.parse().unwrap());
                    } else {
                        headers.insert(header::IF_MATCH, value.parse().unwrap());
                        headers.insert(header::IF_NONE_MATCH, "*".parse().unwrap());
                    }
                    let response = request(&mut router, method.clone(), ART_PATH, headers).await;
                    assert_eq!(
                        response.status(),
                        StatusCode::PRECONDITION_FAILED,
                        "{method} {value} {range_value}"
                    );
                    assert_eq!(response.headers()[header::ETAG], validator);
                    assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
                    assert_eq!(response.headers()[header::ACCEPT_RANGES], "bytes");
                    assert_eq!(response.headers()[header::CONTENT_LENGTH], "0");
                    assert!(!response.headers().contains_key(header::CONTENT_RANGE));
                    assert!(to_bytes(response.into_body(), 0).await.unwrap().is_empty());
                }
            }
        }
    }
    for value in [tag.to_owned(), "*".to_owned()] {
        for method in [Method::GET, Method::HEAD] {
            let mut headers = range("bytes=999-");
            headers.insert(header::IF_MATCH, value.parse().unwrap());
            headers.insert(header::IF_NONE_MATCH, format!("W/{tag}").parse().unwrap());
            headers.insert(header::IF_RANGE, validator.clone());
            let response = request(&mut router, method.clone(), ART_PATH, headers).await;
            assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
            assert_eq!(response.headers()[header::CONTENT_LENGTH], "10");
            assert!(!response.headers().contains_key(header::CONTENT_RANGE));
            assert!(to_bytes(response.into_body(), 0).await.unwrap().is_empty());

            let mut headers = range("bytes=2-5");
            headers.insert(header::IF_MATCH, value.parse().unwrap());
            headers.insert(header::IF_NONE_MATCH, "\"stale\"".parse().unwrap());
            headers.insert(header::IF_RANGE, validator.clone());
            let response = request(&mut router, method.clone(), ART_PATH, headers).await;
            assert_eq!(response.headers()[header::ETAG], validator);
            if method == Method::GET {
                assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
                assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes 2-5/10");
                assert_eq!(response.headers()[header::CONTENT_LENGTH], "4");
                assert_eq!(
                    to_bytes(response.into_body(), 4).await.unwrap(),
                    b"2345".as_slice()
                );
            } else {
                assert_eq!(response.status(), StatusCode::OK);
                assert_eq!(response.headers()[header::CONTENT_LENGTH], "10");
                assert!(!response.headers().contains_key(header::CONTENT_RANGE));
                assert!(to_bytes(response.into_body(), 0).await.unwrap().is_empty());
            }
        }
    }
}

#[tokio::test]
async fn malformed_or_oversized_if_match_is_ignored_as_a_whole() {
    let mut router = fixture(None);
    let validator = etag(&mut router, ART_PATH).await;
    let tag = validator.to_str().unwrap();
    for value in [
        "unquoted".to_owned(),
        "\"unfinished".to_owned(),
        format!("w/{tag}"),
        format!("W/ {tag}"),
        format!("{tag} trailing"),
        format!("{tag}, W/"),
        format!("{tag}, \"unfinished"),
        format!("*, {tag}"),
        format!("{tag}, *"),
        "\"stale\", corrupt".to_string(),
        format!("{tag}, \"bad\ttag\""),
        format!("{} {tag}", " ".repeat(MAX_CONDITIONAL_HEADER_BYTES)),
    ] {
        for method in [Method::GET, Method::HEAD] {
            let mut headers = HeaderMap::new();
            headers.insert(header::IF_MATCH, value.parse().unwrap());
            let response = request(&mut router, method.clone(), ART_PATH, headers).await;
            assert_eq!(response.status(), StatusCode::OK, "{method} {value}");
            assert_eq!(response.headers()[header::ETAG], validator);
            assert_eq!(response.headers()[header::CONTENT_LENGTH], "10");
            let body = to_bytes(response.into_body(), 10).await.unwrap();
            if method == Method::GET {
                assert_eq!(body, b"0123456789".as_slice());
            } else {
                assert!(body.is_empty());
            }
        }
    }
    for (first, second) in [
        (tag, "bad"),
        ("bad", tag),
        ("\"stale\"", "bad"),
        (tag, "*"),
        ("*", tag),
    ] {
        for method in [Method::GET, Method::HEAD] {
            let mut headers = if_none_match("*");
            headers.append(header::IF_MATCH, first.parse().unwrap());
            headers.append(header::IF_MATCH, second.parse().unwrap());
            let response = request(&mut router, method, ART_PATH, headers).await;
            assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
            assert_eq!(response.headers()[header::CONTENT_LENGTH], "10");
            assert!(to_bytes(response.into_body(), 0).await.unwrap().is_empty());
        }
    }
    // Aggregate field bytes, including separators, are bounded even when every
    // repeated value is independently valid; unbounded weak tags would fail 412.
    for method in [Method::GET, Method::HEAD] {
        let mut headers = if_none_match("*");
        for _ in 0..64 {
            headers.append(header::IF_MATCH, format!("W/{tag}").parse().unwrap());
        }
        let response = request(&mut router, method, ART_PATH, headers).await;
        assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
        assert!(to_bytes(response.into_body(), 0).await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn if_match_does_not_replace_redirect_or_route_failures() {
    let mut router = fixture(None);
    for method in [Method::GET, Method::HEAD] {
        for (path, status) in [
            (INN_PATH, StatusCode::TEMPORARY_REDIRECT),
            ("/assets/private.webp", StatusCode::NOT_FOUND),
        ] {
            let mut headers = if_none_match("*");
            headers.insert(header::IF_MATCH, "\"stale\"".parse().unwrap());
            let response = request(&mut router, method.clone(), path, headers).await;
            assert_eq!(response.status(), status);
            assert!(!response.headers().contains_key(header::ETAG));
            assert!(!response.headers().contains_key(header::CONTENT_RANGE));
            if path == INN_PATH {
                assert_eq!(response.headers()[header::LOCATION], ART_PATH);
            }
        }
    }
}

#[tokio::test]
async fn extra_router_serves_exactly_the_loaded_allowlist() {
    let mut router = extra_router(vec![ExtraAsset {
        url: "/assets/ui/generated/wordmark".to_owned(),
        mime: "image/png",
        bytes: Bytes::from_static(b"wordmark"),
    }]);
    let served = request(
        &mut router,
        Method::GET,
        "/assets/ui/generated/wordmark",
        HeaderMap::new(),
    )
    .await;
    assert_eq!(served.status(), StatusCode::OK);
    assert_eq!(served.headers()[header::CONTENT_TYPE], "image/png");
    assert_eq!(
        to_bytes(served.into_body(), usize::MAX).await.unwrap(),
        "wordmark"
    );
    for path in [
        "/assets/ui/generated/portrait-courier",
        "/assets/ui/generated/wordmark.png",
    ] {
        let missing = request(&mut router, Method::GET, path, HeaderMap::new()).await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND, "{path}");
    }
}
