use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use df_providers::{
    CandidateRouteId, DecodedImageFacts, DecoderPolicy, FalQueueObservation, FetchResponseFacts,
    OutputFetchError, ProviderAttemptIdentity, ProviderImageMetadata, ProviderRequestId,
    decode_output, fetch_output, inspect_decoded_image, inspect_dns_answers,
    inspect_fetch_response, prepare_output_fetch_plan,
};
use df_types::OperationId;

fn operation(value: u8) -> OperationId {
    OperationId::from_bytes(&[value; 16]).unwrap()
}
fn request_id() -> ProviderRequestId {
    ProviderRequestId::new("rq_fixture-1").unwrap()
}
fn image(url: &str, content_type: &str, width: u32, height: u32) -> ProviderImageMetadata {
    ProviderImageMetadata {
        url: url.to_owned(),
        content_type: content_type.to_owned(),
        width,
        height,
    }
}
fn attempt(operation: OperationId, request: Option<ProviderRequestId>) -> ProviderAttemptIdentity {
    ProviderAttemptIdentity::new(
        operation,
        CandidateRouteId::FalFluxSchnellDisposableImage,
        request,
    )
}
fn completed(id: ProviderRequestId) -> FalQueueObservation {
    FalQueueObservation::Completed {
        request_id: id,
        inference_time: None,
    }
}

#[test]
fn plan_binds_completed_image_to_exact_attempt_and_stays_inspection_only() {
    let operation_id = operation(1);
    let id = request_id();
    let observation = completed(id.clone());
    let plan = prepare_output_fetch_plan(
        &attempt(operation_id, Some(id.clone())),
        operation_id,
        &id,
        &observation,
        &image(
            "https://cdn.fal.media/output/image.png",
            "image/png",
            1024,
            1024,
        ),
        99 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(plan.operation(), operation_id);
    assert_eq!(
        plan.route(),
        CandidateRouteId::FalFluxSchnellDisposableImage
    );
    assert_eq!(plan.supplier_request_id(), &id);
    assert_eq!(
        plan.fetch_requirements().max_encoded_bytes,
        16 * 1024 * 1024
    );
    assert_eq!(plan.fetch_requirements().max_header_bytes, 16 * 1024);
    assert!(
        plan.fetch_requirements().https_only
            && plan.fetch_requirements().redirects_disabled
            && plan.fetch_requirements().proxy_disabled
    );
    assert!(matches!(
        plan.decoder_policy(),
        DecoderPolicy::DecoderUnavailable(_)
    ));
}

#[test]
fn refuses_incomplete_foreign_attempt_and_malformed_locator() {
    let operation_id = operation(1);
    let foreign = operation(2);
    let id = request_id();
    let observation = completed(id.clone());
    let good = image("https://cdn.fal.media/x.jpg", "image/jpeg", 1, 1);
    assert_eq!(
        prepare_output_fetch_plan(
            &attempt(operation_id, Some(id.clone())),
            operation_id,
            &id,
            &FalQueueObservation::Pending(id.clone()),
            &good,
            100
        )
        .unwrap_err(),
        OutputFetchError::RequestNotCompleted
    );
    assert_eq!(
        prepare_output_fetch_plan(
            &attempt(foreign, Some(id.clone())),
            operation_id,
            &id,
            &observation,
            &good,
            100
        )
        .unwrap_err(),
        OutputFetchError::RequestIdentityMismatch
    );
    for url in [
        "https://cdn.fal.media/x.jpg?token=x",
        "https://cdn.fal.media/x.jpg#f",
        "https://user@cdn.fal.media/x.jpg",
        "https://cdn.fal.media:443/x.jpg",
        "https://fal.media.evil.test/x.jpg",
        "https://evilfal.media/x.jpg",
        "http://cdn.fal.media/x.jpg",
        "https://cdn.fal.media/%2fprivate",
        "https://cdn.fal.media/%GG",
        "https://cdn.fal.media/../x.jpg",
    ] {
        assert_eq!(
            prepare_output_fetch_plan(
                &attempt(operation_id, Some(id.clone())),
                operation_id,
                &id,
                &observation,
                &image(url, "image/jpeg", 1, 1),
                100
            )
            .unwrap_err(),
            OutputFetchError::OutputUrlRefused,
            "{url}"
        );
    }
}

#[test]
fn refuses_missing_foreign_supplier_id_and_wrong_route_before_planning() {
    let operation_id = operation(1);
    let id = request_id();
    let other_id = ProviderRequestId::new("rq_fixture-2").unwrap();
    let observation = completed(id.clone());
    let good = image("https://cdn.fal.media/x.jpg", "image/jpeg", 1, 1);
    for identity in [
        attempt(operation_id, None),
        attempt(operation_id, Some(other_id.clone())),
        ProviderAttemptIdentity::new(
            operation_id,
            CandidateRouteId::ElevenFlashV25Tts,
            Some(id.clone()),
        ),
    ] {
        assert_eq!(
            prepare_output_fetch_plan(&identity, operation_id, &id, &observation, &good, 100)
                .unwrap_err(),
            OutputFetchError::RequestIdentityMismatch
        );
    }
    assert_eq!(
        prepare_output_fetch_plan(
            &attempt(operation_id, Some(id.clone())),
            operation_id,
            &id,
            &completed(other_id),
            &good,
            100,
        )
        .unwrap_err(),
        OutputFetchError::RequestNotCompleted
    );
}

#[test]
fn refuses_invalid_dimensions_mime_and_empty_byte_budget() {
    let operation_id = operation(1);
    let id = request_id();
    let observation = completed(id.clone());
    let identity = attempt(operation_id, Some(id.clone()));
    assert_eq!(
        prepare_output_fetch_plan(
            &identity,
            operation_id,
            &id,
            &observation,
            &image("https://cdn.fal.media/x.jpg", "image/gif", 1, 1),
            100
        )
        .unwrap_err(),
        OutputFetchError::MimeMismatch
    );
    assert_eq!(
        prepare_output_fetch_plan(
            &identity,
            operation_id,
            &id,
            &observation,
            &image("https://cdn.fal.media/x.jpg", "image/jpeg", 8193, 1),
            100
        )
        .unwrap_err(),
        OutputFetchError::DimensionsMismatch
    );
    assert_eq!(
        prepare_output_fetch_plan(
            &identity,
            operation_id,
            &id,
            &observation,
            &image("https://cdn.fal.media/x.jpg", "image/jpeg", 1, 1),
            0
        )
        .unwrap_err(),
        OutputFetchError::BodyTooLarge
    );
}

#[test]
fn dns_inspection_is_finite_all_public_and_performs_no_lookup() {
    assert!(inspect_dns_answers(&[]).is_err());
    assert!(
        inspect_dns_answers(&[
            IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        ])
        .is_err()
    );
    assert!(inspect_dns_answers(&[IpAddr::V6(Ipv6Addr::LOCALHOST)]).is_err());
    let accepted = inspect_dns_answers(&[IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))]).unwrap();
    assert_eq!(
        accepted.candidates(),
        &[IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))]
    );
}

