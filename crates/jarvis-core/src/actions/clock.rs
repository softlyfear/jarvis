// Time, date, timers, alarms, reminders and a stopwatch, all local.
// Pending timers are kept in timers.json in the config directory: Jarvis restarts whenever the
// settings change, and a reminder must survive that. Only jarvis-app runs the scheduler.

use std::path::PathBuf;
use std::time::Duration;

use chrono::{Datelike, Local, NaiveDateTime, TimeZone, Timelike};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use super::text::normalize;
use super::ActionError;
use crate::{assistant_config, APP_CONFIG_DIR};

const FILE_NAME: &str = "timers.json";
const MAX_TIMERS: usize = 20;
const MAX_SECONDS: u64 = 7 * 24 * 3600;

// ---------------------------------------------------------------- Russian words for numbers

const UNITS: [&str; 10] = ["ноль", "один", "два", "три", "четыре", "пять", "шесть", "семь", "восемь", "девять"];
const TEENS: [&str; 10] = [
    "десять", "одиннадцать", "двенадцать", "тринадцать", "четырнадцать", "пятнадцать", "шестнадцать",
    "семнадцать", "восемнадцать", "девятнадцать",
];
const TENS: [&str; 10] = ["", "", "двадцать", "тридцать", "сорок", "пятьдесят", "шестьдесят", "семьдесят", "восемьдесят", "девяносто"];
const HUNDREDS: [&str; 10] = ["", "сто", "двести", "триста", "четыреста", "пятьсот", "шестьсот", "семьсот", "восемьсот", "девятьсот"];

// 0..999 in words; `feminine` for "одна минута", "две секунды"
pub fn words(n: u32, feminine: bool) -> String {
    let n = n % 1000;
    if n == 0 {
        return UNITS[0].into();
    }
    let mut parts: Vec<&str> = Vec::new();
    if n >= 100 {
        parts.push(HUNDREDS[(n / 100) as usize]);
    }
    let rest = n % 100;
    if (10..20).contains(&rest) {
        parts.push(TEENS[(rest - 10) as usize]);
    } else {
        if rest >= 20 {
            parts.push(TENS[(rest / 10) as usize]);
        }
        match (rest % 10, feminine) {
            (0, _) => {}
            (1, true) => parts.push("одна"),
            (2, true) => parts.push("две"),
            (u, _) => parts.push(UNITS[u as usize]),
        }
    }
    parts.join(" ")
}

// "минута" / "минуты" / "минут"
pub fn plural<'a>(n: u32, one: &'a str, few: &'a str, many: &'a str) -> &'a str {
    if (11..=14).contains(&(n % 100)) {
        return many;
    }
    match n % 10 {
        1 => one,
        2..=4 => few,
        _ => many,
    }
}

fn counted(n: u32, feminine: bool, forms: (&str, &str, &str)) -> String {
    format!("{} {}", words(n, feminine), plural(n, forms.0, forms.1, forms.2))
}

const HOURS: (&str, &str, &str) = ("час", "часа", "часов");
const MINUTES: (&str, &str, &str) = ("минута", "минуты", "минут");
const SECONDS: (&str, &str, &str) = ("секунда", "секунды", "секунд");

// "девятнадцать часов сорок две минуты"
pub fn time_words(hour: u32, minute: u32) -> String {
    if minute == 0 {
        format!("ровно {}", counted(hour, false, HOURS))
    } else {
        format!("{} {}", counted(hour, false, HOURS), counted(minute, true, MINUTES))
    }
}

// "один час тридцать минут", "сорок пять секунд"
pub fn duration_words(total: u64) -> String {
    let (h, m, s) = ((total / 3600) as u32, ((total % 3600) / 60) as u32, (total % 60) as u32);
    let mut parts = Vec::new();
    if h > 0 {
        parts.push(counted(h, false, HOURS));
    }
    if m > 0 {
        parts.push(counted(m, true, MINUTES));
    }
    if s > 0 || parts.is_empty() {
        parts.push(counted(s, true, SECONDS));
    }
    parts.join(" ")
}

const ORDINALS: [&str; 20] = [
    "", "первое", "второе", "третье", "четвёртое", "пятое", "шестое", "седьмое", "восьмое", "девятое", "десятое",
    "одиннадцатое", "двенадцатое", "тринадцатое", "четырнадцатое", "пятнадцатое", "шестнадцатое", "семнадцатое",
    "восемнадцатое", "девятнадцатое",
];
const MONTHS: [&str; 12] = [
    "января", "февраля", "марта", "апреля", "мая", "июня", "июля", "августа", "сентября", "октября", "ноября", "декабря",
];
const WEEKDAYS: [&str; 7] = ["понедельник", "вторник", "среда", "четверг", "пятница", "суббота", "воскресенье"];

