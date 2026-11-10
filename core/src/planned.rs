//! What is planned, for the inbox's "Planned" area (2026-11-10): starts that will open a session by themselves, and tasks
//! due on a day. Another program writes them as JSON to the file named by `plannedFile` in the configuration; Lookout
//! only reads it.
//!
//! The file is `{"items": [...]}` or a bare list. An item has a `title` and either `at` (RFC 3339 with an offset: a
//! start) or `date` (YYYY-MM-DD: a task due that day); optional are `project`, `detail` (one line, e.g. how the start
//! runs), `problem` (why a start did not happen), `deadline` (YYYY-MM-DD) and `since_days` (how long it has been due).
//! Unknown keys are ignored, broken items skipped.

pub use chrono::NaiveDate;
use std::time::SystemTime;

use chrono::{DateTime, Datelike, Duration, Local, NaiveTime, TimeZone};
use serde_json::Value;

use crate::i18n::{is_german, tr};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedItem {
    pub title: String,
    pub project: String,
    /// Local day it starts or is due.
    pub day: NaiveDate,
    /// Local time of a start; `None` for a task due that day.
    pub time: Option<NaiveTime>,
    pub detail: Option<String>,
    pub problem: Option<String>,
    pub deadline: Option<NaiveDate>,
    pub since_days: i64,
}

/// The file's bytes, times in the Mac's time zone.
pub fn parse_planned(data: &[u8]) -> Vec<PlannedItem> {
    parse_planned_in(data, &Local)
}

/// `parse_planned` in a given time zone (tests run in any zone).
pub fn parse_planned_in<Tz: TimeZone>(data: &[u8], tz: &Tz) -> Vec<PlannedItem> {
    let Ok(v) = serde_json::from_slice::<Value>(data) else {
        return vec![];
    };
    let list = match &v {
        Value::Array(a) => a,
        Value::Object(o) => match o.get("items") {
            Some(Value::Array(a)) => a,
            _ => return vec![],
        },
        _ => return vec![],
    };
    list.iter().filter_map(|v| item(v, tz)).collect()
}

fn item<Tz: TimeZone>(v: &Value, tz: &Tz) -> Option<PlannedItem> {
    let o = v.as_object()?;
    let text = |k: &str| {
        o.get(k)
            .and_then(Value::as_str)
            .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|s| !s.is_empty())
    };
    let date = |k: &str| text(k).and_then(|d| NaiveDate::parse_from_str(&d, "%Y-%m-%d").ok());
    let (day, time) = match text("at") {
        Some(at) => {
            let local = DateTime::parse_from_rfc3339(&at).ok()?.with_timezone(tz);
            (local.date_naive(), Some(local.time()))
        }
        None => (date("date")?, None),
    };
    Some(PlannedItem {
        title: text("title")?,
        project: text("project").unwrap_or_default(),
        day,
        time,
        detail: text("detail"),
        problem: text("problem"),
        deadline: date("deadline"),
        since_days: o.get("since_days").and_then(Value::as_i64).unwrap_or(0),
    })
}

/// The file at `path` after another look, with the time it was read at: read again only when that time changed. A
/// missing or unreadable file is `None`, so the next look tries again (review quick 2026-11-10).
pub fn reload(
    path: &str,
    cache: Option<(SystemTime, Vec<PlannedItem>)>,
) -> Option<(SystemTime, Vec<PlannedItem>)> {
    let at = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    match cache {
        Some(c) if c.0 == at => Some(c),
        _ => std::fs::read(path)
            .ok()
            .map(|bytes| (at, parse_planned(&bytes))),
    }
}

/// The Mac's date today.
pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// Monday of `d`'s week.
pub fn monday_of(d: NaiveDate) -> NaiveDate {
    d - Duration::days(i64::from(d.weekday().num_days_from_monday()))
}

/// The day the Planned area shows after moving `weeks` weeks from `day`: today in this week, otherwise that Monday.
pub fn shift_week(day: NaiveDate, weeks: i64, today: NaiveDate) -> NaiveDate {
    let monday = monday_of(day) + Duration::days(7 * weeks);
    if monday == monday_of(today) {
        today
    } else {
        monday
    }
}

/// Where an item stands in the week: a task due in the past stands on today.
pub fn shown_day(it: &PlannedItem, today: NaiveDate) -> NaiveDate {
    it.day.max(today)
}

/// The items of `day` in order: starts by time first, then tasks (the longest due first), each in file order.
pub fn items_on(items: &[PlannedItem], day: NaiveDate, today: NaiveDate) -> Vec<&PlannedItem> {
    let mut on: Vec<&PlannedItem> = items
        .iter()
        .filter(|it| shown_day(it, today) == day)
        .collect();
    on.sort_by_key(|it| (it.time.is_none(), it.time, -it.since_days));
    on
}

/// Items whose hard deadline is `day`, standing elsewhere.
pub fn deadlines_on(items: &[PlannedItem], day: NaiveDate, today: NaiveDate) -> Vec<&PlannedItem> {
    items
        .iter()
        .filter(|it| it.deadline == Some(day) && shown_day(it, today) != day)
        .collect()
}

