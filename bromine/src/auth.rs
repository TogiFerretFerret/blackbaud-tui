//! Blackbaud ID login (Azure B2C with Google upstream) for Hunter Schools' LMS.
//!
//! ```text
//!   1. POST sts.sky/azureadb2c/v2/state          -> `state` (+ AuthSvc.AzureSignInState cookie)
//!   2. browser: id.blackbaud.com/.../authorize   -> Google -> sts.sky/azureadb2c/v2/callback/bbid?state&code
//!   3. GET  that callback with the state cookie  -> AuthSvcToken cookie (blackbaud.com)
//!   4. POST sts.sky/session/csrf, /oauth2/token  -> access token
//!   5. POST sts-sso/cookie (accessToken)         -> AuthSvcToken cookie (myschoolapp.com)    = Credentials
//!   6. POST sts-sso/oauth2/token                 -> LMS-scoped bearer token
//!   7. GET  LMS/api/bbid/login                   -> `t` session cookie                       = what Client uses
//! ```
//!
//! Steps 1-5 need a human (step 2); steps 6-7 only need [`Credentials`] and can repeat headlessly.

use reqwest::blocking::{Client as Http, RequestBuilder, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

use crate::cookies::CookieBag;
use crate::error::{Error, Result};

pub const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
pub const ENV_ID: &str = "p-9A4jO0o5LESTJyyg7MVsCA";
pub const EMAIL_DOMAIN: &str = "hunterschools.org";
const B2C_CLIENT_ID: &str = "886aaf26-fc86-43b6-a838-fdc1eef0c3f9";
const B2C_POLICY: &str = "B2C_1A_OIDC";
pub const CALLBACK_PATH: &str = "/azureadb2c/v2/callback/bbid";

/// Where the four services live. Production by default; tests point these at a mock server.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub sts: String,
    pub sts_sso: String,
    pub lms: String,
    pub b2c_authorize: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            sts: "https://sts.sky.blackbaud.com".into(),
            sts_sso: "https://sts-sso.myschoolapp.com".into(),
            lms: "https://hunterschools.myschoolapp.com".into(),
            b2c_authorize: "https://id.blackbaud.com/bbid.onmicrosoft.com/B2C_1A_OIDC/oauth2/v2.0/authorize".into(),
        }
    }
}

impl Endpoints {
    fn landing_url(&self) -> String {
        format!("{}/app?svcId=edu&envId={ENV_ID}&bb_id=1#login", self.lms)
    }
}

/// The long-lived sts-sso session cookies (domain `myschoolapp.com`), exactly as the server sent them.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credentials {
    pub auth_svc_token: String,
    pub user_data_session_id: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Credentials(<redacted>)") // live session secrets: never print them
    }
}

impl Credentials {
    fn bag(&self) -> CookieBag {
        let mut bag = CookieBag::new();
        bag.set("AuthSvcToken", &self.auth_svc_token);
        bag.set("userDataSessionID", &self.user_data_session_id);
        bag
    }

    fn from_bag(bag: &CookieBag) -> Result<Self> {
        let get = |name: &str| bag.get(name).map(str::to_owned).ok_or_else(|| Error::Auth(format!("expected cookie {name:?} was not set")));
        Ok(Self { auth_svc_token: get("AuthSvcToken")?, user_data_session_id: get("userDataSessionID")? })
    }
}

pub(crate) fn http() -> Result<Http> {
    Ok(Http::builder()
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()?)
}

fn expect(resp: Response, what: &str, ok: &[u16]) -> Result<Response> {
    let status = resp.status().as_u16();
    if ok.contains(&status) {
        return Ok(resp);
    }
    let body = resp.text().unwrap_or_default();
    Err(Error::Auth(format!("{what}: unexpected HTTP {status}: {}", body.chars().take(200).collect::<String>())))
}

fn json_field(v: &Value, key: &str, what: &str) -> Result<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned).ok_or_else(|| Error::Auth(format!("{what}: response has no {key:?}")))
}