// "двадцать четвёртое"
fn day_words(day: u32) -> String {
    match day {
        1..=19 => ORDINALS[day as usize].into(),
        20 => "двадцатое".into(),
        30 => "тридцатое".into(),
        21..=29 => format!("двадцать {}", ORDINALS[(day - 20) as usize]),
        31 => "тридцать первое".into(),
        _ => day.to_string(),
    }
}

pub fn now_speech(now: NaiveDateTime) -> String {
    format!("Сейчас {}.", time_words(now.hour(), now.minute()))
}

pub fn date_speech(now: NaiveDateTime) -> String {
    let d = now.date();
    format!(
        "Сегодня {}, {} {}.",
        WEEKDAYS[d.weekday().num_days_from_monday() as usize],
        day_words(d.day()),
        MONTHS[d.month0() as usize]
    )
}

// ---------------------------------------------------------------- parsing spoken times

// "20 5" (normalize turns "двадцать пять" into two numbers) -> 25
fn numbers_joined(tokens: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in tokens {
        if let (Some(last), Ok(n)) = (out.last().and_then(|l| l.parse::<u32>().ok()), t.parse::<u32>()) {
            if last >= 20 && last % 10 == 0 && last < 100 && n < 10 {
                *out.last_mut().unwrap() = (last + n).to_string();
                continue;
            }
        }
        out.push(t.clone());
    }
    out
}

fn tokens(phrase: &str) -> Vec<String> {
    numbers_joined(&normalize(phrase).split_whitespace().map(String::from).collect::<Vec<_>>())
}

fn unit_seconds(word: &str) -> Option<u64> {
    if word.starts_with("сек") {
        Some(1)
    } else if word.starts_with("мин") {
        Some(60)
    } else if word.starts_with("час") {
        Some(3600)
    } else {
        None
    }
}

// the first duration in the phrase ("на 5 минут", "полчаса", "час 30 минут") and the token
// range it takes
fn find_duration(t: &[String]) -> Option<(u64, usize, usize)> {
    let mut total: u64 = 0;
    let mut start: Option<usize> = None;
    let mut end = 0;
    let mut i = 0;
    while i < t.len() {
        let begin = i;
        let w = t[i].as_str();
        let fixed = match w {
            "полчаса" => Some(1800),
            "полминуты" => Some(30),
            "четверть" if t.get(i + 1).is_some_and(|n| n.starts_with("час")) => {
                i += 1;
                Some(900)
            }
            _ => None,
        };
        let (amount, used) = if let Some(f) = fixed {
            (Some(f), 1)
        } else if let Ok(n) = w.parse::<u64>() {
            match t.get(i + 1).and_then(|u| unit_seconds(u)) {
                Some(unit) => (Some(n * unit), 2),
                // "таймер на 5" means minutes
                None if start.is_none() && t.get(i + 1).is_none() => (Some(n * 60), 1),
                None => (None, 1),
            }
        } else if matches!(w, "полтора" | "полторы") {
            match t.get(i + 1).and_then(|u| unit_seconds(u)) {
                Some(unit) => (Some(unit * 3 / 2), 2),
                None => (None, 1),
            }
        } else {
            // a bare unit: "через минуту", "на час"
            (unit_seconds(w).filter(|_| w != "часов" && w != "секунд" && w != "минут"), 1)
        };
        match amount {
            Some(a) => {
                start.get_or_insert(begin);
                total += a;
                i += used;
                end = i;
            }
            None if start.is_some() => break,
            None => i += 1,
        }
    }
    start.filter(|_| total > 0).map(|s| (total, s, end))
}