#[test]
fn response_inspection_refuses_unbounded_or_ambiguous_transport_facts() {
    let operation_id = operation(1);
    let id = request_id();
    let plan = prepare_output_fetch_plan(
        &attempt(operation_id, Some(id.clone())),
        operation_id,
        &id,
        &completed(id.clone()),
        &image("https://cdn.fal.media/x.jpg", "image/jpeg", 32, 16),
        1024,
    )
    .unwrap();
    let good = FetchResponseFacts {
        status: 200,
        redirected: false,
        used_proxy: false,
        cancelled: false,
        header_bytes: 100,
        content_type: "image/jpeg",
        body_bytes: 512,
        body_complete: true,
        connect_elapsed_seconds: 1,
        elapsed_seconds: 4,
    };
    assert_eq!(inspect_fetch_response(&plan, good), Ok(()));
    assert_eq!(
        inspect_fetch_response(
            &plan,
            FetchResponseFacts {
                cancelled: true,
                ..good
            }
        ),
        Err(OutputFetchError::Cancelled)
    );
    assert_eq!(
        inspect_fetch_response(
            &plan,
            FetchResponseFacts {
                used_proxy: true,
                ..good
            }
        ),
        Err(OutputFetchError::ProxyRefused)
    );
    assert_eq!(
        inspect_fetch_response(
            &plan,
            FetchResponseFacts {
                connect_elapsed_seconds: 3,
                ..good
            }
        ),
        Err(OutputFetchError::DeadlineExceeded)
    );
    assert_eq!(
        inspect_fetch_response(
            &plan,
            FetchResponseFacts {
                connect_elapsed_seconds: 5,
                ..good
            }
        ),
        Err(OutputFetchError::TransportOutcomeUnknown)
    );
    assert_eq!(
        inspect_fetch_response(
            &plan,
            FetchResponseFacts {
                redirected: true,
                ..good
            }
        ),
        Err(OutputFetchError::RedirectRefused)
    );
    assert_eq!(
        inspect_fetch_response(
            &plan,
            FetchResponseFacts {
                status: 404,
                ..good
            }
        ),
        Err(OutputFetchError::HttpStatusRefused)
    );
    assert_eq!(
        inspect_fetch_response(
            &plan,
            FetchResponseFacts {
                elapsed_seconds: 16,
                ..good
            }
        ),
        Err(OutputFetchError::DeadlineExceeded)
    );
    assert_eq!(
        inspect_fetch_response(
            &plan,
            FetchResponseFacts {
                header_bytes: 16 * 1024 + 1,
                ..good
            }
        ),
        Err(OutputFetchError::HeadersTooLarge)
    );
    assert_eq!(
        inspect_fetch_response(
            &plan,
            FetchResponseFacts {
                body_bytes: 1025,
                ..good
            }
        ),
        Err(OutputFetchError::BodyTooLarge)
    );
    assert_eq!(
        inspect_fetch_response(
            &plan,
            FetchResponseFacts {
                body_complete: false,
                ..good
            }
        ),
        Err(OutputFetchError::TransportOutcomeUnknown)
    );
    assert_eq!(
        inspect_fetch_response(
            &plan,
            FetchResponseFacts {
                content_type: "image/png",
                ..good
            }
        ),
        Err(OutputFetchError::MimeMismatch)
    );
}

