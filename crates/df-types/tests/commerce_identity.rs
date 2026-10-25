use df_types::{IdentityError, PaidInvoiceId, PriceVersion, SubscriptionId, TextIdentityError};

#[test]
fn commercial_roles_reuse_canonical_byte_and_text_validation() {
    let bytes = [1; 16];
    let hex_identity = "01010101010101010101010101010101";
    assert_eq!(
        SubscriptionId::from_bytes(&bytes).unwrap().as_bytes(),
        &bytes
    );
    assert_eq!(
        PaidInvoiceId::from_bytes(&bytes).unwrap().as_bytes(),
        &bytes
    );
    assert_eq!(PriceVersion::from_bytes(&bytes).unwrap().as_bytes(), &bytes);
    assert_eq!(
        SubscriptionId::from_hex(hex_identity).unwrap().as_bytes(),
        &bytes
    );
    assert_eq!(
        PaidInvoiceId::from_hex(hex_identity).unwrap().as_bytes(),
        &bytes
    );
    assert_eq!(
        PriceVersion::from_hex(hex_identity).unwrap().as_bytes(),
        &bytes
    );
    assert_eq!(
        SubscriptionId::from_bytes(&[]),
        Err(IdentityError::InvalidLength { actual: 0 })
    );
    assert_eq!(
        PaidInvoiceId::from_bytes(&[1; 17]),
        Err(IdentityError::InvalidLength { actual: 17 })
    );
    assert_eq!(PriceVersion::from_bytes(&[0; 16]), Err(IdentityError::Zero));
    assert_eq!(
        SubscriptionId::from_hex("00000000000000000000000000000000"),
        Err(TextIdentityError::Zero)
    );
    assert_eq!(
        PaidInvoiceId::from_hex("ABCDEF00000000000000000000000000"),
        Err(TextIdentityError::Malformed)
    );
    assert_eq!(
        PriceVersion::from_hex(" "),
        Err(TextIdentityError::InvalidLength { actual: 1 })
    );
}

#[test]
fn each_commercial_role_preserves_supplied_order_and_refuses_bad_input() {
    let bytes = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
    let hex_identity = "0102030405060708090a0b0c0d0e0f10";
    assert_eq!(
        SubscriptionId::from_hex(hex_identity).unwrap(),
        SubscriptionId::from_bytes(&bytes).unwrap()
    );
    assert_eq!(
        PaidInvoiceId::from_hex(hex_identity).unwrap(),
        PaidInvoiceId::from_bytes(&bytes).unwrap()
    );
    assert_eq!(
        PriceVersion::from_hex(hex_identity).unwrap(),
        PriceVersion::from_bytes(&bytes).unwrap()
    );
    assert_eq!(
        SubscriptionId::from_bytes(&[0; 16]),
        Err(IdentityError::Zero)
    );
    assert_eq!(
        PaidInvoiceId::from_bytes(&[0; 16]),
        Err(IdentityError::Zero)
    );
    assert_eq!(
        PriceVersion::from_bytes(&[]),
        Err(IdentityError::InvalidLength { actual: 0 })
    );
    for invalid_hex_identity in [
        "                                ",
        "GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG",
        "éééééééééééééééé",
    ] {
        assert_eq!(
            SubscriptionId::from_hex(invalid_hex_identity),
            Err(TextIdentityError::Malformed)
        );
        assert_eq!(
            PaidInvoiceId::from_hex(invalid_hex_identity),
            Err(TextIdentityError::Malformed)
        );
        assert_eq!(
            PriceVersion::from_hex(invalid_hex_identity),
            Err(TextIdentityError::Malformed)
        );
    }
}