// "в 7 30 утра", "на 19 00", "в 8 вечера" -> (hour, minute) and the token range
fn find_clock_time(t: &[String]) -> Option<(u32, u32, usize, usize)> {
    for i in 0..t.len() {
        let hour_at = if matches!(t[i].as_str(), "в" | "на" | "к") { i + 1 } else { continue };
        let Some(mut hour) = t.get(hour_at).and_then(|h| h.parse::<u32>().ok()).filter(|h| *h < 24) else { continue };
        // "8 часов" / "8 утра" are clearly clock times; a bare number needs a unit-free context
        if t.get(hour_at + 1).is_some_and(|u| u.starts_with("мин") || u.starts_with("сек")) {
            continue;
        }
        let mut end = hour_at + 1;
        if t.get(end).is_some_and(|w| w.starts_with("час")) {
            end += 1;
        }
        let mut minute = 0;
        if let Some(m) = t.get(end).and_then(|m| m.parse::<u32>().ok()).filter(|m| *m < 60) {
            minute = m;
            end += 1;
            if t.get(end).is_some_and(|w| w.starts_with("мин")) {
                end += 1;
            }
        }
        match t.get(end).map(|s| s.as_str()) {
            Some("утра") => {
                if hour == 12 {
                    hour = 0;
                }
                end += 1;
            }
            Some("дня") | Some("вечера") => {
                if hour < 12 {
                    hour += 12;
                }
                end += 1;
            }
            Some("ночи") => {
                if hour == 12 {
                    hour = 0;
                }
                end += 1;
            }
            _ => {}
        }
        return Some((hour, minute, i, end));
    }
    None
}

// seconds from `now` to the next hour:minute (tomorrow if it has passed today)
pub fn seconds_until(now: NaiveDateTime, hour: u32, minute: u32) -> u64 {
    let today = now.date().and_hms_opt(hour, minute, 0).expect("valid time");
    let target = if today > now { today } else { today + chrono::Duration::days(1) };
    (target - now).num_seconds().max(1) as u64
}

const LABEL_FILLERS: &[&str] = &["мне", "что", "чтобы", "о", "об", "про", "том", "нужно", "надо", "пожалуйста"];

fn label_after(t: &[String], end: usize) -> String {
    let rest = &t[end.min(t.len())..];
    let start = rest.iter().position(|w| !LABEL_FILLERS.contains(&w.as_str())).unwrap_or(rest.len());
    rest[start..].join(" ")
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Timer,
    Alarm,
    Reminder,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "timer" => Some(Kind::Timer),
            "alarm" => Some(Kind::Alarm),
            "reminder" => Some(Kind::Reminder),
            _ => None,
        }
    }
}

// "поставь таймер на 5 минут", "разбуди в 7 30", "напомни через час позвонить маме"
pub fn parse_request(kind: Kind, phrase: &str, now: NaiveDateTime) -> Result<(u64, String), ActionError> {
    let t = tokens(phrase);
    let by_duration = find_duration(&t).map(|(secs, _, end)| (secs, end));
    let by_clock = find_clock_time(&t).map(|(h, m, _, end)| (seconds_until(now, h, m), end));
    let found = match kind {
        Kind::Timer => by_duration,
        Kind::Alarm => by_clock.or(by_duration),
        // "через ..." is a duration, "в ..." a clock time
        Kind::Reminder => {
            if t.iter().any(|w| w == "через") {
                by_duration.or(by_clock)
            } else {
                by_clock.or(by_duration)
            }
        }
    };
    let (seconds, end) = found.ok_or_else(|| ActionError::NotFound("не расслышал время".into()))?;
    if seconds > MAX_SECONDS {
        return Err(ActionError::Denied("можно ставить не больше чем на неделю".into()));
    }
    let label = if kind == Kind::Reminder { label_after(&t, end) } else { String::new() };
    if kind == Kind::Reminder && label.is_empty() {
        return Err(ActionError::NotFound("не расслышал, о чём напомнить".into()));
    }
    Ok((seconds, label))
}

// ---------------------------------------------------------------- scheduler

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    // unix seconds
    pub due: i64,
    pub kind: Kind,
    #[serde(default)]
    pub text: String,
    // what was asked, for the announcement: "таймер на пять минут"
    #[serde(default)]
    pub seconds: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Saved {
    timers: Vec<Entry>,
    // unix seconds when the stopwatch was started
    stopwatch: Option<i64>,
}

static STATE: Lazy<Mutex<Saved>> = Lazy::new(|| Mutex::new(Saved::default()));
static SCHEDULER: Lazy<()> = Lazy::new(|| {
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_secs(1));
        let due: Vec<Entry> = {
            let mut st = STATE.lock();
            let now = Local::now().timestamp();
            let (fired, left): (Vec<Entry>, Vec<Entry>) = st.timers.drain(..).partition(|e| e.due <= now);
            st.timers = left;
            if !fired.is_empty() {
                save(&st);
            }
            fired
        };
        for e in due {
            announce(&e);
        }
    });
});

fn file() -> Option<PathBuf> {
    APP_CONFIG_DIR.get().map(|d| d.join(FILE_NAME))
}

