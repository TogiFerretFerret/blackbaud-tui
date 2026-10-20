//! Formatting and parsing helpers for the UI. Pure functions, so they are easy to test.

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// `CHEMISTRY - 20` -> `CHEMISTRY`, cut to `width` columns.
pub fn short_class(name: &str, width: usize) -> String {
    truncate(name.split(" - ").next().unwrap_or(name).trim(), width)
}

/// Cut `s` to at most `width` terminal columns, marking the cut with `…`.
pub fn truncate(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_owned();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

/// Pad (or cut) to exactly `width` columns.
pub fn fit(s: &str, width: usize) -> String {
    let t = truncate(s, width);
    format!("{t}{}", " ".repeat(width.saturating_sub(t.width())))
}

/// A short, relative due time: `today 8:00a`, `tmrw 10:34a`, `Fri 8:00a`, `Oct 13`.
pub fn due_label(due: Option<NaiveDateTime>, now: NaiveDateTime) -> String {
    let Some(due) = due else { return "no date".into() };
    let days = (due.date() - now.date()).num_days();
    let time = if due.time() == NaiveTime::MIN { String::new() } else { format!(" {}", clock(due)) };
    match days {
        0 => format!("today{time}"),
        1 => format!("tmrw{time}"),
        2..=6 => format!("{}{time}", due.format("%a")),
        _ => due.format("%b %-d").to_string(),
    }
}

/// `8:00a`, `12:10p`.
pub fn clock(dt: NaiveDateTime) -> String {
    use chrono::Timelike;
    let h = dt.hour();
    format!("{}:{:02}{}", if h.is_multiple_of(12) { 12 } else { h % 12 }, dt.minute(), if h >= 12 { "p" } else { "a" })
}

/// `Wed Oct 7, 8:00 AM`.
pub fn long_when(dt: NaiveDateTime) -> String {
    dt.format("%a %b %-d, %-I:%M %p").to_string()
}

/// Greedy word wrap to `width` columns. Keeps blank lines; breaks over-long words.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            out.push(String::new());
            continue;
        }
        let mut cur = String::new();
        for word in line.split_whitespace() {
            let mut word = word;
            // a word wider than the line is split by columns
            while word.width() > width {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                let mut cut = 0;
                let mut used = 0;
                for (i, c) in word.char_indices() {
                    let w = c.width().unwrap_or(0);
                    if used + w > width {
                        break;
                    }
                    used += w;
                    cut = i + c.len_utf8();
                }
                let (head, tail) = word.split_at(cut.max(word.chars().next().map_or(0, char::len_utf8)));
                out.push(head.to_owned());
                word = tail;
            }
            if cur.is_empty() {
                cur = word.to_owned();
            } else if cur.width() + 1 + word.width() <= width {
                cur.push(' ');
                cur.push_str(word);
            } else {
                out.push(std::mem::take(&mut cur));
                cur = word.to_owned();
            }
        }
        if !cur.is_empty() {
            out.push(cur);
        }
    }
    out
}

/// A due date typed by a person: `2026-10-02`, `2026-10-02 15:30`, `today`, `tomorrow 9:00`, `fri`.
/// A bare date means the end of that day (23:59).
pub fn parse_due(input: &str, now: NaiveDateTime) -> Option<NaiveDateTime> {
    let input = input.trim().to_lowercase();
    let (day, time) = match input.split_once(' ') {
        Some((d, t)) => (d, Some(t.trim())),
        None => (input.as_str(), None),
    };
    let date = match day {
        "today" | "tod" => now.date(),
        "tomorrow" | "tmrw" | "tom" => now.date() + Duration::days(1),
        _ => match weekday(day) {
            Some(target) => {
                let ahead = (7 + target - now.date().weekday().num_days_from_monday() as i64) % 7;
                now.date() + Duration::days(if ahead == 0 { 7 } else { ahead })
            }
            None => NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()?,
        },
    };
    let time = match time {
        None | Some("") => NaiveTime::from_hms_opt(23, 59, 0)?,
        Some(t) => parse_time(t)?,
    };
    Some(date.and_time(time))
}