#[test]
fn decode_fact_inspection_rejects_mismatch_incomplete_and_unsafe_features() {
    let operation_id = operation(1);
    let id = request_id();
    let plan = prepare_output_fetch_plan(
        &attempt(operation_id, Some(id.clone())),
        operation_id,
        &id,
        &completed(id.clone()),
        &image("https://cdn.fal.media/x.png", "image/png", 32, 16),
        1024,
    )
    .unwrap();
    let good = DecodedImageFacts {
        format: "image/png",
        width: 32,
        height: 16,
        input_bytes: 512,
        consumed_bytes: 512,
        frame_count: 1,
        animated: false,
        unsupported_metadata: false,
        external_references: false,
    };
    assert_eq!(inspect_decoded_image(&plan, good), Ok(()));
    assert_eq!(
        inspect_decoded_image(
            &plan,
            DecodedImageFacts {
                input_bytes: 1025,
                consumed_bytes: 1025,
                ..good
            }
        ),
        Err(OutputFetchError::BodyTooLarge)
    );
    assert_eq!(
        inspect_decoded_image(&plan, DecodedImageFacts { width: 31, ..good }),
        Err(OutputFetchError::DimensionsMismatch)
    );
    assert_eq!(
        inspect_decoded_image(
            &plan,
            DecodedImageFacts {
                consumed_bytes: 511,
                ..good
            }
        ),
        Err(OutputFetchError::DecodeRejected)
    );
    assert_eq!(
        inspect_decoded_image(
            &plan,
            DecodedImageFacts {
                frame_count: 2,
                animated: true,
                ..good
            }
        ),
        Err(OutputFetchError::DecodeRejected)
    );
    assert_eq!(
        inspect_decoded_image(
            &plan,
            DecodedImageFacts {
                unsupported_metadata: true,
                ..good
            }
        ),
        Err(OutputFetchError::DecodeRejected)
    );
    assert_eq!(
        inspect_decoded_image(
            &plan,
            DecodedImageFacts {
                external_references: true,
                ..good
            }
        ),
        Err(OutputFetchError::DecodeRejected)
    );
    assert_eq!(
        fetch_output(&plan),
        Err(OutputFetchError::TransportOutcomeUnknown)
    );
    assert_eq!(
        decode_output(&plan),
        Err(OutputFetchError::DecoderUnavailable)
    );
    assert!(!format!("{plan:?}").contains("cdn.fal.media"));
}

#[test]
fn dns_inspection_denies_special_global_looking_ranges() {
    for address in [
        IpAddr::V4(Ipv4Addr::new(192, 88, 99, 1)),
        IpAddr::V6("2001:2::1".parse::<Ipv6Addr>().unwrap()),
        IpAddr::V6("2001:10::1".parse::<Ipv6Addr>().unwrap()),
        IpAddr::V6("2001:20::1".parse::<Ipv6Addr>().unwrap()),
        IpAddr::V6("2001:100::1".parse::<Ipv6Addr>().unwrap()),
        IpAddr::V6("2002::1".parse::<Ipv6Addr>().unwrap()),
        IpAddr::V6("3fff::1".parse::<Ipv6Addr>().unwrap()),
        IpAddr::V6("::ffff:10.0.0.1".parse::<Ipv6Addr>().unwrap()),
    ] {
        assert_eq!(
            inspect_dns_answers(&[address]),
            Err(OutputFetchError::DnsTargetRefused)
        );
    }
}
