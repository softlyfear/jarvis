// Native PC actions shared by voice commands (type = "action" in command.toml)
// and by LLM tool calls. Dangerous actions go through a spoken yes/no confirmation.

pub mod apps;
pub mod clock;
pub mod confirm;
pub mod files;
pub mod input;
pub mod pc;
pub mod platform;
pub mod steam;
pub mod system;
pub mod text;

use std::fmt;

use crate::assistant_config;

#[derive(Debug, Clone, PartialEq)]
pub enum ActionError {
    // nothing matched the spoken name; the caller may fall back to the LLM
    NotFound(String),
    // refused by safety rules
    Denied(String),
    Failed(String),
    Unsupported,
}

impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActionError::NotFound(m) => write!(f, "{}", m),
            ActionError::Denied(m) => write!(f, "{}", m),
            ActionError::Failed(m) => write!(f, "ошибка: {}", m),
            ActionError::Unsupported => write!(f, "не поддерживается на этой системе"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    OpenApp { name: String },
    CloseApp { name: String },
    LaunchGame { name: String },
    ListGames,
    OpenFolder { name: String },
    FindFiles { query: String, folder: Option<String> },
    DeleteFile { path: String },
    CreateFolder { path: String },
    VolumeUp { percent: u32 },
    VolumeDown { percent: u32 },
    SetVolume { level: u32 },
    ToggleMute,
    Media { action: String },
    Screenshot,
    Snip,
    ShowDesktop,
    Lock,
    Sleep,
    Shutdown,
    Restart,
    CancelShutdown,
    EmptyRecycleBin,
    WebSearch { query: String },
    OpenUrl { url: String },
    // a named shortcut from input::NAMED_HOTKEYS or a combination like "ctrl+w"
    Hotkey { keys: String },
    // minimize | maximize | restore | close, for the window in front
    Window { action: String },
    TypeText { text: String },
    // bring a running program's window to the front
    FocusApp { name: String },
    // clock::CLOCK_QUERIES: time, date, timers left, cancel, stopwatch
    Clock { what: String },
    SetTimer { kind: clock::Kind, seconds: u64, text: String },
    // pc::INFO_QUERIES: cpu, memory, disk, battery, uptime
    Info { what: String },
    // an absolute level, or a step up/down
    Brightness { level: Option<u32>, delta: i32 },
    // pc::SITES: youtube, music, maps, wiki, translate
    SiteSearch { site: String, query: String },
    AddNote { text: String },
    // pc::NOTE_QUERIES: read, open
    Notes { what: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActionOutcome {
    // keep listening after the action (e.g. waiting for "yes/no")
    pub chain: bool,
    // text to speak, if any (Jarvis sound packs cover the plain "done" case)
    pub speech: Option<String>,
    // result for the LLM
    pub report: String,
}

impl ActionOutcome {
    fn done(report: impl Into<String>) -> Self {
        Self { chain: false, speech: None, report: report.into() }
    }

    // the answer is the result: "Сейчас семь часов"
    fn said(speech: String) -> Self {
        Self { chain: false, speech: Some(speech.clone()), report: speech }
    }
}

impl Action {
    pub fn is_dangerous(&self) -> bool {
        matches!(
            self,
            Action::DeleteFile { .. } | Action::Shutdown | Action::Restart | Action::Sleep | Action::EmptyRecycleBin
        )
    }

    // question asked before a dangerous action
    pub fn confirmation_question(&self) -> String {
        match self {
            Action::DeleteFile { path } => {
                let name = std::path::Path::new(path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.clone());
                format!("Переместить «{}» в корзину? Скажите да или нет.", name)
            }
            Action::Shutdown => "Выключить компьютер? Скажите да или нет.".into(),
            Action::Restart => "Перезагрузить компьютер? Скажите да или нет.".into(),
            Action::Sleep => "Перевести компьютер в сон? Скажите да или нет.".into(),
            Action::EmptyRecycleBin => "Очистить корзину безвозвратно? Скажите да или нет.".into(),
            other => format!("Выполнить {:?}? Скажите да или нет.", other),
        }
    }

    // run immediately, without confirmation checks
    pub fn execute(&self) -> Result<ActionOutcome, ActionError> {
        match self {
            Action::OpenApp { name } => apps::open(name).map(|n| ActionOutcome::done(format!("открыто: {}", n))),
            Action::CloseApp { name } => apps::close(name).map(|n| ActionOutcome::done(format!("закрыто: {}", n))),
            Action::LaunchGame { name } => apps::launch_game(name).map(|n| ActionOutcome::done(format!("запущено: {}", n))),
            Action::ListGames => {
                let games: Vec<String> = steam::games().into_iter().map(|g| g.name).collect();
                if games.is_empty() {
                    Ok(ActionOutcome::done("игр Steam не найдено"))
                } else {
                    Ok(ActionOutcome::done(format!("установленные игры Steam: {}", games.join(", "))))
                }
            }
            Action::OpenFolder { name } => files::open_folder(name).map(|n| ActionOutcome::done(format!("открыта папка: {}", n))),
            Action::FindFiles { query, folder } => {
                let found = files::find(query, folder.as_deref())?;
                if found.is_empty() {
                    Err(ActionError::NotFound(format!("файлов по запросу «{}» не найдено", query)))
                } else {
                    let list: Vec<String> = found.iter().map(|f| f.path.display().to_string()).collect();
                    Ok(ActionOutcome::done(format!("найдено: {}", list.join(" | "))))
                }
            }
            Action::DeleteFile { path } => {
                let p = files::check_deletable(path)?;
                files::delete_to_recycle_bin(&p)?;
                Ok(ActionOutcome::done(format!("перемещено в корзину: {}", p.display())))
            }
            Action::CreateFolder { path } => files::create_folder(path).map(|p| ActionOutcome::done(format!("создана папка: {}", p.display()))),
            Action::VolumeUp { percent } => system::volume_up(*percent).map(|_| ActionOutcome::done("громкость увеличена")),
            Action::VolumeDown { percent } => system::volume_down(*percent).map(|_| ActionOutcome::done("громкость уменьшена")),
            Action::SetVolume { level } => system::set_volume(*level).map(|_| ActionOutcome::done(format!("громкость {}%", level))),
            Action::ToggleMute => system::toggle_mute().map(|_| ActionOutcome::done("звук переключён")),
            Action::Media { action } => system::media(action).map(|_| ActionOutcome::done("готово")),
            Action::Screenshot => system::screenshot().map(|_| ActionOutcome::done("скриншот сохранён в Изображения\\Снимки экрана")),
            Action::Snip => system::snip().map(|_| ActionOutcome::done("выделите область")),
            Action::ShowDesktop => system::show_desktop().map(|_| ActionOutcome::done("готово")),
            Action::Lock => system::lock().map(|_| ActionOutcome::done("компьютер заблокирован")),
            Action::Sleep => system::sleep().map(|_| ActionOutcome::done("сон")),
            Action::Shutdown => system::shutdown().map(|_| ActionOutcome::done("выключение через 10 секунд")),
            Action::Restart => system::restart().map(|_| ActionOutcome::done("перезагрузка через 10 секунд")),
            Action::CancelShutdown => system::cancel_shutdown().map(|_| ActionOutcome::done("выключение отменено")),
            Action::EmptyRecycleBin => system::empty_recycle_bin().map(|_| ActionOutcome::done("корзина очищена")),
            Action::WebSearch { query } => system::web_search(query).map(|_| ActionOutcome::done(format!("ищу: {}", query))),
            Action::OpenUrl { url } => system::open_url(url).map(|_| ActionOutcome::done(format!("открыто: {}", url))),
            Action::Hotkey { keys } => input::press(keys).map(|_| ActionOutcome::done(format!("нажато: {}", keys))),
            Action::Window { action } => input::window(action).map(|_| ActionOutcome::done("готово")),
            Action::TypeText { text } => input::type_text(text).map(|_| ActionOutcome::done("текст напечатан")),
            Action::FocusApp { name } => input::focus_app(name).map(|t| ActionOutcome::done(format!("на экране: {}", t))),
            Action::Clock { what } => clock::query(what).map(ActionOutcome::said),
            Action::SetTimer { kind, seconds, text } => clock::add(*kind, *seconds, text).map(ActionOutcome::said),
            Action::Info { what } => pc::info(what).map(ActionOutcome::said),
            Action::Brightness { level, delta } => pc::brightness(*level, *delta).map(ActionOutcome::said),
            Action::SiteSearch { site, query } => pc::site_search(site, query).map(|_| ActionOutcome::done(format!("открыт поиск: {}", query))),
            Action::AddNote { text } => pc::add_note_to(&pc::notes_path(), text).map(|_| ActionOutcome::said("Записал.".into())),
            Action::Notes { what } => pc::notes(what).map(|t| if t.is_empty() { ActionOutcome::done("заметки открыты") } else { ActionOutcome::said(t) }),
        }
    }

    // execute, or ask for confirmation first when the action is dangerous
    pub fn run(self) -> Result<ActionOutcome, ActionError> {
        if self.is_dangerous() && assistant_config::get().safety.confirm_dangerous {
            // fail early (missing file, forbidden folder) instead of asking a pointless question
            if let Action::DeleteFile { path } = &self {
                files::check_deletable(path)?;
            }
            let question = self.confirmation_question();
            confirm::request(self);
            return Ok(ActionOutcome {
                chain: true,
                speech: Some(question.clone()),
                report: format!("ожидает подтверждения пользователя: {}", question),
            });
        }
        self.execute()
    }
}

// build an action for a voice command from command.toml:
//   type = "action", action = "open_app", phrases.ru = ["открой {app}", ...]
pub fn from_voice_command(action_id: &str, phrase: &str, templates: &[String], args: &std::collections::HashMap<String, String>) -> Result<Action, ActionError> {
    let object = || {
        let o = text::extract_object(phrase, templates);
        if o.is_empty() {
            Err(ActionError::NotFound("не расслышал название".into()))
        } else {
            Ok(o)
        }
    };
    let percent = |default: u32| text::extract_number(phrase).unwrap_or(default).min(100);

    let action = match action_id {
        "open_app" => Action::OpenApp { name: object()? },
        "close_app" => Action::CloseApp { name: object()? },
        "focus_app" => Action::FocusApp { name: object()? },
        "launch_game" => Action::LaunchGame { name: object()? },
        "list_games" => Action::ListGames,
        "open_folder" => Action::OpenFolder { name: object()? },
        "volume_up" => Action::VolumeUp { percent: percent(10) },
        "volume_down" => Action::VolumeDown { percent: percent(10) },
        "set_volume" => Action::SetVolume {
            level: text::extract_number(phrase)
                .ok_or_else(|| ActionError::NotFound("не расслышал уровень громкости".into()))?
                .min(100),
        },
        "mute" => Action::ToggleMute,
        "media" => Action::Media { action: args.get("action").cloned().unwrap_or_else(|| "play_pause".into()) },
        "screenshot" => Action::Screenshot,
        "snip" => Action::Snip,
        "show_desktop" => Action::ShowDesktop,
        "lock" => Action::Lock,
        "sleep" => Action::Sleep,
        "shutdown" => Action::Shutdown,
        "restart" => Action::Restart,
        "cancel_shutdown" => Action::CancelShutdown,
        "empty_recycle_bin" => Action::EmptyRecycleBin,
        "web_search" => Action::WebSearch { query: object()? },
        "open_url" => Action::OpenUrl {
            url: args.get("url").cloned().ok_or_else(|| ActionError::Failed("action open_url needs args.url".into()))?,
        },
        "hotkey" => Action::Hotkey {
            keys: args.get("keys").cloned().ok_or_else(|| ActionError::Failed("action hotkey needs args.keys".into()))?,
        },
        "window" => Action::Window {
            action: args.get("action").cloned().ok_or_else(|| ActionError::Failed("action window needs args.action".into()))?,
        },
        "type_text" => Action::TypeText { text: input::sentence(&object()?) },
        "clock" => Action::Clock {
            what: args.get("what").cloned().ok_or_else(|| ActionError::Failed("action clock needs args.what".into()))?,
        },
        "info" => Action::Info {
            what: args.get("what").cloned().ok_or_else(|| ActionError::Failed("action info needs args.what".into()))?,
        },
        "brightness" => Action::Brightness { level: text::extract_number(phrase).map(|n| n.min(100)), delta: 0 },
        "brightness_up" => Action::Brightness { level: None, delta: percent(20) as i32 },
        "brightness_down" => Action::Brightness { level: None, delta: -(percent(20) as i32) },
        "site_search" => Action::SiteSearch {
            site: args.get("site").cloned().ok_or_else(|| ActionError::Failed("action site_search needs args.site".into()))?,
            query: object()?,
        },
        "add_note" => Action::AddNote { text: object()? },
        "notes" => Action::Notes {
            what: args.get("what").cloned().ok_or_else(|| ActionError::Failed("action notes needs args.what".into()))?,
        },
        "timer" | "alarm" | "reminder" => {
            let kind = clock::Kind::parse(action_id).expect("matched above");
            let (seconds, text) = clock::parse_request(kind, phrase, chrono::Local::now().naive_local())?;
            Action::SetTimer { kind, seconds, text }
        }
        other => return Err(ActionError::Failed(format!("unknown action: {}", other))),
    };
    Ok(action)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn voice_command_builds_actions() {
        let t = vec!["открой {app}".to_string()];
        let none = HashMap::new();
        assert_eq!(
            from_voice_command("open_app", "открой телеграм", &t, &none).unwrap(),
            Action::OpenApp { name: "телеграм".into() }
        );
        assert_eq!(
            from_voice_command("set_volume", "громкость пятьдесят", &[], &none).unwrap(),
            Action::SetVolume { level: 50 }
        );
        assert_eq!(
            from_voice_command("volume_up", "сделай громче", &[], &none).unwrap(),
            Action::VolumeUp { percent: 10 }
        );
        assert!(matches!(from_voice_command("open_app", "открой", &t, &none), Err(ActionError::NotFound(_))));
        let keys: HashMap<String, String> = [("keys".to_string(), "close_tab".to_string())].into();
        assert_eq!(from_voice_command("hotkey", "закрой вкладку", &[], &keys).unwrap(), Action::Hotkey { keys: "close_tab".into() });
        assert!(matches!(from_voice_command("hotkey", "x", &[], &none), Err(ActionError::Failed(_))));
        assert_eq!(
            from_voice_command("timer", "поставь таймер на пять минут", &[], &none).unwrap(),
            Action::SetTimer { kind: clock::Kind::Timer, seconds: 300, text: String::new() }
        );
        assert!(matches!(from_voice_command("reminder", "напомни", &[], &none), Err(ActionError::NotFound(_))));
        let youtube: HashMap<String, String> = [("site".to_string(), "youtube".to_string())].into();
        assert_eq!(
            from_voice_command("site_search", "включи на ютубе котиков", &["включи на ютубе {query}".to_string()], &youtube).unwrap(),
            Action::SiteSearch { site: "youtube".into(), query: "котиков".into() }
        );
        assert_eq!(from_voice_command("brightness", "яркость пятьдесят", &[], &none).unwrap(), Action::Brightness { level: Some(50), delta: 0 });
        assert_eq!(from_voice_command("brightness_down", "темнее", &[], &none).unwrap(), Action::Brightness { level: None, delta: -20 });
        let typing = vec!["напечатай {text}".to_string()];
        assert_eq!(
            from_voice_command("type_text", "напечатай привет как дела", &typing, &none).unwrap(),
            Action::TypeText { text: "Привет как дела".into() }
        );
        assert!(from_voice_command("rm_rf", "x", &[], &none).is_err());
    }

    #[test]
    fn dangerous_actions_are_marked() {
        assert!(Action::Shutdown.is_dangerous());
        assert!(Action::DeleteFile { path: "C:\\x".into() }.is_dangerous());
        assert!(!Action::OpenApp { name: "x".into() }.is_dangerous());
    }

    #[test]
    fn dangerous_action_waits_for_confirmation() {
        let _guard = confirm::TEST_LOCK.lock();
        confirm::clear();
        let out = Action::Shutdown.run().unwrap();
        assert!(out.chain);
        assert!(out.speech.unwrap().contains("да или нет"));
        assert_eq!(confirm::answer("нет"), confirm::Answer::Cancelled);
        assert_eq!(confirm::answer("нет"), confirm::Answer::NoPending);
    }
    #[test]
    fn bundled_command_packs_parse_and_reference_known_actions() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/commands");
        let mut action_commands = 0;
        // the same phrase in two commands: which one runs depends on the classifier's mood
        let mut phrases: HashMap<String, String> = HashMap::new();
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let file = entry.path().join("command.toml");
            if !file.exists() {
                continue;
            }
            let content = std::fs::read_to_string(&file).unwrap();
            let list: crate::commands::JCommandsList = toml::from_str(&content)
                .unwrap_or_else(|e| panic!("{} does not parse: {}", file.display(), e));
            for cmd in &list.commands {
                assert!(!cmd.get_phrases("ru").is_empty(), "{} has no ru phrases", cmd.id);
                for p in cmd.get_phrases("ru").iter() {
                    let key = p.to_lowercase().replace('ё', "е");
                    if let Some(other) = phrases.insert(key, cmd.id.clone()) {
                        panic!("phrase «{}» is in both {} and {}", p, other, cmd.id);
                    }
                }
                if cmd.cmd_type == "action" {
                    action_commands += 1;
                    // a phrase with an object so object-taking actions build too
                    if cmd.action == "hotkey" {
                        let keys = &cmd.args["keys"];
                        input::parse_combo(input::named_hotkey(keys).unwrap_or(keys))
                            .unwrap_or_else(|e| panic!("{}: {}", cmd.id, e));
                    }
                    let r = from_voice_command(&cmd.action, "открой громкость 50 тест", &[], &cmd.args);
                    assert!(
                        !matches!(r, Err(ActionError::Failed(_))),
                        "{}: unknown or misconfigured action {}: {:?}", cmd.id, cmd.action, r
                    );
                }
            }
        }
        assert!(action_commands >= 20, "only {} action commands found", action_commands);
    }
}
