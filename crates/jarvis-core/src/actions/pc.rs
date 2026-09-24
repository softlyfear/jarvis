// The computer's state spoken aloud (processor, memory, disks, battery, uptime), screen
// brightness, searches on popular sites and a plain-text notes file.

use std::path::PathBuf;

use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};

use super::clock::{duration_words, plural, words};
use super::{platform, ActionError};
use crate::assistant_config::expand_env;

const GB: f64 = 1024.0 * 1024.0 * 1024.0;

// "двенадцать гигабайт", "1500 гигабайт" (words go up to 999)
fn gigabytes(bytes: u64) -> String {
    let n = (bytes as f64 / GB).round() as u32;
    let number = if n < 1000 { words(n, false) } else { n.to_string() };
    format!("{} {}", number, plural(n, "гигабайт", "гигабайта", "гигабайт"))
}

fn percent(n: u32) -> String {
    format!("{} {}", words(n.min(100), false), plural(n, "процент", "процента", "процентов"))
}

pub fn cpu_speech(usage: f32) -> String {
    format!("Процессор загружен на {}.", percent(usage.round() as u32))
}

pub fn memory_speech(used: u64, total: u64) -> String {
    format!("Занято {} памяти, свободно {}.", gigabytes(used), gigabytes(total.saturating_sub(used)))
}

pub fn disk_speech(disks: &[(String, u64)]) -> String {
    if disks.is_empty() {
        return "Не нашёл дисков.".into();
    }
    let parts: Vec<String> = disks.iter().map(|(name, free)| format!("на диске {} свободно {}", name, gigabytes(*free))).collect();
    format!("{}.", super::input::sentence(&parts.join(", ")))
}

pub fn uptime_speech(seconds: u64) -> String {
    // whole minutes: "три часа двадцать минут", not "... и сорок секунд"
    let rounded = if seconds >= 60 { seconds / 60 * 60 } else { seconds };
    format!("Компьютер работает {}.", duration_words(rounded))
}

pub const INFO_QUERIES: &[&str] = &["cpu", "memory", "disk", "battery", "uptime"];

fn cpu_usage() -> f32 {
    use sysinfo::System;
    let mut sys = System::new();
    sys.refresh_cpu_usage();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL.max(std::time::Duration::from_millis(500)));
    sys.refresh_cpu_usage();
    sys.global_cpu_usage()
}

fn fixed_disks() -> Vec<(String, u64)> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let mut out: Vec<(String, u64)> = disks
        .list()
        .iter()
        .filter(|d| !d.is_removable() && d.total_space() > 0)
        .map(|d| {
            let mount = d.mount_point().to_string_lossy().to_string();
            // "C:\" -> "C"
            let name = mount.trim_end_matches(['\\', '/']).trim_end_matches(':').to_string();
            (if name.is_empty() { mount } else { name }, d.available_space())
        })
        .collect();
    out.sort();
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

fn battery_speech() -> Result<String, ActionError> {
    if !cfg!(windows) {
        return Err(ActionError::Unsupported);
    }
    let script = "$b = Get-CimInstance Win32_Battery | Select-Object -First 1; if ($b) { $b.EstimatedChargeRemaining }";
    let out = platform::powershell(script, &[]).map_err(ActionError::Failed)?;
    Ok(match out.trim().parse::<u32>() {
        Ok(level) => format!("Заряд батареи {}.", percent(level)),
        Err(_) => "Батареи нет: компьютер работает от сети.".into(),
    })
}

pub fn info(what: &str) -> Result<String, ActionError> {
    Ok(match what {
        "cpu" => cpu_speech(cpu_usage()),
        "memory" => {
            let mut sys = sysinfo::System::new();
            sys.refresh_memory();
            memory_speech(sys.used_memory(), sys.total_memory())
        }
        "disk" => disk_speech(&fixed_disks()),
        "battery" => battery_speech()?,
        "uptime" => uptime_speech(sysinfo::System::uptime()),
        other => return Err(ActionError::Failed(format!("unknown info query: {}", other))),
    })
}

// ---------------------------------------------------------------- brightness

// laptops and some monitors; a desktop monitor over HDMI/DisplayPort usually does not answer
pub fn brightness(level: Option<u32>, delta: i32) -> Result<String, ActionError> {
    if !cfg!(windows) {
        return Err(ActionError::Unsupported);
    }
    let script = "$ErrorActionPreference = 'Stop'; \
        $cur = (Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightness | Select-Object -First 1).CurrentBrightness; \
        $l = if ($env:JARVIS_LEVEL -ne '') { [int]$env:JARVIS_LEVEL } else { [int]$cur + [int]$env:JARVIS_DELTA }; \
        $l = [Math]::Max(0, [Math]::Min(100, $l)); \
        $m = Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightnessMethods | Select-Object -First 1; \
        Invoke-CimMethod -InputObject $m -MethodName WmiSetBrightness -Arguments @{ Timeout = [uint32]1; Brightness = [byte]$l } | Out-Null; $l";
    let level_s = level.map(|l| l.min(100).to_string()).unwrap_or_default();
    let delta_s = delta.to_string();
    match platform::powershell(script, &[("JARVIS_LEVEL", &level_s), ("JARVIS_DELTA", &delta_s)]) {
        Ok(out) => Ok(match out.trim().parse::<u32>() {
            Ok(l) => format!("Яркость {}.", percent(l)),
            Err(_) => "Готово.".into(),
        }),
        Err(e) => {
            warn!("Brightness: {}", e);
            Err(ActionError::Failed("яркость этого монитора Windows не меняет, только кнопками на самом мониторе".into()))
        }
    }
}

// ---------------------------------------------------------------- sites

