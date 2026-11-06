#![cfg(not(target_arch = "wasm32"))]

use df_providers::{
    CandidateRouteId, NativeHttpLimits, NativeHttpRefusal, ProviderFailureClass, TextOutputSchema,
    TextProviderConfig, TextResponseError, TextSchemaLimits, registered_text_provider,
};
use std::{net::SocketAddr, time::Duration};

fn config() -> TextProviderConfig {
    let schema = TextOutputSchema::new(
        "fixture",
        br#"{"type":"object","properties":{},"required":[],"additionalProperties":false}"#,
        TextSchemaLimits {
            maximum_schema_bytes: 1024,
            maximum_depth: 16,
            maximum_nodes: 128,
        },
    )
    .unwrap();
    TextProviderConfig::new("caller-supplied-snapshot", schema, 32).unwrap()
}

fn limits() -> NativeHttpLimits {
    NativeHttpLimits {
        connect_timeout: Duration::from_secs(1),
        read_timeout: Duration::from_secs(1),
        total_timeout: Duration::from_secs(2),
        maximum_request_bytes: 8192,
        maximum_response_bytes: 8192,
        maximum_header_bytes: 1024,
        maximum_chunks: 64,
    }
}

#[test]
fn native_factory_refuses_foreign_route_invalid_credentials_and_unbounded_transport_before_egress()
{
    for route in [
        CandidateRouteId::ElevenFlashV25Tts,
        CandidateRouteId::FalFluxSchnellDisposableImage,
    ] {
        assert!(matches!(
            registered_text_provider(route, config(), "owned-key", limits()),
            Err(TextResponseError::UnregisteredRoute)
        ));
    }
    for credential in ["", "line\r\ninjection", "leading space", "unicode-☃"] {
        assert!(matches!(
            registered_text_provider(
                CandidateRouteId::OpenAiResponsesText,
                config(),
                credential,
                limits()
            ),
            Err(TextResponseError::InvalidCredential)
        ));
    }
    let mut invalid = limits();
    invalid.maximum_response_bytes = 0;
    assert!(matches!(
        registered_text_provider(
            CandidateRouteId::OpenAiResponsesText,
            config(),
            "owned-key",
            invalid
        ),
        Err(TextResponseError::Transport(
            NativeHttpRefusal::InvalidLimits
        ))
    ));
    invalid = limits();
    invalid.total_timeout = df_providers::MAXIMUM_EXCHANGE_TIME + Duration::from_secs(1);
    assert!(matches!(
        registered_text_provider(
            CandidateRouteId::OpenAiResponsesText,
            config(),
            "owned-key",
            invalid
        ),
        Err(TextResponseError::Transport(
            NativeHttpRefusal::InvalidLimits
        ))
    ));
}

#[test]
fn explicit_fixture_policy_accepts_only_literal_localhost_and_nonzero_owned_port() {
    for address in [
        "127.0.0.2:8000",
        "10.0.0.1:8000",
        "0.0.0.0:8000",
        "[::]:8000",
        "127.0.0.1:0",
    ] {
        let provider = registered_text_provider(
            CandidateRouteId::OpenAiResponsesText,
            config(),
            "owned-key",
            limits(),
        )
        .unwrap();
        assert!(matches!(
            provider.for_loopback_fixture(address.parse::<SocketAddr>().unwrap()),
            Err(TextResponseError::Transport(
                NativeHttpRefusal::InvalidEndpoint
            ))
        ));
    }
    for address in ["127.0.0.1:8000", "[::1]:8000"] {
        let provider = registered_text_provider(
            CandidateRouteId::OpenAiResponsesText,
            config(),
            "owned-key",
            limits(),
        )
        .unwrap();
        assert!(
            provider
                .for_loopback_fixture(address.parse().unwrap())
                .is_ok()
        );
    }
}

#[test]
fn canonical_failure_categories_preserve_cancellation_deadline_and_unknown_liability() {
    assert_eq!(
        NativeHttpRefusal::Cancelled.failure_class(),
        ProviderFailureClass::Cancelled
    );
    assert_eq!(
        NativeHttpRefusal::Deadline.failure_class(),
        ProviderFailureClass::Deadline
    );
    assert_eq!(
        NativeHttpRefusal::Unknown.failure_class(),
        ProviderFailureClass::Unknown
    );
    assert_eq!(
        NativeHttpRefusal::ResponseTooLarge.failure_class(),
        ProviderFailureClass::Contract
    );
}