fn weekday(s: &str) -> Option<i64> {
    ["mon", "tue", "wed", "thu", "fri", "sat", "sun"].iter().position(|d| s.starts_with(d) && s.len() <= 9).map(|i| i as i64)
}

fn parse_time(t: &str) -> Option<NaiveTime> {
    let t = t.replace(' ', "");
    let (digits, pm) = match t.strip_suffix("pm").or_else(|| t.strip_suffix('p')) {
        Some(d) => (d.to_owned(), Some(true)),
        None => match t.strip_suffix("am").or_else(|| t.strip_suffix('a')) {
            Some(d) => (d.to_owned(), Some(false)),
            None => (t.clone(), None),
        },
    };
    let (h, m) = match digits.split_once(':') {
        Some((h, m)) => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
        None => (digits.parse::<u32>().ok()?, 0),
    };
    let h = match pm {
        Some(true) if h < 12 => h + 12,
        Some(false) if h == 12 => 0,
        _ => h,
    };
    NaiveTime::from_hms_opt(h, m, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, min, 0).unwrap()
    }

    #[test]
    fn truncate_and_fit_count_columns_not_bytes() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 6), "hello…");
        assert_eq!(truncate("日本語のテキスト", 7), "日本語…");
        assert_eq!(fit("ab", 5), "ab   ");
        assert_eq!(fit("abcdefgh", 5), "abcd…");
        assert_eq!(short_class("CHEMISTRY - 20", 20), "CHEMISTRY");
        assert_eq!(short_class("ADV SCI RESCH SEMINAR - 10", 10), "ADV SCI R…");
    }

    #[test]
    fn due_labels_are_relative() {
        let now = dt(2026, 9, 30, 17, 40); // a Wednesday
        assert_eq!(due_label(Some(dt(2026, 9, 30, 8, 0)), now), "today 8:00a");
        assert_eq!(due_label(Some(dt(2026, 10, 1, 10, 34)), now), "tmrw 10:34a");
        assert_eq!(due_label(Some(dt(2026, 10, 5, 12, 10)), now), "Mon 12:10p");
        assert_eq!(due_label(Some(dt(2026, 10, 13, 8, 0)), now), "Oct 13");
        assert_eq!(due_label(Some(dt(2026, 10, 2, 0, 0)), now), "Fri");
        assert_eq!(due_label(None, now), "no date");
        assert_eq!(long_when(dt(2026, 10, 7, 8, 0)), "Wed Oct 7, 8:00 AM");
    }

    #[test]
    fn wrapping_keeps_blank_lines_and_splits_long_words() {
        assert_eq!(wrap("aaa bbb ccc", 7), ["aaa bbb", "ccc"]);
        assert_eq!(wrap("one\n\ntwo", 10), ["one", "", "two"]);
        assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap("", 5), Vec::<String>::new());
    }

    #[test]
    fn due_dates_as_typed() {
        let now = dt(2026, 9, 30, 17, 40); // Wednesday
        assert_eq!(parse_due("2026-10-02", now), Some(dt(2026, 10, 2, 23, 59)));
        assert_eq!(parse_due("2026-10-02 15:30", now), Some(dt(2026, 10, 2, 15, 30)));
        assert_eq!(parse_due("today", now), Some(dt(2026, 9, 30, 23, 59)));
        assert_eq!(parse_due("tomorrow 9:00", now), Some(dt(2026, 10, 1, 9, 0)));
        assert_eq!(parse_due("tmrw 9pm", now), Some(dt(2026, 10, 1, 21, 0)));
        assert_eq!(parse_due("fri", now), Some(dt(2026, 10, 2, 23, 59)));
        assert_eq!(parse_due("wed", now), Some(dt(2026, 10, 7, 23, 59)), "the same weekday means next week");
        assert_eq!(parse_due("12am", now), None, "a time needs a day");
        assert_eq!(parse_due("tomorrow 25:00", now), None);
        assert_eq!(parse_due("soon", now), None);
        assert_eq!(parse_due("tomorrow 12am", now), Some(dt(2026, 10, 1, 0, 0)));
    }
}