/// `(state, code)` from a captured B2C callback URL.
pub fn parse_callback(url: &str) -> Result<(String, String)> {
    let parsed = Url::parse(url).map_err(|e| Error::Auth(format!("not a URL: {e}")))?;
    if parsed.path().trim_end_matches('/') != CALLBACK_PATH {
        return Err(Error::Auth(format!("not a B2C callback URL: {}", parsed.path())));
    }
    let find = |key: &str| parsed.query_pairs().find(|(k, _)| k == key).map(|(_, v)| v.into_owned());
    match (find("state"), find("code")) {
        (Some(s), Some(c)) => Ok((s, c)),
        (None, _) => Err(Error::Auth("callback URL is missing \"state\"".into())),
        (_, None) => Err(Error::Auth("callback URL is missing \"code\"".into())),
    }
}

/// One interactive login attempt: [`LoginAttempt::start`], let a human sign in, then [`LoginAttempt::finish`].
pub struct LoginAttempt {
    ep: Endpoints,
    http: Http,
    state: String,
    cookies: CookieBag,
}

impl LoginAttempt {
    /// Register the attempt and return it with the URL to open in a browser.
    ///
    /// The browser must not hold this attempt's state cookie: its callback request is meant to *fail*,
    /// leaving the single-use code unconsumed in the URL for `capture.user.js` to read.
    pub fn start(ep: &Endpoints, username: &str) -> Result<(Self, String)> {
        let http = http()?;
        let login_hint = format!("{username}@{EMAIL_DOMAIN}");
        let landing = ep.landing_url();
        let resp = http
            .post(format!("{}/azureadb2c/v2/state", ep.sts))
            .query(&[("login_hint", login_hint.as_str()), ("redirectUrl", landing.as_str()), ("bbcid", "spa-signin")])
            .header("origin", "https://app.blackbaud.com")
            .json(&json!({"embedded": false, "policy": B2C_POLICY, "redirect_url": landing, "custom_branding_present": true}))
            .send()?;
        let resp = expect(resp, "azureadb2c/v2/state", &[200])?;
        let mut cookies = CookieBag::new();
        cookies.absorb(resp.headers());
        let state = json_field(&resp.json::<Value>()?, "state", "azureadb2c/v2/state")?;

        let mut url = Url::parse(&ep.b2c_authorize).map_err(|e| Error::Auth(e.to_string()))?;
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("response_mode", "query")
            .append_pair("scope", "openid")
            .append_pair("client_id", B2C_CLIENT_ID)
            .append_pair("redirect_uri", &format!("{}{CALLBACK_PATH}", ep.sts))
            .append_pair("state", &state)
            .append_pair("login_hint", &login_hint)
            .append_pair("domain_hint", &EMAIL_DOMAIN.replace('.', "-"));
        Ok((Self { ep: ep.clone(), http, state, cookies }, url.into()))
    }

    /// Redeem the captured callback URL (steps 3-5) for sts-sso credentials.
    pub fn finish(mut self, callback_url: &str) -> Result<Credentials> {
        let (state, code) = parse_callback(callback_url)?;
        if state != self.state {
            return Err(Error::Auth("callback state does not match this login attempt".into()));
        }
        let origin = "https://app.blackbaud.com";

        // 3. The callback 302s to the SPA; only its Set-Cookie matters.
        let resp = self
            .http
            .get(format!("{}{CALLBACK_PATH}", self.ep.sts))
            .query(&[("state", state.as_str()), ("code", code.as_str())])
            .header("cookie", self.cookies.header())
            .send()?;
        let resp = expect(resp, "azureadb2c/v2/callback", &[302])?;
        self.cookies.absorb(resp.headers());

        // 4. An access token, bound to a session CSRF token.
        let csrf = csrf(&self.http, &self.ep.sts, &self.cookies, origin)?;
        let resp = self
            .http
            .post(format!("{}/oauth2/token", self.ep.sts))
            .header("content-type", "application/json")
            .header("x-csrf", csrf)
            .header("origin", origin)
            .header("cookie", self.cookies.header())
            .send()?;
        let access_token = json_field(&expect(resp, "sts oauth2/token", &[200])?.json::<Value>()?, "access_token", "sts oauth2/token")?;

        // 5. Trade it for cookies scoped to myschoolapp.com.
        let landing = self.ep.landing_url();
        let resp = self
            .http
            .post(format!("{}/cookie", self.ep.sts_sso))
            .header("origin", origin)
            .header("referer", format!("{origin}/"))
            .form(&[
                ("redirectUrl", landing.as_str()),
                ("accessToken", access_token.as_str()),
                ("stsDestinationUrl", self.ep.sts_sso.as_str()),
            ])
            .send()?;
        let resp = expect(resp, "sts-sso/cookie", &[302])?;
        let mut sso = CookieBag::new();
        sso.absorb(resp.headers());
        Credentials::from_bag(&sso)
    }
}

