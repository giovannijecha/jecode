use super::*;

const TOKEN: &str = "eyJhbGciOiJSUzI1NiIsImtpZCI6ImZpeHR1cmUiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2F1dGgub3BlbmFpLmNvbSIsImF1ZCI6ImZpeHR1cmUtY2xpZW50Iiwic3ViIjoiZml4dHVyZS1zdWJqZWN0Iiwibm9uY2UiOiJmaXh0dXJlLW5vbmNlIiwiZXhwIjo0MTAyNDQ0ODAwLCJpYXQiOjE3MDAwMDAwMDAsImVtYWlsIjoiZml4dHVyZUBleGFtcGxlLnRlc3QifQ.hJqkvEDMWMQgoJ4D5TVgP0cYt2C5NPuZryEdwKWZElbnR0dbD7PchwCEvTFAHuOYGHcDCzjk39MprkG9ff4s-8pjanuv2ytpDjA2WgUmP8vCerQ4yT_sxkVGSyr40-DHWE6GZpXCTxZo0SFeIsgnOhsJ6xRrZpxEes46DJW9eq7hoZHQxtYSatxHQuCncxRKjRIzda0ZlxrUwp9wOBiFAIGjVT_Mx6_zAH8DeShbSx7MP3jWz9uha-AuSun3-RHB6YSxf1Mdz_v8iKqFm6bfmt0zVc-N_2UjX_kwzJv0a6Pk3Qw-2cT_vn9zIgFkNf5Xe9RYRFuiaZuHe7C7DGe1tg";
const N: &str = "st44QFDcH0WDn1awC3RMz_zDOw8jcN6LfTt0CXawnjgZgz6Xg2nrP1lv56sZS3NZEG220I1uI2YmzH_41BmSaMHjmsuE525CXTX2Au0Q9FjHijfs1z43Esphcu1ZGiQugNAceKBk_ooLGXENDkpuhNNldMeevZQvLL--ZpgTeSD_2U7uJtCBL0WfQ0VMFE1M22ucIRu_CBhczBkxOAd2aY5U0Xz0jnmGFlkk0cULm9IEu_FNdN9_PkPPQB-UbA39ToLh_2Wy5Vf7otfTvTggYqFlACtbp0ylc8mi9YqfQ39wtariHLpfKLOHaTdS8qFQF3W0n8qrSMA7E4xEFSQtvQ";

fn keys() -> Value {
    Value::object([(
        "keys",
        Value::Array(vec![Value::object([
            ("kid", Value::string("fixture")),
            ("kty", Value::string("RSA")),
            ("alg", Value::string("RS256")),
            ("use", Value::string("sig")),
            ("n", Value::string(N)),
            ("e", Value::string("AQAB")),
        ])]),
    )])
}

#[test]
fn signed_id_token_requires_valid_signature_audience_and_nonce() {
    let cancel = Cancellation::default();
    let claims =
        validate_with_keys(TOKEN, "fixture-client", "fixture-nonce", &keys(), &cancel).unwrap();
    assert_eq!(claims.subject, "fixture-subject");
    assert!(
        validate_with_keys(TOKEN, "other-client", "fixture-nonce", &keys(), &cancel)
            .unwrap_err()
            .contains("audience")
    );
    assert!(
        validate_with_keys(TOKEN, "fixture-client", "other-nonce", &keys(), &cancel)
            .unwrap_err()
            .contains("nonce")
    );
    let mut tampered = TOKEN.as_bytes().to_vec();
    let last = tampered.last_mut().unwrap();
    *last = if *last == b'A' { b'B' } else { b'A' };
    let tampered = String::from_utf8(tampered).unwrap();
    assert!(
        validate_with_keys(
            &tampered,
            "fixture-client",
            "fixture-nonce",
            &keys(),
            &cancel
        )
        .unwrap_err()
        .contains("signature")
    );
}
