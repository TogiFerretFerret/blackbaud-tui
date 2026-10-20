//! HTML-to-text and LMS date handling.

use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Timelike};

const BLOCK_TAGS: &[&str] = &["br", "p", "div", "li", "tr", "h1", "h2", "h3", "h4", "h5", "h6", "ul", "ol", "table"];

/// The LMS's HTML descriptions as plain text: block tags become line breaks, entities are decoded,
/// and `<a href>` keeps its URL as `text (url)`. `None` when nothing is left.
pub fn html_to_text(html: &str) -> Option<String> {
    let mut out = String::new();
    let mut rest = html;
    let mut href: Option<String> = None;
    let mut link_start = 0usize;
    let mut skip_until: Option<&str> = None;

    while !rest.is_empty() {
        match rest.find('<') {
            None => {
                if skip_until.is_none() {
                    out.push_str(&decode_entities(rest));
                }
                break;
            }
            Some(i) => {
                if skip_until.is_none() {
                    out.push_str(&decode_entities(&rest[..i]));
                }
                let after = &rest[i + 1..];
                let Some(end) = after.find('>') else {
                    // a stray '<' with no closing '>': treat the rest as text
                    if skip_until.is_none() {
                        out.push_str(&decode_entities(&rest[i..]));
                    }
                    break;
                };
                let tag = &after[..end];
                rest = &after[end + 1..];

                let closing = tag.starts_with('/');
                let name =
                    tag.trim_start_matches('/').split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("").to_ascii_lowercase();
                if let Some(s) = skip_until {
                    if closing && name == s {
                        skip_until = None;
                    }
                    continue;
                }
                match (name.as_str(), closing) {
                    ("script", false) => skip_until = Some("script"),
                    ("style", false) => skip_until = Some("style"),
                    ("a", false) => {
                        href = attr(tag, "href");
                        link_start = out.len();
                    }
                    ("a", true) => {
                        if let Some(h) = href.take() {
                            let text = out[link_start..].trim();
                            if (h.starts_with("http://") || h.starts_with("https://")) && h != text {
                                out.push_str(&format!(" ({h})"));
                            }
                        }
                    }
                    (n, _) if BLOCK_TAGS.contains(&n) => out.push('\n'),
                    _ => {}
                }
            }
        }
    }

    let text = out
        .replace('\u{a0}', " ")
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(text)
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let at = lower.find(&format!("{name}="))? + name.len() + 1;
    let rest = &tag[at..];
    let (quote, body) = match rest.chars().next()? {
        q @ ('"' | '\'') => (Some(q), &rest[1..]),
        _ => (None, rest),
    };
    let end = match quote {
        Some(q) => body.find(q)?,
        None => body.find(char::is_whitespace).unwrap_or(body.len()),
    };
    Some(decode_entities(&body[..end]))
}

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let decoded = tail.find(';').filter(|&e| e <= 10).and_then(|e| entity(&tail[1..e]).map(|c| (c, e + 1)));
        match decoded {
            Some((c, used)) => {
                out.push_str(&c);
                rest = &tail[used..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity(name: &str) -> Option<String> {
    if let Some(num) = name.strip_prefix('#') {
        let code = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => num.parse().ok()?,
        };
        return char::from_u32(code).map(String::from);
    }
    Some(
        match name {
            "amp" => "&",
            "lt" => "<",
            "gt" => ">",
            "quot" => "\"",
            "apos" => "'",
            "nbsp" => " ",
            "ndash" => "–",
            "mdash" => "—",
            "lsquo" => "‘",
            "rsquo" => "’",
            "ldquo" => "“",
            "rdquo" => "”",
            "hellip" => "…",
            "bull" => "•",
            "deg" => "°",
            "times" => "×",
            _ => return None,
        }
        .to_owned(),
    )
}

/// `"9/30/2026 12:10 PM"` or `"9/30/2026"` (midnight).
pub fn parse_lms_datetime(s: &str) -> Option<NaiveDateTime> {
    let s = s.trim();
    NaiveDateTime::parse_from_str(s, "%m/%d/%Y %I:%M %p")
        .ok()
        .or_else(|| NaiveDate::parse_from_str(s, "%m/%d/%Y").ok().map(|d| d.and_time(NaiveTime::MIN)))
}

/// The LMS's own format: `9/30/2026 12:10 PM`.
pub fn format_lms_datetime(dt: NaiveDateTime) -> String {
    use chrono::Datelike;
    let (h, pm) = (dt.hour(), dt.hour() >= 12);
    format!(
        "{}/{}/{} {}:{:02} {}",
        dt.month(),
        dt.day(),
        dt.year(),
        if h.is_multiple_of(12) { 12 } else { h % 12 },
        dt.minute(),
        if pm { "PM" } else { "AM" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_becomes_plain_text() {
        let html = "<div>Full&nbsp;year<br />Credits &ndash; 1.0</div><div>Texts:<br>A &amp; B &#169; &#x41;</div>";
        assert_eq!(html_to_text(html).unwrap(), "Full year\nCredits – 1.0\nTexts:\nA & B © A");
        assert_eq!(html_to_text("<div> </div>"), None);
        assert_eq!(html_to_text("a &unknown; b & c"), Some("a &unknown; b & c".into()));
    }

    #[test]
    fn links_keep_their_url_unless_redundant() {
        assert_eq!(
            html_to_text(r#"See <a href="https://docs.google.com/d/1">the doc</a> now"#).unwrap(),
            "See the doc (https://docs.google.com/d/1) now"
        );
        assert_eq!(html_to_text(r#"<a href="https://x.org">https://x.org</a>"#).unwrap(), "https://x.org");
        assert_eq!(html_to_text(r#"<a href="javascript:void(0)">btn</a>"#).unwrap(), "btn");
        assert_eq!(html_to_text("a<script>var x = '<b>';</script>b").unwrap(), "ab");
    }

    #[test]
    fn lms_datetimes_round_trip() {
        let dt = parse_lms_datetime("9/30/2026 12:10 PM").unwrap();
        assert_eq!(dt.to_string(), "2026-09-30 12:10:00");
        assert_eq!(format_lms_datetime(dt), "9/30/2026 12:10 PM");
        for s in ["10/2/2026 12:05 AM", "10/2/2026 11:59 PM", "1/5/2026 9:00 AM"] {
            assert_eq!(format_lms_datetime(parse_lms_datetime(s).unwrap()), s);
        }
        assert_eq!(parse_lms_datetime("10/7/2026").unwrap().to_string(), "2026-10-07 00:00:00");
        assert_eq!(parse_lms_datetime("garbage"), None);
    }
}