fn save(st: &Saved) {
    if let Some(p) = file() {
        match serde_json::to_string_pretty(st) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&p, json) {
                    warn!("Timers: cannot save {}: {}", p.display(), e);
                }
            }
            Err(e) => warn!("Timers: {}", e),
        }
    }
}

// jarvis-app at startup: timers set before a restart keep running; ones missed while
// Jarvis was off are announced at once
pub fn restore() {
    if let Some(p) = file() {
        if let Ok(text) = std::fs::read_to_string(&p) {
            match serde_json::from_str::<Saved>(&text) {
                Ok(saved) => {
                    info!("Timers: {} restored", saved.timers.len());
                    *STATE.lock() = saved;
                }
                Err(e) => warn!("Timers: {} is broken ({}), starting empty", p.display(), e),
            }
        }
    }
    Lazy::force(&SCHEDULER);
}

fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

pub fn announcement(e: &Entry, address: &str) -> String {
    let a = capitalized(address);
    match e.kind {
        Kind::Timer => format!("{}, время вышло: таймер на {}.", a, duration_words(e.seconds)),
        Kind::Alarm => {
            let at = Local.timestamp_opt(e.due, 0).single().map(|d| d.naive_local());
            let when = at.map(|d| format!(" Сейчас {}.", time_words(d.hour(), d.minute()))).unwrap_or_default();
            format!("{}, будильник.{} Пора.", a, when)
        }
        Kind::Reminder => format!("{}, напоминаю: {}.", a, e.text),
    }
}

fn announce(e: &Entry) {
    let text = announcement(e, &assistant_config::address());
    info!("Timer fired: {:?}", e);
    super::platform::notify("Джарвис", &text);
    #[cfg(feature = "reqwest")]
    {
        // an alarm is said twice: the first time may go unheard
        let times = if e.kind == Kind::Alarm { 2 } else { 1 };
        for _ in 0..times {
            crate::tts::speak(&text);
        }
    }
}

pub fn add(kind: Kind, seconds: u64, text: &str) -> Result<String, ActionError> {
    if seconds == 0 || seconds > MAX_SECONDS {
        return Err(ActionError::Denied("можно ставить от секунды до недели".into()));
    }
    Lazy::force(&SCHEDULER);
    let now = Local::now();
    let entry = Entry { due: now.timestamp() + seconds as i64, kind, text: text.trim().to_string(), seconds };
    {
        let mut st = STATE.lock();
        if st.timers.len() >= MAX_TIMERS {
            return Err(ActionError::Denied("слишком много таймеров, отмените старые".into()));
        }
        st.timers.push(entry.clone());
        save(&st);
    }
    let due = (now + chrono::Duration::seconds(seconds as i64)).naive_local();
    Ok(match kind {
        Kind::Timer => format!("Таймер на {} поставлен.", duration_words(seconds)),
        Kind::Alarm => format!("Будильник на {}.", time_words(due.hour(), due.minute())),
        Kind::Reminder => format!("Напомню через {}.", duration_words(seconds)),
    })
}

pub fn left_speech() -> String {
    let st = STATE.lock();
    let now = Local::now().timestamp();
    let Some(next) = st.timers.iter().min_by_key(|e| e.due) else {
        return "Таймеров нет.".into();
    };
    let what = match next.kind {
        Kind::Timer => "до конца таймера",
        Kind::Alarm => "до будильника",
        Kind::Reminder => "до напоминания",
    };
    let more = if st.timers.len() > 1 { format!(" Всего таймеров: {}.", words(st.timers.len() as u32, false)) } else { String::new() };
    format!("{} {}.{}", capitalized(what), duration_words((next.due - now).max(0) as u64), more)
}

pub fn cancel_all() -> String {
    let mut st = STATE.lock();
    let n = st.timers.len();
    st.timers.clear();
    save(&st);
    if n == 0 { "Таймеров не было.".into() } else { "Все таймеры и напоминания отменены.".into() }
}

pub fn stopwatch_start() -> String {
    let mut st = STATE.lock();
    st.stopwatch = Some(Local::now().timestamp());
    save(&st);
    "Засекаю.".into()
}

pub fn stopwatch_stop() -> String {
    let mut st = STATE.lock();
    match st.stopwatch.take() {
        Some(started) => {
            save(&st);
            format!("Прошло {}.", duration_words((Local::now().timestamp() - started).max(0) as u64))
        }
        None => "Секундомер не запущен.".into(),
    }
}

// Action::Clock { what }
pub const CLOCK_QUERIES: &[&str] = &["time", "date", "left", "cancel", "stopwatch_start", "stopwatch_stop"];

