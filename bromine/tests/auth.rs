mod common;

use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use bromine::auth::{LoginAttempt, mint_tt, parse_callback};
use bromine::{Credentials, Error, SavedSession, SessionStore};
use common::{Lms, path};
use mockito::{Matcher, Mock};
use serde_json::json;

const CALLBACK: &str = "/azureadb2c/v2/callback/bbid";

fn creds(a: &str, u: &str) -> Credentials {
    Credentials { auth_svc_token: a.into(), user_data_session_id: u.into() }
}

/// Everything up to and including the sts-sso `/cookie` exchange.
fn login_mocks(s: &mut Lms) -> Vec<Mock> {
    vec![
        s.sts
            .mock("POST", "/azureadb2c/v2/state")
            .match_query(Matcher::Any)
            .match_body(Matcher::PartialJson(json!({"policy": "B2C_1A_OIDC", "embedded": false})))
            .with_header("set-cookie", "AuthSvc.AzureSignInState=abc; Path=/; Secure")
            .with_body(r#"{"state":"ST"}"#)
            .create(),
        s.sts
            .mock("GET", CALLBACK)
            .match_query(Matcher::Any)
            .match_header("cookie", Matcher::Regex("AuthSvc.AzureSignInState=abc".into()))
            .with_status(302)
            .with_header("location", "https://app.blackbaud.com/signin/")
            .with_header("set-cookie", "AuthSvc.AzureSignInState=; expires=Thu, 01 Jan 1970 00:00:00 GMT; path=/")
            .with_header("set-cookie", "AuthSvcToken=blackbaud%2Bside; domain=blackbaud.com; path=/; secure; httponly")
            .with_header("set-cookie", "userDataSessionID=B1; domain=blackbaud.com; path=/")
            .create(),
        s.sts.mock("POST", "/session/csrf").with_body(r#"{"csrf_token":"C1","session_id":"x"}"#).create(),
        s.sts
            .mock("POST", "/oauth2/token")
            .match_header("x-csrf", "C1")
            .match_header("cookie", Matcher::Regex("AuthSvcToken=blackbaud%2Bside".into()))
            .with_body(r#"{"access_token":"AT"}"#)
            .create(),
        s.sso
            .mock("POST", "/cookie")
            .match_body(Matcher::AllOf(vec![
                Matcher::Regex("accessToken=AT".into()),
                Matcher::Regex("stsDestinationUrl=".into()),
                Matcher::Regex("redirectUrl=".into()),
            ]))
            .with_status(302)
            .with_header("set-cookie", "userDataSessionID=U1%7Cx; domain=myschoolapp.com; path=/; secure")
            .with_header("set-cookie", "AuthSvcToken=sso%2Bside; domain=myschoolapp.com; path=/; secure; httponly")
            .create(),
    ]
}

fn mint_mocks(s: &mut Lms, rotate: bool) -> Vec<Mock> {
    let mut v = vec![
        s.sso
            .mock("POST", "/oauth2/token")
            .match_header("cookie", Matcher::Regex("AuthSvcToken=sso%2Bside".into()))
            .match_header("x-csrf", "token_needed")
            .match_body(Matcher::PartialJson(json!({"svc_id": "edu", "permission_scope": "bem-legacy"})))
            .with_body(r#"{"access_token":"BEARER"}"#)
            .create(),
        s.lms
            .mock("GET", "/api/bbid/login")
            .match_query(Matcher::Any)
            .match_header("authorization", "Bearer BEARER")
            .with_header("set-cookie", "t=TT123; path=/; secure; httponly")
            .with_body(r#"{"LoginSuccessful":true}"#)
            .create(),
        s.sso.mock("POST", "/session/csrf").with_body(r#"{"csrf_token":"C2"}"#).create(),
    ];
    let mut renew = s.sso.mock("POST", "/session/renew").match_header("x-csrf", "C2").with_status(200);
    if rotate {
        renew = renew.with_header("set-cookie", "AuthSvcToken=rotated; domain=myschoolapp.com; path=/; secure");
    }
    v.push(renew.create());
    v
}

#[test]
fn parse_callback_checks_path_state_and_code() {
    let base = "https://sts.sky.blackbaud.com";
    assert_eq!(parse_callback(&format!("{base}{CALLBACK}?state=S&code=C")).unwrap(), ("S".into(), "C".into()));
    assert_eq!(parse_callback(&format!("{base}{CALLBACK}/?state=S&code=C%2B1")).unwrap(), ("S".into(), "C+1".into()));
    assert!(parse_callback(&format!("{base}/somewhere/else?state=S&code=C")).is_err());
    assert!(parse_callback(&format!("{base}{CALLBACK}?state=S")).is_err());
    assert!(parse_callback("not a url").is_err());
}

#[test]
fn the_login_flow_keeps_the_two_auth_svc_tokens_apart() {
    let mut s = Lms::new();
    let mocks = login_mocks(&mut s);
    let (attempt, url) = LoginAttempt::start(&s.endpoints(), "me").unwrap();
    assert!(url.contains("state=ST") && url.contains("login_hint=me%40hunterschools.org") && url.contains("domain_hint=hunterschools-org"));
    assert!(url.contains("redirect_uri=") && url.contains("azureadb2c%2Fv2%2Fcallback%2Fbbid"), "{url}");
    let creds = attempt.finish(&format!("https://sts.sky.blackbaud.com{CALLBACK}?state=ST&code=CODE")).unwrap();
    assert_eq!(creds, self::creds("sso%2Bside", "U1%7Cx"), "the myschoolapp.com cookies, not blackbaud.com's");
    for m in mocks {
        m.assert();
    }
}

#[test]
fn a_callback_from_another_login_attempt_is_rejected() {
    let mut s = Lms::new();
    let _mocks = login_mocks(&mut s);
    let (attempt, _) = LoginAttempt::start(&s.endpoints(), "me").unwrap();
    let err = attempt.finish(&format!("https://sts.sky.blackbaud.com{CALLBACK}?state=OTHER&code=CODE")).unwrap_err();
    assert!(err.to_string().contains("state does not match"), "{err}");
}

#[test]
fn mint_tt_sends_the_saved_cookies_unchanged_and_returns_rotated_ones() {
    let mut s = Lms::new();
    let mocks = mint_mocks(&mut s, true);
    let (tt, creds) = mint_tt(&s.endpoints(), &creds("sso%2Bside", "U1%7Cx")).unwrap();
    assert_eq!(tt, "TT123");
    assert_eq!(creds, self::creds("rotated", "U1%7Cx"));
    for m in mocks {
        m.assert();
    }
}

#[test]
fn a_failed_renew_does_not_lose_the_session_we_already_have() {
    let mut s = Lms::new();
    let _ = mint_mocks(&mut s, false);
    s.sso.reset(); // drop the renew chain, keep a token endpoint that works but a renew that fails
    s.sso.mock("POST", "/oauth2/token").with_body(r#"{"access_token":"BEARER"}"#).create();
    s.sso.mock("POST", "/session/csrf").with_body(r#"{"csrf_token":"C2"}"#).create();
    s.sso.mock("POST", "/session/renew").with_status(500).create();
    let (tt, creds) = mint_tt(&s.endpoints(), &creds("a", "b")).unwrap();
    assert_eq!((tt.as_str(), creds), ("TT123", self::creds("a", "b")));
}

#[test]
fn only_rejected_credentials_mean_sign_in_again() {
    for status in [400, 401, 403] {
        let mut s = Lms::new();
        s.sso.mock("POST", "/oauth2/token").with_status(status).create();
        assert!(matches!(mint_tt(&s.endpoints(), &creds("a", "b")), Err(Error::LoginRequired(_))), "{status}");
    }
    let mut s = Lms::new();
    s.sso.mock("POST", "/oauth2/token").with_status(500).create();
    let err = mint_tt(&s.endpoints(), &creds("a", "b")).unwrap_err();
    assert!(matches!(err, Error::Auth(_)) && !err.needs_login(), "a server error must not look like a lapsed login: {err}");

    let mut s = Lms::new();
    s.sso.mock("POST", "/oauth2/token").with_body(r#"{"access_token":"B"}"#).create();
    s.lms.mock("GET", path("/api/bbid/login")).with_body(r#"{"LoginSuccessful":false}"#).create();
    assert!(mint_tt(&s.endpoints(), &creds("a", "b")).unwrap_err().needs_login());
}

// ------------------------------------------------------------------ the session store

fn saved() -> SavedSession {
    SavedSession { username: "me".into(), tt: "TT".into(), credentials: creds("a%2Bb", "u") }
}

#[test]
fn the_store_round_trips_with_private_permissions_and_never_prints_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::at(dir.path().join("cfg").join("session.json"));
    assert!(store.load().unwrap().is_none());
    store.save(&saved()).unwrap();
    let back = store.load().unwrap().unwrap();
    assert_eq!((back.username.as_str(), back.tt.as_str(), back.credentials.clone()), ("me", "TT", creds("a%2Bb", "u")));
    assert_eq!(std::fs::metadata(store.path()).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::metadata(store.path().parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
    let shown = format!("{:?} {:?}", back, back.credentials);
    assert!(!shown.contains("TT") || shown.contains("redacted"));
    assert!(!shown.contains("a%2Bb") && !shown.contains("\"u\""), "{shown}");
}

#[test]
fn the_store_reads_the_python_servers_session_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    std::fs::write(
        &path,
        r#"{"username": "student", "tt": "abc-123", "credentials": {"auth_svc_token": "AST%2B", "user_data_session_id": "UDS%7C1"}}"#,
    )
    .unwrap();
    let s = SessionStore::at(&path).load().unwrap().unwrap();
    assert_eq!((s.username.as_str(), s.tt.as_str()), ("student", "abc-123"));
    assert_eq!(s.credentials, creds("AST%2B", "UDS%7C1"));
    std::fs::write(&path, "not json").unwrap();
    assert!(SessionStore::at(&path).load().unwrap().is_none(), "a corrupt file means 'no session', not a crash");
}

#[test]
fn refresh_needs_a_session_and_persists_the_new_tt_and_rotated_credentials() {
    let mut s = Lms::new();
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::at(dir.path().join("session.json")).with_username("student");
    assert!(matches!(store.refresh(&s.endpoints()), Err(Error::LoginRequired(_))));

    store.save(&SavedSession { username: "me".into(), tt: "old".into(), credentials: creds("sso%2Bside", "U1%7Cx") }).unwrap();
    let _mocks = mint_mocks(&mut s, true);
    let fresh = store.refresh(&s.endpoints()).unwrap();
    assert_eq!(fresh.tt, "TT123");
    let on_disk = store.load().unwrap().unwrap();
    assert_eq!((on_disk.tt.as_str(), on_disk.credentials, on_disk.username.as_str()), ("TT123", creds("rotated", "U1%7Cx"), "me"));
}

#[test]
fn interactive_login_waits_for_the_userscripts_post_and_saves_the_session() {
    let mut s = Lms::new();
    let _login = login_mocks(&mut s);
    let _mint = mint_mocks(&mut s, false);
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::at(dir.path().join("session.json")).with_username("student");
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port(); // a free port

    let session = store
        .interactive_login_on(
            &s.endpoints(),
            port,
            |url| {
                assert!(url.contains("state=ST"));
                std::thread::spawn(move || {
                    let http = reqwest::blocking::Client::new();
                    // a stray request on the port must not end the wait ...
                    let _ = http.post(format!("http://127.0.0.1:{port}/cb")).body("hello").send();
                    // ... only the userscript's callback does
                    let body = format!("https://sts.sky.blackbaud.com{CALLBACK}?state=ST&code=CODE");
                    http.post(format!("http://127.0.0.1:{port}/cb")).body(body).send().unwrap();
                });
            },
            Duration::from_secs(10),
        )
        .unwrap();
    assert_eq!(session.tt, "TT123");
    assert_eq!(store.load().unwrap().unwrap().credentials, creds("sso%2Bside", "U1%7Cx"));
}

#[test]
fn interactive_login_times_out_and_reports_a_busy_port() {
    let mut s = Lms::new();
    let _login = login_mocks(&mut s);
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::at(dir.path().join("session.json")).with_username("student");
    let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = held.local_addr().unwrap().port();
    let busy = store.interactive_login_on(&s.endpoints(), port, |_| {}, Duration::from_secs(1)).unwrap_err();
    assert!(busy.to_string().contains("another sign-in"), "{busy}");
    drop(held);
    let timeout = store.interactive_login_on(&s.endpoints(), port, |_| {}, Duration::from_millis(300)).unwrap_err();
    assert!(timeout.to_string().contains("no callback received"), "{timeout}");
    assert!(store.load().unwrap().is_none());
}