/// How many items stand in today's week from today on (the tile's count).
pub fn count_this_week(items: &[PlannedItem], today: NaiveDate) -> usize {
    let sunday = monday_of(today) + Duration::days(6);
    items
        .iter()
        .filter(|it| shown_day(it, today) <= sunday)
        .count()
}

const WEEKDAYS_DE: [&str; 7] = ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"];
const WEEKDAYS_EN: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTHS_DE: [&str; 12] = [
    "Januar",
    "Februar",
    "März",
    "April",
    "Mai",
    "Juni",
    "Juli",
    "August",
    "September",
    "Oktober",
    "November",
    "Dezember",
];
const MONTHS_EN: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// "Di" / "Tue".
pub fn weekday_short(d: NaiveDate) -> &'static str {
    let k = d.weekday().num_days_from_monday() as usize;
    tr(WEEKDAYS_DE[k], WEEKDAYS_EN[k])
}

fn month(d: NaiveDate) -> &'static str {
    let k = d.month0() as usize;
    tr(MONTHS_DE[k], MONTHS_EN[k])
}

/// "Di 10.11." / "Tue Nov 10".
pub fn short_date(d: NaiveDate) -> String {
    if is_german() {
        format!("{} {}.{}.", weekday_short(d), d.day(), d.month())
    } else {
        format!("{} {} {}", weekday_short(d), &month(d)[..3], d.day())
    }
}

/// "Heute · Di 10.11.", "Morgen · Mi 11.11." or "Do 12.11.".
pub fn day_title(d: NaiveDate, today: NaiveDate) -> String {
    let when = if d == today {
        Some(tr("Heute", "Today"))
    } else if d == today + Duration::days(1) {
        Some(tr("Morgen", "Tomorrow"))
    } else {
        None
    };
    match when {
        Some(w) => format!("{w} · {}", short_date(d)),
        None => short_date(d),
    }
}

/// "KW 46 · 9. bis 15. November" / "Week 46 · November 9 to 15", with both months when the week spans two.
pub fn week_title(monday: NaiveDate) -> String {
    let sunday = monday + Duration::days(6);
    let week = monday.iso_week().week();
    if is_german() {
        let from = if monday.month() == sunday.month() {
            format!("{}.", monday.day())
        } else {
            format!("{}. {}", monday.day(), month(monday))
        };
        format!("KW {week} · {from} bis {}. {}", sunday.day(), month(sunday))
    } else {
        let to = if monday.month() == sunday.month() {
            sunday.day().to_string()
        } else {
            format!("{} {}", month(sunday), sunday.day())
        };
        format!("Week {week} · {} {} to {to}", month(monday), monday.day())
    }
}

/// "10:00".
pub fn clock(t: NaiveTime) -> String {
    t.format("%H:%M").to_string()
}