pub fn query(what: &str) -> Result<String, ActionError> {
    let now = Local::now().naive_local();
    Ok(match what {
        "time" => now_speech(now),
        "date" => date_speech(now),
        "left" => left_speech(),
        "cancel" => cancel_all(),
        "stopwatch_start" => stopwatch_start(),
        "stopwatch_stop" => stopwatch_stop(),
        other => return Err(ActionError::Failed(format!("unknown clock query: {}", other))),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(h: u32, m: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 9, 24).unwrap().and_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn numbers_are_spoken_in_russian() {
        assert_eq!(words(0, false), "ноль");
        assert_eq!(words(21, false), "двадцать один");
        assert_eq!(words(42, true), "сорок две");
        assert_eq!(words(115, false), "сто пятнадцать");
        assert_eq!(plural(1, "минута", "минуты", "минут"), "минута");
        assert_eq!(plural(3, "минута", "минуты", "минут"), "минуты");
        assert_eq!(plural(11, "минута", "минуты", "минут"), "минут");
        assert_eq!(plural(22, "минута", "минуты", "минут"), "минуты");
    }

    #[test]
    fn time_and_date_speech() {
        assert_eq!(now_speech(at(19, 42)), "Сейчас девятнадцать часов сорок две минуты.");
        assert_eq!(now_speech(at(21, 1)), "Сейчас двадцать один час одна минута.");
        assert_eq!(now_speech(at(7, 0)), "Сейчас ровно семь часов.");
        assert_eq!(date_speech(at(12, 0)), "Сегодня четверг, двадцать четвёртое сентября.");
        assert_eq!(duration_words(5400), "один час тридцать минут");
        assert_eq!(duration_words(45), "сорок пять секунд");
        assert_eq!(duration_words(0), "ноль секунд");
    }

    #[test]
    fn timers_parse_durations() {
        let now = at(12, 0);
        let p = |s: &str| parse_request(Kind::Timer, s, now).map(|r| r.0);
        assert_eq!(p("поставь таймер на пять минут"), Ok(300));
        assert_eq!(p("таймер на двадцать пять минут"), Ok(1500));
        assert_eq!(p("таймер на полчаса"), Ok(1800));
        assert_eq!(p("таймер на полторы минуты"), Ok(90));
        assert_eq!(p("таймер на час тридцать минут"), Ok(5400));
        assert_eq!(p("таймер на 10 секунд"), Ok(10));
        assert_eq!(p("таймер на минуту"), Ok(60));
        assert_eq!(p("таймер на пять"), Ok(300));
        assert!(matches!(p("поставь таймер"), Err(ActionError::NotFound(_))));
    }

    #[test]
    fn alarms_parse_clock_times() {
        let now = at(19, 42);
        let p = |s: &str| parse_request(Kind::Alarm, s, now).map(|r| r.0);
        // tomorrow morning
        assert_eq!(p("разбуди меня в семь утра"), Ok(seconds_until(now, 7, 0)));
        assert_eq!(seconds_until(now, 7, 0), (11 * 60 + 18) * 60);
        assert_eq!(p("будильник на 7 30"), Ok(seconds_until(now, 7, 30)));
        assert_eq!(p("будильник на восемь вечера"), Ok(18 * 60));
        assert_eq!(p("будильник через 20 минут"), Ok(1200));
    }

    #[test]
    fn reminders_keep_their_text() {
        let now = at(12, 0);
        assert_eq!(
            parse_request(Kind::Reminder, "напомни через 20 минут выключить духовку", now),
            Ok((1200, "выключить духовку".into()))
        );
        assert_eq!(
            parse_request(Kind::Reminder, "напомни мне в 15 часов что нужно позвонить маме", now),
            Ok((3 * 3600, "позвонить маме".into()))
        );
        assert!(matches!(parse_request(Kind::Reminder, "напомни через час", now), Err(ActionError::NotFound(_))));
        assert!(matches!(parse_request(Kind::Timer, "таймер на 200 часов", now), Err(ActionError::Denied(_))));
    }

    #[test]
    fn announcements_address_the_user() {
        let e = Entry { due: 0, kind: Kind::Timer, text: String::new(), seconds: 300 };
        assert_eq!(announcement(&e, "мисс"), "Мисс, время вышло: таймер на пять минут.");
        let r = Entry { due: 0, kind: Kind::Reminder, text: "позвонить маме".into(), seconds: 60 };
        assert_eq!(announcement(&r, "сэр"), "Сэр, напоминаю: позвонить маме.");
    }
}