fn csrf(http: &Http, base: &str, cookies: &CookieBag, origin: &str) -> Result<String> {
    let resp = http
        .post(format!("{base}/session/csrf"))
        .header("content-type", "application/json")
        .header("x-csrf", "token_needed")
        .header("origin", origin)
        .header("cookie", cookies.header())
        .send()?;
    json_field(&expect(resp, "session/csrf", &[200])?.json::<Value>()?, "csrf_token", "session/csrf")
}

fn with_cookies(rb: RequestBuilder, bag: &CookieBag) -> RequestBuilder {
    rb.header("cookie", bag.header())
}

/// Steps 6-7: headlessly turn `creds` into an LMS `t` cookie.
///
/// Also renews the sts-sso session, so it returns the (possibly rotated) credentials that must be saved
/// in place of the ones passed in. [`Error::LoginRequired`] means the sts-sso session is dead.
pub fn mint_tt(ep: &Endpoints, creds: &Credentials) -> Result<(String, Credentials)> {
    let http = http()?;
    let sso = creds.bag();
    let origin = ep.lms.as_str();

    let resp = with_cookies(http.post(format!("{}/oauth2/token", ep.sts_sso)), &sso)
        .header("x-csrf", "token_needed")
        .header("origin", origin)
        .json(&json!({"environment_id": ENV_ID, "permission_scope": "bem-legacy", "svc_id": "edu"}))
        .send()?;
    if matches!(resp.status().as_u16(), 400 | 401 | 403) {
        return Err(Error::LoginRequired("the sts-sso session expired".into()));
    }
    let bearer = json_field(&expect(resp, "sts-sso oauth2/token", &[200])?.json::<Value>()?, "access_token", "sts-sso oauth2/token")?;

    let resp = with_cookies(http.get(format!("{}/api/bbid/login", ep.lms)), &sso)
        .query(&[("loginTypeId", "1")])
        .bearer_auth(bearer)
        .header("x-requested-with", "XMLHttpRequest")
        .send()?;
    let resp = expect(resp, "api/bbid/login", &[200])?;
    let mut login = CookieBag::new();
    login.absorb(resp.headers());
    let body: Value = resp.json()?;
    if body.get("LoginSuccessful").and_then(Value::as_bool) != Some(true) {
        return Err(Error::LoginRequired("the LMS rejected the bearer token".into()));
    }
    let tt = login.get("t").ok_or_else(|| Error::Auth("api/bbid/login did not set the `t` cookie".into()))?.to_owned();

    // Keep the sts-sso session alive. Best effort: a failed renew must not lose the `t` we already have.
    let renewed = (|| -> Result<Credentials> {
        let csrf = csrf(&http, &ep.sts_sso, &sso, origin)?;
        let resp = with_cookies(http.post(format!("{}/session/renew", ep.sts_sso)), &sso)
            .header("content-type", "application/json")
            .header("x-csrf", csrf)
            .header("origin", origin)
            .send()?;
        let resp = expect(resp, "session/renew", &[200])?;
        let mut bag = sso.clone();
        bag.absorb(resp.headers());
        Credentials::from_bag(&bag)
    })();
    Ok((tt, renewed.unwrap_or_else(|_| creds.clone())))
}
