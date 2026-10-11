use df_providers::{
    FalQueueObservation, FalSubmitObservation, HttpMethod, HttpRequestError, ProviderImageMetadata,
    ProviderRequestId, classify_fal_status, classify_fal_status_response,
    classify_fal_submit_response, elevenlabs_stream_request, fal_result_request,
    fal_status_request, fal_submit_request, validate_elevenlabs_audio, validate_fal_images,
};

#[test]
fn elevenlabs_stream_request_matches_documented_path_headers_and_body() {
    let request = elevenlabs_stream_request("voice/id", "private-key", "Say \"hello\"\n").unwrap();

    assert_eq!(request.method(), HttpMethod::Post);
    assert_eq!(
        request.url(),
        "https://api.elevenlabs.io/v1/text-to-speech/voice%2Fid/stream?output_format=mp3_22050_32"
    );
    assert_eq!(request.header("xi-api-key"), Some("private-key"));
    assert_eq!(request.header("content-type"), Some("application/json"));
    assert_eq!(
        request.body(),
        br#"{"text":"Say \"hello\"\n","model_id":"eleven_flash_v2_5"}"#
    );

    let debug = format!("{request:?}");
    assert!(!debug.contains("private-key"));
    assert!(!debug.contains("Say"));
}

#[test]
fn elevenlabs_stream_request_refuses_empty_required_fields() {
    assert_eq!(
        elevenlabs_stream_request(" ", "key", "text"),
        Err(HttpRequestError::EmptyVoiceId)
    );
    assert_eq!(
        elevenlabs_stream_request("voice", "", "text"),
        Err(HttpRequestError::EmptyCredential)
    );
    assert_eq!(
        elevenlabs_stream_request("voice", "key", " \n"),
        Err(HttpRequestError::EmptyText)
    );
}

#[test]
fn elevenlabs_audio_requires_successful_complete_mp3_and_keeps_reported_headers_separate() {
    let observation = validate_elevenlabs_audio(
        200,
        "audio/mpeg; charset=binary",
        true,
        1536,
        Some("supplier-request-9"),
        Some("trace-7"),
        Some("42"),
    )
    .unwrap();

    assert_eq!(observation.byte_count(), 1536);
    assert_eq!(observation.request_id(), Some("supplier-request-9"));
    assert_eq!(observation.trace_id(), Some("trace-7"));
    assert_eq!(observation.character_cost(), Some("42"));
    let debug = format!("{observation:?}");
    assert!(!debug.contains("supplier-request-9"));
    assert!(!debug.contains("42"));

    assert_eq!(
        validate_elevenlabs_audio(422, "audio/mpeg", true, 10, None, None, None),
        Err(HttpRequestError::UnsuccessfulAudioStatus)
    );
    assert_eq!(
        validate_elevenlabs_audio(200, "application/json", true, 10, None, None, None),
        Err(HttpRequestError::InvalidAudioContentType)
    );
    assert_eq!(
        validate_elevenlabs_audio(200, "audio/mpeg", false, 10, None, None, None),
        Err(HttpRequestError::IncompleteAudioStream)
    );
    assert_eq!(
        validate_elevenlabs_audio(200, "audio/mpeg", true, 0, None, None, None),
        Err(HttpRequestError::EmptyAudioBody)
    );
}

#[test]
fn fal_submit_request_matches_queue_auth_and_selected_model_fields() {
    let request = fal_submit_request("private-key", "A watchtower at dusk").unwrap();

    assert_eq!(request.method(), HttpMethod::Post);
    assert_eq!(request.url(), "https://queue.fal.run/fal-ai/flux/schnell");
    assert_eq!(request.header("Authorization"), Some("Key private-key"));
    assert_eq!(request.header("Content-Type"), Some("application/json"));
    assert_eq!(
        request.body(),
        br#"{"prompt":"A watchtower at dusk","num_inference_steps":4,"image_size":"landscape_4_3","num_images":1,"enable_safety_checker":true,"output_format":"jpeg"}"#
    );

    let debug = format!("{request:?}");
    assert!(!debug.contains("private-key"));
    assert!(!debug.contains("watchtower"));
}

