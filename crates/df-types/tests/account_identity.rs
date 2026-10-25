use df_types::{
    AccountId, CampaignId, CheckoutId, IdentityError, OperatorId, TenantId, TextIdentityError,
    WebSessionId,
};

const BYTES: [u8; 16] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
const HEX: &str = "0102030405060708090a0b0c0d0e0f10";

fn assert_role_validates<T: std::fmt::Debug + PartialEq>(
    from_bytes: fn(&[u8]) -> Result<T, IdentityError>,
    from_hex: fn(&str) -> Result<T, TextIdentityError>,
    as_bytes: fn(&T) -> [u8; 16],
) {
    let parsed = from_bytes(&BYTES).unwrap();
    assert_eq!(as_bytes(&parsed), BYTES);
    assert_eq!(from_hex(HEX).unwrap(), parsed);
    assert_eq!(from_bytes(&[0; 16]), Err(IdentityError::Zero));
    assert_eq!(
        from_bytes(&[1; 15]),
        Err(IdentityError::InvalidLength { actual: 15 })
    );
    assert_eq!(
        from_hex("0102030405060708090A0B0C0D0E0F10"),
        Err(TextIdentityError::Malformed)
    );
    assert_eq!(
        from_hex("00000000000000000000000000000000"),
        Err(TextIdentityError::Zero)
    );
}

#[test]
fn account_service_roles_reuse_canonical_byte_and_text_validation() {
    assert_role_validates(AccountId::from_bytes, AccountId::from_hex, |id| {
        *id.as_bytes()
    });
    assert_role_validates(TenantId::from_bytes, TenantId::from_hex, |id| {
        *id.as_bytes()
    });
    assert_role_validates(CampaignId::from_bytes, CampaignId::from_hex, |id| {
        *id.as_bytes()
    });
    assert_role_validates(WebSessionId::from_bytes, WebSessionId::from_hex, |id| {
        *id.as_bytes()
    });
    assert_role_validates(CheckoutId::from_bytes, CheckoutId::from_hex, |id| {
        *id.as_bytes()
    });
    assert_role_validates(OperatorId::from_bytes, OperatorId::from_hex, |id| {
        *id.as_bytes()
    });
}
