use jecode::providers::openai_account::auth::{self, DeviceLogin, Error, PollOutcome};
use std::time::{Duration, Instant};

const START: &str =
    r#"{"device_auth_id":"synthetic-device","user_code":"ABCD-EFGH","interval":"5"}"#;
const TOKEN: &str = "e30.eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoiYWNjb3VudC10ZXN0In19.c2ln";
fn granted() -> String {
    format!(
        r#"{{"authorization_code":"synthetic+code&value","code_verifier":"{}"}}"#,
        "a".repeat(43)
    )
}
fn tokens() -> String {
    format!(
        r#"{{"access_token":"{TOKEN}","refresh_token":"synthetic-refresh","expires_in":3600,"token_type":"Bearer"}}"#
    )
}
fn authorized(now: Instant) -> DeviceLogin {
    let mut login = DeviceLogin::begin(200, START, now).unwrap();
    let next = now + Duration::from_secs(5);
    assert!(login.poll_request(next).unwrap().is_some());
    assert_eq!(
        login.accept_poll(200, &granted(), next),
        Ok(PollOutcome::Authorized)
    );
    login
}

#[test]
fn saved_account_roundtrip_refresh_rotation_and_redaction() {
    let now = Instant::now();
    let mut login = authorized(now);
    login
        .exchange_request(now + Duration::from_secs(5))
        .unwrap();
    let initial = login
        .complete(200, &tokens(), now + Duration::from_secs(6), 1000)
        .unwrap();
    let saved = initial.saved_json().unwrap();
    let restored = auth::Tokens::from_saved_json(&saved).unwrap().unwrap();
    assert_eq!(restored.account_id(), "account-test");
    let request = String::from_utf8(restored.refresh_request().unwrap()).unwrap();
    assert!(request.starts_with("POST /oauth/token HTTP/1.1\r\nHost: auth.openai.com\r\n"));
    assert!(request.contains("grant_type=refresh_token"));
    let rotated = restored
        .refreshed(
            200,
            &tokens().replace("synthetic-refresh", "rotated-token"),
            2000,
        )
        .unwrap();
    assert_eq!(rotated.refresh_token(), "rotated-token");
    assert_eq!(rotated.expires_at(), 5600);
    let without_refresh = tokens().replace("\"refresh_token\":\"synthetic-refresh\",", "");
    assert_eq!(
        rotated
            .refreshed(200, &without_refresh, 3000)
            .unwrap()
            .refresh_token(),
        "rotated-token"
    );
    assert!(matches!(
        rotated.refreshed(400, r#"{"error":"invalid_grant"}"#, 3000),
        Err(Error::Denied)
    ));
    assert!(
        auth::Tokens::from_saved_json(&saved.replace("account-test", "another-account")).is_err()
    );
    assert!(
        auth::Tokens::from_saved_json(&saved.replace("\"version\":1", "\"version\":9")).is_err()
    );
    assert!(
        auth::Tokens::from_saved_json(r#"{"version":1,"state":"reauth_required"}"#)
            .unwrap()
            .is_none()
    );
    assert!(!format!("{rotated:?}").contains("rotated-token"));
}

#[test]
fn device_login_prepares_fixed_authority_and_single_token_exchange() {
    let start = String::from_utf8(auth::start_request().unwrap()).unwrap();
    assert!(start.starts_with(
        "POST /api/accounts/deviceauth/usercode HTTP/1.1\r\nHost: auth.openai.com\r\n"
    ));
    assert!(start.contains("\"client_id\":\"app_EMoamEEZ73f0CkXaXp7hrann\""));
    let now = Instant::now();
    let mut login = authorized(now);
    assert_eq!(login.user_code(), "ABCD-EFGH");
    let request = String::from_utf8(
        login
            .exchange_request(now + Duration::from_secs(5))
            .unwrap(),
    )
    .unwrap();
    assert!(request.contains("Content-Type: application/x-www-form-urlencoded\r\n"));
    assert!(request.contains("code=synthetic%2Bcode%26value"));
    assert!(request.contains("redirect_uri=https%3A%2F%2Fauth.openai.com%2Fdeviceauth%2Fcallback"));
    assert_eq!(
        login.exchange_request(now + Duration::from_secs(6)),
        Err(Error::State)
    );
    let account = login
        .complete(200, &tokens(), now + Duration::from_secs(6), 1000)
        .unwrap();
    assert_eq!(account.access_token(), TOKEN);
    assert_eq!(account.refresh_token(), "synthetic-refresh");
    assert_eq!(account.account_id(), "account-test");
    assert_eq!(account.expires_at(), 4600);
    assert_eq!(format!("{account:?}"), "Tokens { [redacted] }");
}

#[test]
fn polling_enforces_interval_slowdown_and_timeout_backoff() {
    let now = Instant::now();
    let mut login = DeviceLogin::begin(200, START, now).unwrap();
    assert!(login.poll_request(now).unwrap().is_none());
    let first = now + Duration::from_secs(5);
    assert!(login.poll_request(first).unwrap().is_some());
    assert_eq!(login.poll_request(first), Err(Error::State));
    assert_eq!(
        login.accept_poll(400, r#"{"error":"slow_down"}"#, first),
        Ok(PollOutcome::Waiting)
    );
    assert_eq!(login.next_poll(), now + Duration::from_secs(15));
    assert!(
        login
            .poll_request(now + Duration::from_secs(14))
            .unwrap()
            .is_none()
    );
    assert!(
        login
            .poll_request(now + Duration::from_secs(15))
            .unwrap()
            .is_some()
    );
    login.poll_timed_out(now + Duration::from_secs(16)).unwrap();
    assert_eq!(login.next_poll(), now + Duration::from_secs(36));
    login.poll_request(now + Duration::from_secs(36)).unwrap();
    assert_eq!(
        login.accept_poll(403, "{}", now + Duration::from_secs(36)),
        Ok(PollOutcome::Waiting)
    );
    assert_eq!(login.next_poll(), now + Duration::from_secs(56));
}

#[test]
fn cancellation_deadline_and_unknown_responses_end_attempt() {
    let now = Instant::now();
    let mut login = DeviceLogin::begin(200, START, now).unwrap();
    assert_eq!(
        login.poll_request(now + Duration::from_secs(900)),
        Err(Error::Expired)
    );
    assert_eq!(login.poll_request(now), Err(Error::Closed));
    let mut login = DeviceLogin::begin(200, START, now).unwrap();
    login.cancel();
    assert_eq!(login.user_code(), "");
    assert_eq!(login.poll_request(now), Err(Error::Closed));
    for (status, body, expected) in [
        (400, r#"{"error":"access_denied"}"#, Error::Denied),
        (400, r#"{"error":{"code":"expired_token"}}"#, Error::Expired),
        (500, r#"{"error":"authorization_pending"}"#, Error::Remote),
        (400, r#"{"error":"synthetic-secret"}"#, Error::Remote),
        (400, r#"{"error":false}"#, Error::Invalid),
        (
            200,
            r#"{"authorization_code":"code","code_verifier":"short"}"#,
            Error::Invalid,
        ),
    ] {
        let mut login = DeviceLogin::begin(200, START, now).unwrap();
        login.poll_request(now + Duration::from_secs(5)).unwrap();
        assert_eq!(
            login.accept_poll(status, body, now + Duration::from_secs(5)),
            Err(expected)
        );
        assert_eq!(
            login.poll_request(now + Duration::from_secs(10)),
            Err(Error::Closed)
        );
        assert!(!expected.to_string().contains("synthetic-secret"));
    }
}

#[test]
fn token_reply_requires_account_and_valid_lifetime() {
    let now = Instant::now();
    for body in [
        tokens().replace("3600", "0"),
        tokens().replace("3600", "-1"),
        tokens().replace("3600", "1.5"),
        tokens().replace(TOKEN, "bad.token"),
        tokens().replace(TOKEN, "e30.e30.c2ln"),
        tokens().replace("Bearer", "Other"),
        tokens().replace("synthetic-refresh", ""),
    ] {
        let mut login = authorized(now);
        login
            .exchange_request(now + Duration::from_secs(5))
            .unwrap();
        assert!(matches!(
            login.complete(200, &body, now + Duration::from_secs(6), 1000),
            Err(Error::Invalid)
        ));
    }
    let mut login = authorized(now);
    login
        .exchange_request(now + Duration::from_secs(5))
        .unwrap();
    assert!(matches!(
        login.complete(200, &tokens(), now + Duration::from_secs(6), u64::MAX),
        Err(Error::Invalid)
    ));
    let mut login = authorized(now);
    login
        .exchange_request(now + Duration::from_secs(5))
        .unwrap();
    assert!(matches!(
        login.complete(200, &tokens(), now + Duration::from_secs(900), 1000),
        Err(Error::Expired)
    ));
}

#[test]
fn invalid_start_data_and_unrequested_replies_are_rejected() {
    let now = Instant::now();
    for body in [
        "{}".to_owned(),
        START.replace("\"5\"", "0"),
        START.replace("\"5\"", "31"),
        START.replace("\"5\"", "\"5.0\""),
        START.replace("ABCD-EFGH", "bad\\ncode"),
        START.replace("ABCD-EFGH", ""),
        " ".repeat(32769),
    ] {
        assert!(matches!(
            DeviceLogin::begin(200, &body, now),
            Err(Error::Invalid)
        ));
    }
    let mut login = DeviceLogin::begin(200, START, now).unwrap();
    assert_eq!(login.accept_poll(200, &granted(), now), Err(Error::State));
    assert_eq!(login.exchange_request(now), Err(Error::State));
    assert!(matches!(
        login.complete(200, &tokens(), now, 0),
        Err(Error::State)
    ));
}