// site -> search URL prefix; the query is appended percent-encoded
pub const SITES: &[(&str, &str)] = &[
    ("youtube", "https://www.youtube.com/results?search_query="),
    ("music", "https://music.yandex.ru/search?text="),
    ("maps", "https://yandex.ru/maps/?text="),
    ("wiki", "https://ru.wikipedia.org/w/index.php?search="),
    ("translate", "https://translate.yandex.ru/?text="),
];

pub fn site_url(site: &str, query: &str) -> Result<String, ActionError> {
    let prefix = SITES
        .iter()
        .find(|(s, _)| *s == site)
        .map(|(_, p)| *p)
        .ok_or_else(|| ActionError::Failed(format!("unknown site: {}", site)))?;
    let query = query.trim();
    if query.is_empty() {
        return Err(ActionError::NotFound("не расслышал, что найти".into()));
    }
    Ok(format!("{}{}", prefix, utf8_percent_encode(query, NON_ALPHANUMERIC)))
}

pub fn site_search(site: &str, query: &str) -> Result<(), ActionError> {
    platform::open_target(&site_url(site, query)?).map_err(ActionError::Failed)
}

// ---------------------------------------------------------------- notes

const NOTES_FILE: &str = "Заметки Джарвиса.txt";
const MAX_NOTE_CHARS: usize = 1000;

pub fn notes_path() -> PathBuf {
    let docs = expand_env("%USERPROFILE%\\Documents");
    if docs.starts_with('%') {
        // not Windows: the tests and Linux builds
        return std::env::temp_dir().join(NOTES_FILE);
    }
    PathBuf::from(docs).join(NOTES_FILE)
}

pub fn note_line(text: &str, now: chrono::NaiveDateTime) -> String {
    format!("{} — {}", now.format("%d.%m.%Y %H:%M"), super::input::sentence(text))
}

pub fn add_note_to(path: &std::path::Path, text: &str) -> Result<(), ActionError> {
    use std::io::Write;
    let text = text.trim();
    if text.is_empty() {
        return Err(ActionError::NotFound("не расслышал, что записать".into()));
    }
    if text.chars().count() > MAX_NOTE_CHARS {
        return Err(ActionError::Denied("слишком длинная заметка".into()));
    }
    let line = note_line(text, chrono::Local::now().naive_local());
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| ActionError::Failed(format!("{}: {}", path.display(), e)))?;
    // CRLF: the file is opened in Notepad
    write!(f, "{}\r\n", line).map_err(|e| ActionError::Failed(e.to_string()))
}

// the last notes, newest first, without their dates
pub fn last_notes_from(path: &std::path::Path, count: usize) -> Vec<String> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .rev()
        .filter(|l| !l.trim().is_empty())
        .take(count)
        .map(|l| l.split_once(" — ").map(|(_, t)| t).unwrap_or(l).trim().to_string())
        .collect()
}

pub const NOTE_QUERIES: &[&str] = &["read", "open"];

pub fn notes(what: &str) -> Result<String, ActionError> {
    let path = notes_path();
    match what {
        "read" => {
            let last = last_notes_from(&path, 3);
            if last.is_empty() {
                Ok("Заметок пока нет.".into())
            } else {
                Ok(format!("Последние заметки: {}.", last.join(". ").trim_end_matches('.')))
            }
        }
        "open" => {
            if !path.exists() {
                return Ok("Заметок пока нет.".into());
            }
            platform::open_target(&path.to_string_lossy()).map_err(ActionError::Failed)?;
            Ok(String::new())
        }
        other => Err(ActionError::Failed(format!("unknown notes query: {}", other))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_is_spoken_in_words() {
        assert_eq!(cpu_speech(23.4), "Процессор загружен на двадцать три процента.");
        assert_eq!(cpu_speech(100.0), "Процессор загружен на сто процентов.");
        let gb = 1024 * 1024 * 1024;
        assert_eq!(memory_speech(12 * gb, 32 * gb), "Занято двенадцать гигабайт памяти, свободно двадцать гигабайт.");
        assert_eq!(disk_speech(&[("C".into(), 121 * gb), ("D".into(), 2 * gb)]), "На диске C свободно сто двадцать один гигабайт, на диске D свободно два гигабайта.");
        assert_eq!(uptime_speech(3 * 3600 + 20 * 60 + 41), "Компьютер работает три часа двадцать минут.");
        assert_eq!(gigabytes(1800 * gb), "1800 гигабайт");
    }

    #[test]
    fn site_searches_are_encoded() {
        assert_eq!(site_url("youtube", "котики").unwrap(), "https://www.youtube.com/results?search_query=%D0%BA%D0%BE%D1%82%D0%B8%D0%BA%D0%B8");
        assert!(site_url("music", "a&b").unwrap().ends_with("a%26b"));
        assert!(matches!(site_url("youtube", " "), Err(ActionError::NotFound(_))));
        assert!(matches!(site_url("file", "x"), Err(ActionError::Failed(_))));
    }

    #[test]
    fn notes_are_appended_and_read_back() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("notes.txt");
        assert!(last_notes_from(&p, 3).is_empty());
        for t in ["купить молоко", "позвонить маме", "сдать реферат", "полить цветы"] {
            add_note_to(&p, t).unwrap();
        }
        assert_eq!(last_notes_from(&p, 3), vec!["Полить цветы", "Сдать реферат", "Позвонить маме"]);
        assert!(std::fs::read_to_string(&p).unwrap().contains(" — Купить молоко\r\n"));
        assert!(matches!(add_note_to(&p, "  "), Err(ActionError::NotFound(_))));
        let at = chrono::NaiveDate::from_ymd_opt(2026, 9, 24).unwrap().and_hms_opt(20, 5, 0).unwrap();
        assert_eq!(note_line("тест", at), "24.09.2026 20:05 — Тест");
    }
}