/// The second line of an item's card: a problem, how a start runs, its deadline, or since when it is due.
pub fn item_text(it: &PlannedItem, today: NaiveDate) -> String {
    if let Some(p) = &it.problem {
        return format!("{} {p}", tr("Nicht gestartet:", "Did not start:"));
    }
    if it.time.is_some() {
        let starts = tr("Startet von selbst", "Starts by itself");
        return match &it.detail {
            Some(d) => format!("{starts} · {d}"),
            None => starts.to_owned(),
        };
    }
    if let Some(d) = it.deadline {
        return format!("{} {}", tr("Frist", "Deadline"), short_date(d));
    }
    match it.since_days {
        n if n > 1 => format!(
            "{} {n} {}",
            tr("Fällig seit", "Due for"),
            tr("Tagen", "days")
        ),
        1 => tr("Fällig seit gestern", "Due since yesterday").to_owned(),
        _ if it.day <= today => tr("Frei ab heute", "Due from today").to_owned(),
        _ => format!("{} {}", tr("Frei ab", "Due from"), short_date(it.day)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    const FILE: &str = r#"{"version": 1, "items": [
        {"kind": "start", "id": "a", "title": "Acme:  report", "project": "studio",
         "at": "2026-11-10T10:00:00+01:00", "detail": "claude · sonnet"},
        {"kind": "card", "title": "Shop files", "project": "shop", "date": "2026-11-05", "since_days": 5},
        {"kind": "card", "title": "Domain", "date": "2026-11-12", "deadline": "2026-11-13"},
        {"kind": "start", "title": "Backup", "at": "2026-11-09T09:50:00+01:00", "problem": "no window"},
        {"title": "ohne Tag"}, {"date": "2026-11-10"}, "kein Objekt",
        {"title": "falsches Datum", "date": "10.11.2026"}
    ]}"#;

    #[test]
    fn reads_starts_tasks_and_skips_broken_items() {
        let zurich = FixedOffset::east_opt(3600).unwrap();
        let items = parse_planned_in(FILE.as_bytes(), &zurich);
        assert_eq!(items.len(), 4, "{items:?}");
        assert_eq!(items[0].title, "Acme: report", "white space collapses");
        assert_eq!(
            (items[0].day, items[0].time.map(clock)),
            (d(2026, 11, 10), Some("10:00".into()))
        );
        assert_eq!(items[1].since_days, 5);
        assert_eq!(items[2].deadline, Some(d(2026, 11, 13)));
        assert_eq!(items[3].problem.as_deref(), Some("no window"));
        // In another zone the start moves with it.
        let utc = FixedOffset::east_opt(0).unwrap();
        assert_eq!(
            parse_planned_in(FILE.as_bytes(), &utc)[0].time.map(clock),
            Some("09:00".into())
        );
        // A bare list works too; anything else is empty.
        assert_eq!(
            parse_planned_in(br#"[{"title": "x", "date": "2026-11-10"}]"#, &utc).len(),
            1
        );
        assert!(parse_planned_in(b"{\"items\": 3}", &utc).is_empty());
        assert!(parse_planned_in(b"nonsense", &utc).is_empty());
    }

    #[test]
    fn days_weeks_and_order() {
        let zurich = FixedOffset::east_opt(3600).unwrap();
        let items = parse_planned_in(FILE.as_bytes(), &zurich);
        let today = d(2026, 11, 10); // a Tuesday
        assert_eq!(monday_of(today), d(2026, 11, 9));
        assert_eq!(monday_of(d(2026, 11, 9)), d(2026, 11, 9));
        // Overdue items stand on today: the start first (by time), then the task due longest.
        let on: Vec<&str> = items_on(&items, today, today)
            .iter()
            .map(|i| i.title.as_str())
            .collect();
        assert_eq!(on, ["Backup", "Acme: report", "Shop files"]);
        assert_eq!(deadlines_on(&items, d(2026, 11, 13), today).len(), 1);
        assert!(
            deadlines_on(&items, d(2026, 11, 12), today).is_empty(),
            "stands there itself"
        );
        assert_eq!(count_this_week(&items, today), 4);
        assert_eq!(
            count_this_week(&items, d(2026, 11, 16)),
            4,
            "next Monday: all overdue, on today"
        );
        // Week moves: back into this week = today, otherwise that Monday.
        assert_eq!(shift_week(today, 1, today), d(2026, 11, 16));
        assert_eq!(shift_week(d(2026, 11, 18), -1, today), today);
        assert_eq!(shift_week(today, -1, today), d(2026, 11, 2));
    }

    /// A read that failed is not kept: the next look reads again, though the file's time did not change (review quick
    /// 2026-11-10: it stayed an empty plan until another program changed the file).
    #[test]
    fn reload_reads_again_after_a_failed_read() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("lookout-planned-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("planned.json");
        std::fs::write(&file, r#"[{"title": "x", "date": "2026-11-10"}]"#).unwrap();
        let path = file.to_str().unwrap();
        let mode = |m| std::fs::set_permissions(&file, std::fs::Permissions::from_mode(m)).unwrap();
        mode(0o000);
        // Root reads it anyway; then only the rest is checked.
        let blocked = std::fs::read(&file).is_err();
        let first = reload(path, None);
        mode(0o644);
        if blocked {
            assert_eq!(first, None, "unreadable: nothing kept");
        }
        let read = reload(path, first);
        assert_eq!(read.as_ref().map(|c| c.1.len()), Some(1), "readable again");
        let at = read.unwrap().0;
        assert_eq!(
            reload(path, Some((at, vec![]))),
            Some((at, vec![])),
            "unchanged: not read again"
        );
        assert_eq!(reload(&format!("{path}.gone"), None), None, "missing");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn german_texts() {
        crate::i18n::set_german(true);
        let today = d(2026, 11, 10);
        assert_eq!(day_title(today, today), "Heute · Di 10.11.");
        assert_eq!(day_title(d(2026, 11, 11), today), "Morgen · Mi 11.11.");
        assert_eq!(day_title(d(2026, 11, 12), today), "Do 12.11.");
        assert_eq!(week_title(d(2026, 11, 9)), "KW 46 · 9. bis 15. November");
        assert_eq!(
            week_title(d(2026, 11, 30)),
            "KW 49 · 30. November bis 6. Dezember"
        );
        let zurich = FixedOffset::east_opt(3600).unwrap();
        let items = parse_planned_in(FILE.as_bytes(), &zurich);
        assert_eq!(
            item_text(&items[0], today),
            "Startet von selbst · claude · sonnet"
        );
        assert_eq!(item_text(&items[1], today), "Fällig seit 5 Tagen");
        assert_eq!(item_text(&items[2], today), "Frist Fr 13.11.");
        assert_eq!(item_text(&items[3], today), "Nicht gestartet: no window");
        let plain = |day, since| PlannedItem {
            day,
            since_days: since,
            deadline: None,
            ..items[1].clone()
        };
        assert_eq!(item_text(&plain(today, 1), today), "Fällig seit gestern");
        assert_eq!(item_text(&plain(today, 0), today), "Frei ab heute");
        assert_eq!(
            item_text(&plain(d(2026, 11, 12), 0), today),
            "Frei ab Do 12.11."
        );
    }
}