#[test]
fn fal_status_requires_matching_supplier_request_identity_and_preserves_states() {
    let request_id = ProviderRequestId::new("req-123").unwrap();

    assert!(matches!(
        classify_fal_status(&request_id, "req-123", "IN_QUEUE", None, None, None),
        Ok(FalQueueObservation::Pending(_))
    ));
    assert!(matches!(
        classify_fal_status(&request_id, "req-123", "IN_PROGRESS", None, None, None),
        Ok(FalQueueObservation::Pending(_))
    ));
    assert!(matches!(
        classify_fal_status(&request_id, "req-123", "COMPLETED", None, None, Some(0.34)),
        Ok(FalQueueObservation::Completed {
            inference_time: Some(0.34),
            ..
        })
    ));
    let timing =
        classify_fal_status(&request_id, "req-123", "COMPLETED", None, None, Some(0.34)).unwrap();
    assert_eq!(timing.inference_time_seconds(), Some(0.34));
    assert!(!format!("{timing:?}").contains("0.34"));
    for invalid_timing in [-1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(
            classify_fal_status(
                &request_id,
                "req-123",
                "COMPLETED",
                None,
                None,
                Some(invalid_timing),
            ),
            Ok(FalQueueObservation::Unknown(request_id.clone()))
        );
    }
    assert!(matches!(
        classify_fal_status(&request_id, "req-123", "COMPLETED", Some("RunnerError"), Some("failed"), None),
        Ok(FalQueueObservation::Failed { error_type: Some(kind), error: Some(message), .. })
            if kind == "RunnerError" && message == "failed"
    ));
    assert!(matches!(
        classify_fal_status(&request_id, "req-123", "FUTURE_STATUS", None, None, None),
        Ok(FalQueueObservation::Unknown(_))
    ));
    assert_eq!(
        classify_fal_status(&request_id, "other-request", "COMPLETED", None, None, None),
        Ok(FalQueueObservation::Unknown(request_id.clone()))
    );
    assert_eq!(
        classify_fal_status(&request_id, "", "IN_QUEUE", None, None, None),
        Ok(FalQueueObservation::Unknown(request_id.clone()))
    );
    assert_eq!(
        classify_fal_status(
            &request_id,
            "req-123",
            "IN_PROGRESS",
            None,
            Some("unexpected"),
            None
        ),
        Ok(FalQueueObservation::Unknown(request_id.clone()))
    );
    assert_eq!(
        classify_fal_status_response(404, &request_id, None, None, None, None, None),
        Ok(FalQueueObservation::Unknown(request_id.clone()))
    );
    assert_eq!(
        classify_fal_status_response(200, &request_id, None, None, None, None, None),
        Ok(FalQueueObservation::Unknown(request_id))
    );
}

#[test]
fn fal_status_and_result_requests_use_canonical_encoded_urls() {
    let request_id = ProviderRequestId::new("request/with space").unwrap();
    let status = fal_status_request("private-key", &request_id).unwrap();
    let completed = FalQueueObservation::Completed {
        request_id: request_id.clone(),
        inference_time: None,
    };
    let result = fal_result_request("private-key", &request_id, &completed).unwrap();

    assert_eq!(status.method(), HttpMethod::Get);
    assert_eq!(
        status.url(),
        "https://queue.fal.run/fal-ai/flux/schnell/requests/request%2Fwith%20space/status"
    );
    assert_eq!(result.method(), HttpMethod::Get);
    assert_eq!(
        result.url(),
        "https://queue.fal.run/fal-ai/flux/schnell/requests/request%2Fwith%20space/response"
    );
    assert_eq!(status.header("Authorization"), Some("Key private-key"));
    assert_eq!(status.body(), b"");
}

