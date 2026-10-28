//! Minimal iCalendar (RFC 5545) import/export of reminders as VTODO entries.

use crate::store::{Freq, Reminder, Repeat};
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};

const STAMP: &str = "%Y%m%dT%H%M%S";

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\n', "\\n")
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(c) => out.push(c),
            None => {}
        }
    }
    out
}

pub fn export(reminders: &[Reminder]) -> String {
    let mut s = String::from("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Nudge//EN\r\n");
    let now = Utc::now().format(STAMP);
    for r in reminders {
        s += "BEGIN:VTODO\r\n";
        s += &format!("UID:nudge-{}-{}\r\n", r.id, r.created.timestamp());
        s += &format!("DTSTAMP:{now}Z\r\n");
        s += &format!("SUMMARY:{}\r\n", escape(&r.title));
        if !r.notes.is_empty() {
            s += &format!("DESCRIPTION:{}\r\n", escape(&r.notes));
        }
        if let Some(due) = r.due {
            s += &format!("DUE:{}Z\r\n", due.with_timezone(&Utc).format(STAMP));
        }
        if r.repeat.is_on() {
            let freq = match r.repeat.freq {
                Freq::Minutes => "MINUTELY",
                Freq::Hours => "HOURLY",
                Freq::Days => "DAILY",
                Freq::Weeks => "WEEKLY",
                Freq::Months => "MONTHLY",
                _ => "YEARLY",
            };
            s += &format!("RRULE:FREQ={freq};INTERVAL={}", r.repeat.every);
            if let Some(u) = r.repeat.until {
                s += &format!(";UNTIL={}", u.format("%Y%m%d"));
            }
            s += "\r\n";
        }
        if r.important {
            s += "PRIORITY:1\r\n";
        }
        s += if r.done { "STATUS:COMPLETED\r\n" } else { "STATUS:NEEDS-ACTION\r\n" };
        s += "END:VTODO\r\n";
    }
    s + "END:VCALENDAR\r\n"
}

fn parse_time(v: &str) -> Option<DateTime<Local>> {
    if let Some(utc) = v.strip_suffix('Z') {
        let t = NaiveDateTime::parse_from_str(utc, STAMP).ok()?;
        return Some(Utc.from_utc_datetime(&t).with_timezone(&Local));
    }
    let t = NaiveDateTime::parse_from_str(v, STAMP)
        .or_else(|_| NaiveDate::parse_from_str(v, "%Y%m%d").map(|d| d.and_time(NaiveTime::from_hms_opt(9, 0, 0).unwrap())))
        .ok()?;
    // ponytail: TZID parameters are ignored, times are read as local
    Local.from_local_datetime(&t).earliest()
}

fn parse_rrule(v: &str) -> Repeat {
    let mut rep = Repeat::default();
    for part in v.split(';') {
        let Some((k, val)) = part.split_once('=') else { continue };
        match k {
            "FREQ" => {
                rep.freq = match val {
                    "MINUTELY" => Freq::Minutes,
                    "HOURLY" => Freq::Hours,
                    "DAILY" => Freq::Days,
                    "WEEKLY" => Freq::Weeks,
                    "MONTHLY" => Freq::Months,
                    "YEARLY" => Freq::Years,
                    _ => Freq::Never,
                }
            }
            "INTERVAL" => rep.every = val.parse().unwrap_or(1).max(1),
            "UNTIL" => rep.until = val.get(..8).and_then(|d| NaiveDate::parse_from_str(d, "%Y%m%d").ok()),
            _ => {}
        }
    }
    rep
}

/// Parses VTODO and VEVENT entries. Ids are assigned by `new_id`.
pub fn import(text: &str, mut new_id: impl FnMut() -> u64, list: Option<u64>) -> Vec<Reminder> {
    // Unfold continuation lines (RFC 5545 section 3.1).
    let unfolded = text.replace("\r\n", "\n").replace("\n ", "").replace("\n\t", "");
    let mut out = Vec::new();
    let mut cur: Option<Reminder> = None;
    for line in unfolded.lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        let name = key.split(';').next().unwrap_or_default().to_ascii_uppercase();
        match (name.as_str(), value) {
            ("BEGIN", "VTODO" | "VEVENT") => cur = Some(Reminder::new(new_id(), list)),
            ("END", "VTODO" | "VEVENT") => {
                if let Some(r) = cur.take().filter(|r| !r.title.trim().is_empty()) {
                    out.push(r);
                }
            }
            _ => {
                let Some(r) = cur.as_mut() else { continue };
                match name.as_str() {
                    "SUMMARY" => r.title = unescape(value),
                    "DESCRIPTION" => r.notes = unescape(value),
                    "DUE" => r.due = parse_time(value),
                    "DTSTART" if r.due.is_none() => r.due = parse_time(value),
                    "RRULE" => r.repeat = parse_rrule(value),
                    "PRIORITY" => r.important = matches!(value.trim(), "1" | "2" | "3" | "4"),
                    "STATUS" => r.done = value.trim() == "COMPLETED",
                    _ => {}
                }
            }
        }
    }
    // Past one-off reminders from a calendar must not all fire at once.
    let now = Local::now();
    for r in &mut out {
        r.notified = r.due.is_some_and(|d| d <= now) && !r.repeat.is_on();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut r = Reminder::new(7, None);
        r.title = "Pay rent; call bob, now".into();
        r.notes = "line1\nline2".into();
        r.due = Some(Local.with_ymd_and_hms(2030, 3, 4, 18, 30, 0).unwrap());
        r.repeat = Repeat { freq: Freq::Months, every: 2, until: NaiveDate::from_ymd_opt(2031, 1, 1) };
        r.important = true;

        let mut id = 100;
        let back = import(&export(&[r.clone()]), || { id += 1; id }, Some(3));
        assert_eq!(back.len(), 1);
        let b = &back[0];
        assert_eq!((b.id, b.list), (101, Some(3)));
        assert_eq!((&b.title, &b.notes, b.due, &b.repeat), (&r.title, &r.notes, r.due, &r.repeat));
        assert!(b.important && !b.done && !b.notified);
    }
}
