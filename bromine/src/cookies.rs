//! A tiny cookie bag. The login flow touches two hosts that both set a cookie called `AuthSvcToken`,
//! so cookies are tracked by hand, per phase, instead of in one shared jar.

use std::collections::BTreeMap;

use reqwest::header::{HeaderMap, SET_COOKIE};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CookieBag(BTreeMap<String, String>);

impl CookieBag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, name: &str, value: &str) {
        self.0.insert(name.to_owned(), value.to_owned());
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Take in every `Set-Cookie` of a response. A cookie set to an empty value is a deletion.
    pub fn absorb(&mut self, headers: &HeaderMap) {
        for line in headers.get_all(SET_COOKIE).iter().filter_map(|v| v.to_str().ok()) {
            let pair = line.split(';').next().unwrap_or("");
            let Some((name, value)) = pair.split_once('=') else { continue };
            let (name, value) = (name.trim(), value.trim());
            if name.is_empty() {
                continue;
            }
            if value.is_empty() {
                self.0.remove(name);
            } else {
                self.0.insert(name.to_owned(), value.to_owned());
            }
        }
    }

    /// The value for a `Cookie` request header. Values are kept exactly as the server sent them
    /// (still percent-encoded), because they must be echoed back unchanged.
    pub fn header(&self) -> String {
        self.0.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    fn headers(lines: &[&str]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for l in lines {
            h.append(SET_COOKIE, HeaderValue::from_str(l).unwrap());
        }
        h
    }

    #[test]
    fn absorbs_by_name_and_keeps_values_raw() {
        let mut bag = CookieBag::new();
        bag.absorb(&headers(&[
            "AuthSvcToken=abc%2Bdef%3D%3D; domain=blackbaud.com; path=/; secure; httponly",
            "userDataSessionID=u1%7Cx; path=/",
        ]));
        assert_eq!(bag.get("AuthSvcToken"), Some("abc%2Bdef%3D%3D"));
        assert_eq!(bag.header(), "AuthSvcToken=abc%2Bdef%3D%3D; userDataSessionID=u1%7Cx");
    }

    #[test]
    fn empty_value_deletes_and_later_values_replace() {
        let mut bag = CookieBag::new();
        bag.set("AuthSvc.AzureSignInState", "s1");
        bag.absorb(&headers(&["AuthSvc.AzureSignInState=; expires=Thu, 01 Jan 1970 00:00:00 GMT; path=/"]));
        assert!(bag.is_empty());
        bag.absorb(&headers(&["t=one; path=/", "t=two; path=/"]));
        assert_eq!(bag.get("t"), Some("two"));
    }

    #[test]
    fn value_containing_equals_signs_survives() {
        let mut bag = CookieBag::new();
        bag.absorb(&headers(&["ck=a=ArR+L+swW44=; path=/; secure"]));
        assert_eq!(bag.get("ck"), Some("a=ArR+L+swW44="));
    }
}
