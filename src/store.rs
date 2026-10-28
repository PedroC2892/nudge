use chrono::{DateTime, Duration, Local, Months, NaiveDate};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Freq {
    #[default]
    Never,
    Minutes,
    Hours,
    Days,
    Weeks,
    Months,
    Years,
}

impl Freq {
    pub const ALL: [Freq; 7] = [
        Freq::Never,
        Freq::Minutes,
        Freq::Hours,
        Freq::Days,
        Freq::Weeks,
        Freq::Months,
        Freq::Years,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Freq::Never => "Does not repeat",
            Freq::Minutes => "minutes",
            Freq::Hours => "hours",
            Freq::Days => "days",
            Freq::Weeks => "weeks",
            Freq::Months => "months",
            Freq::Years => "years",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Repeat {
    pub freq: Freq,
    pub every: u32,
    pub until: Option<NaiveDate>,
}

impl Default for Repeat {
    fn default() -> Self {
        Repeat { freq: Freq::Never, every: 1, until: None }
    }
}

impl Repeat {
    pub fn is_on(&self) -> bool {
        self.freq != Freq::Never
    }

    /// Next occurrence strictly after `from`, or None when the series ended.
    pub fn next(&self, from: DateTime<Local>) -> Option<DateTime<Local>> {
        let n = self.every.max(1);
        let next = match self.freq {
            Freq::Never => return None,
            Freq::Minutes => from + Duration::minutes(n.into()),
            Freq::Hours => from + Duration::hours(n.into()),
            Freq::Days => from + Duration::days(n.into()),
            Freq::Weeks => from + Duration::weeks(n.into()),
            Freq::Months => from.checked_add_months(Months::new(n))?,
            Freq::Years => from.checked_add_months(Months::new(n.checked_mul(12)?))?,
        };
        match self.until {
            Some(until) if next.date_naive() > until => None,
            _ => Some(next),
        }
    }

    /// First occurrence after `now`, skipping the ones missed while the computer was off.
    pub fn next_after(&self, mut due: DateTime<Local>, now: DateTime<Local>) -> Option<DateTime<Local>> {
        // ponytail: linear walk, a minutely reminder missed for a year is ~500k steps (still < 1s)
        loop {
            due = self.next(due)?;
            if due > now {
                return Some(due);
            }
        }
    }

    pub fn describe(&self) -> String {
        if !self.is_on() {
            return String::new();
        }
        let mut s = if self.every == 1 {
            format!("Every {}", self.freq.label().trim_end_matches('s'))
        } else {
            format!("Every {} {}", self.every, self.freq.label())
        };
        if let Some(u) = self.until {
            s += &format!(" until {}", u.format("%d %b %Y"));
        }
        s
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Reminder {
    pub id: u64,
    pub title: String,
    #[serde(default)]
    pub notes: String,
    pub due: Option<DateTime<Local>>,
    #[serde(default)]
    pub repeat: Repeat,
    #[serde(default)]
    pub important: bool,
    #[serde(default)]
    pub done: bool,
    /// Set once the last notification of this reminder was sent.
    #[serde(default)]
    pub notified: bool,
    pub list: Option<u64>,
    pub created: DateTime<Local>,
}

impl Reminder {
    pub fn new(id: u64, list: Option<u64>) -> Self {
        Reminder {
            id,
            title: String::new(),
            notes: String::new(),
            due: None,
            repeat: Repeat::default(),
            important: false,
            done: false,
            notified: false,
            list,
            created: Local::now(),
        }
    }

    pub fn is_overdue(&self, now: DateTime<Local>) -> bool {
        !self.done && self.due.is_some_and(|d| d <= now)
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct List {
    pub id: u64,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Settings {
    pub time_24h: bool,
    pub sound: bool,
    pub snooze_minutes: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { time_24h: true, sound: true, snooze_minutes: 10 }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct Data {
    pub reminders: Vec<Reminder>,
    pub lists: Vec<List>,
    #[serde(default)]
    pub settings: Settings,
    pub next_id: u64,
}

impl Data {
    pub fn new_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut Reminder> {
        self.reminders.iter_mut().find(|r| r.id == id)
    }

    /// Insert or replace a reminder by id.
    pub fn upsert(&mut self, r: Reminder) {
        match self.get_mut(r.id) {
            Some(old) => *old = r,
            None => self.reminders.push(r),
        }
    }

    /// Marks due reminders as notified (or moves repeating ones to their next
    /// occurrence) and returns copies of the ones that must be notified now.
    pub fn take_due(&mut self, now: DateTime<Local>) -> Vec<Reminder> {
        let mut fired = Vec::new();
        for r in &mut self.reminders {
            let Some(due) = r.due else { continue };
            if r.done || r.notified || due > now {
                continue;
            }
            fired.push(r.clone());
            match r.repeat.next_after(due, now) {
                Some(next) => r.due = Some(next),
                None => r.notified = true,
            }
        }
        fired
    }
}

/// JSON file shared by the window and the background service. Every write is
/// a locked read-modify-write so the two processes never overwrite each other.
pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn open() -> io::Result<Store> {
        let dir = dirs::data_dir()
            .ok_or_else(|| io::Error::other("no XDG data directory"))?
            .join("nudge");
        fs::create_dir_all(&dir)?;
        Ok(Store { path: dir.join("data.json") })
    }

    fn lock(&self) -> io::Result<File> {
        let f = File::create(self.path.with_extension("lock"))?;
        f.lock()?;
        Ok(f)
    }

    fn read(&self) -> io::Result<Data> {
        match fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).or_else(|e| {
                // Keep the broken file so nothing is lost, then start clean.
                let bak = self.path.with_extension(format!("corrupt-{}", Local::now().timestamp()));
                eprintln!("nudge: {} is invalid ({e}), moved to {}", self.path.display(), bak.display());
                fs::rename(&self.path, bak)?;
                Ok(Data::default())
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Data::default()),
            Err(e) => Err(e),
        }
    }

    pub fn load(&self) -> io::Result<Data> {
        let _lock = self.lock()?;
        self.read()
    }

    pub fn update<T>(&self, f: impl FnOnce(&mut Data) -> T) -> io::Result<(Data, T)> {
        let _lock = self.lock()?;
        let mut data = self.read()?;
        let out = f(&mut data);
        let tmp = self.path.with_extension("tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(&data).map_err(io::Error::other)?)?;
        fs::rename(&tmp, &self.path)?;
        Ok((data, out))
    }

    pub fn modified(&self) -> Option<std::time::SystemTime> {
        fs::metadata(&self.path).and_then(|m| m.modified()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(y: i32, m: u32, d: u32, h: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    #[test]
    fn repeat_and_until() {
        let r = Repeat { freq: Freq::Days, every: 2, until: NaiveDate::from_ymd_opt(2026, 1, 5) };
        assert_eq!(r.next(at(2026, 1, 1, 9)), Some(at(2026, 1, 3, 9)));
        assert_eq!(r.next(at(2026, 1, 3, 9)), Some(at(2026, 1, 5, 9)));
        assert_eq!(r.next(at(2026, 1, 5, 9)), None);
        let m = Repeat { freq: Freq::Months, every: 1, until: None };
        assert_eq!(m.next(at(2026, 1, 31, 9)), Some(at(2026, 2, 28, 9)));
    }

    #[test]
    fn take_due_skips_missed_occurrences() {
        let mut d = Data::default();
        let mut once = Reminder::new(1, None);
        once.due = Some(at(2026, 1, 1, 9));
        let mut daily = Reminder::new(2, None);
        daily.due = Some(at(2026, 1, 1, 9));
        daily.repeat = Repeat { freq: Freq::Days, every: 1, until: None };
        let mut future = Reminder::new(3, None);
        future.due = Some(at(2027, 1, 1, 9));
        d.reminders = vec![once, daily, future];

        let fired = d.take_due(at(2026, 1, 10, 12));
        assert_eq!(fired.iter().map(|r| r.id).collect::<Vec<_>>(), vec![1, 2]);
        assert!(d.reminders[0].notified);
        assert_eq!(d.reminders[1].due, Some(at(2026, 1, 11, 9)));
        assert!(!d.reminders[1].notified);
        assert!(d.take_due(at(2026, 1, 10, 12)).is_empty());
    }
}