#[test]
fn fal_submit_requires_same_request_documented_status_and_response_urls() {
    let expected = ProviderRequestId::new("request-42").unwrap();
    let observation = classify_fal_submit_response(
        200,
        Some("request-42"),
        Some("https://queue.fal.run/fal-ai/flux/schnell/requests/request-42/status"),
        Some("https://queue.fal.run/fal-ai/flux/schnell/requests/request-42/response"),
    );
    assert_eq!(observation, FalSubmitObservation::Accepted(expected));
    assert_eq!(
        classify_fal_submit_response(200, None, None, None),
        FalSubmitObservation::Unknown
    );
    assert_eq!(
        classify_fal_submit_response(
            200,
            Some("request-42"),
            Some("https://queue.fal.run/fal-ai/flux/schnell/requests/other/status"),
            Some("https://queue.fal.run/fal-ai/flux/schnell/requests/request-42/response"),
        ),
        FalSubmitObservation::Unknown
    );
    assert_eq!(
        classify_fal_submit_response(
            200,
            Some("request-42"),
            Some("https://attacker.example/status"),
            Some("https://queue.fal.run/fal-ai/flux/schnell/requests/request-42/response"),
        ),
        FalSubmitObservation::Unknown
    );
    assert_eq!(
        classify_fal_submit_response(
            503,
            Some("request-42"),
            Some("https://queue.fal.run/fal-ai/flux/schnell/requests/request-42/status"),
            Some("https://queue.fal.run/fal-ai/flux/schnell/requests/request-42/response"),
        ),
        FalSubmitObservation::Unknown
    );
}

#[test]
fn fal_images_require_same_request_completion_and_valid_media_metadata() {
    let request_id = ProviderRequestId::new("req-123").unwrap();
    let pending =
        classify_fal_status(&request_id, "req-123", "IN_PROGRESS", None, None, None).unwrap();
    let completed =
        classify_fal_status(&request_id, "req-123", "COMPLETED", None, None, None).unwrap();
    let image = ProviderImageMetadata {
        url: "https://v3b.fal.media/files/image.jpeg".to_owned(),
        content_type: "image/jpeg".to_owned(),
        width: 1024,
        height: 768,
    };

    assert_eq!(
        validate_fal_images(&pending, &request_id, std::slice::from_ref(&image)),
        Err(HttpRequestError::ResultBeforeCompletion)
    );
    assert_eq!(
        fal_result_request("private-key", &request_id, &pending),
        Err(HttpRequestError::ResultBeforeCompletion)
    );
    assert_eq!(
        validate_fal_images(&completed, &request_id, std::slice::from_ref(&image)),
        Ok(())
    );
    assert_eq!(
        validate_fal_images(&completed, &request_id, &[]),
        Err(HttpRequestError::EmptyImages)
    );

    let invalid_host = ProviderImageMetadata {
        url: "https://fal.media.evil.example/file.jpeg".to_owned(),
        ..image.clone()
    };
    assert_eq!(
        validate_fal_images(&completed, &request_id, &[invalid_host]),
        Err(HttpRequestError::InvalidImageUrl)
    );
    let delimiter_host = ProviderImageMetadata {
        url: "https://evil.example?.fal.media/image.jpeg".to_owned(),
        ..image.clone()
    };
    assert_eq!(
        validate_fal_images(&completed, &request_id, &[delimiter_host]),
        Err(HttpRequestError::InvalidImageUrl)
    );
    let invalid_type = ProviderImageMetadata {
        content_type: "text/html".to_owned(),
        ..image.clone()
    };
    assert_eq!(
        validate_fal_images(&completed, &request_id, &[invalid_type]),
        Err(HttpRequestError::InvalidImageContentType)
    );
    let invalid_dimensions = ProviderImageMetadata { width: 0, ..image };
    assert_eq!(
        validate_fal_images(&completed, &request_id, &[invalid_dimensions]),
        Err(HttpRequestError::InvalidImageDimensions)
    );
}
