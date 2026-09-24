use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::fs;
use std::time::Duration;
use std::process::{Child, Command};

use seqdiff::ratio;

mod structs;
pub use structs::*;

use crate::{config, i18n, APP_DIR};

#[cfg(feature = "lua")]
use crate::lua::{self, SandboxLevel, CommandContext};

pub fn parse_commands() -> Result<Vec<JCommandsList>, String> {
    let mut commands: Vec<JCommandsList> = Vec::new();

    let commands_path = APP_DIR.join(config::COMMANDS_PATH);
    let cmd_dirs = fs::read_dir(&commands_path)
        .map_err(|e| format!("Error reading commands directory {:?}: {}", commands_path, e))?;

    for entry in cmd_dirs.flatten() {
        let cmd_path = entry.path();
        let toml_file = cmd_path.join("command.toml");
        
        if !toml_file.exists() {
            continue;
        }
        
        let content = match fs::read_to_string(&toml_file) {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to read {}: {}", toml_file.display(), e);
                continue;
            }
        };

        let file: JCommandsList = match toml::from_str(&content) {
            Ok(f) => f,
            Err(e) => {
                warn!("Failed to parse {}: {}", toml_file.display(), e);
                continue;
            }
        };

        commands.push(JCommandsList {
            path: cmd_path,
            commands: file.commands,
        });
    }

    if commands.is_empty() {
        Err("No commands found".into())
    } else {
        info!("Loaded {} command pack(s)", commands.len());
        Ok(commands)
    }
}


pub fn commands_hash(commands: &[JCommandsList]) -> String {
    use sha2::{Sha256, Digest};
    
    let mut hasher = Sha256::new();
    
    let lang = i18n::get_language();
    hasher.update(lang.as_bytes());
    hasher.update(b"|");

    // collect all command ids and phrases for current language, sorted
    let mut all_data: Vec<(&str, _)> = commands.iter()
        .flat_map(|ac| ac.commands.iter().map(|c| (c.id.as_str(), c.get_phrases(&lang))))
        .collect();
    all_data.sort_by_key(|(id, _)| *id);
    
    for (id, phrases) in all_data {
        hasher.update(id.as_bytes());
        for phrase in phrases.iter() {
            hasher.update(phrase.as_bytes());
        }
    }
    
    format!("{:x}", hasher.finalize())
}


pub fn fetch_command<'a>(
    phrase: &str,
    commands: &'a [JCommandsList],
) -> Option<(&'a PathBuf, &'a JCommand)> {
    let lang = i18n::get_language();

    let phrase = crate::actions::text::tidy_command(phrase);
    if phrase.is_empty() {
        return None;
    }

    let phrase_chars: Vec<char> = phrase.chars().collect();
    let phrase_words: Vec<&str> = phrase.split_whitespace().collect();

    let mut result: Option<(&PathBuf, &JCommand)> = None;
    let mut best_score = config::CMD_RATIO_THRESHOLD;
    // "громкость {percent}" against "громкость пятьдесят": the literal part decides
    let mut template: Option<(usize, (&PathBuf, &JCommand))> = None;

    for cmd_list in commands {
        for cmd in &cmd_list.commands {
            let cmd_phrases = cmd.get_phrases(&lang);
            
            for cmd_phrase in cmd_phrases.iter() {
                let cmd_phrase_lower = cmd_phrase.trim().to_lowercase();
                if let Some(len) = template_match(&phrase, &cmd_phrase_lower) {
                    if template.as_ref().is_none_or(|(best, _)| len > *best) {
                        template = Some((len, (&cmd_list.path, cmd)));
                    }
                }
                let cmd_phrase_chars: Vec<char> = cmd_phrase_lower.chars().collect();
                
                // character-level similarity
                let char_ratio = ratio(&phrase_chars, &cmd_phrase_chars);
                
                // word-level similarity
                let cmd_words: Vec<&str> = cmd_phrase_lower.split_whitespace().collect();
                let word_score = word_overlap_score(&phrase_words, &cmd_words);
                
                // combined score
                let score = (char_ratio * 0.6) + (word_score * 0.4);
                
                // early exit on perfect match
                if score >= 99.0 {
                    debug!("Perfect match: '{}' -> '{}'", phrase, cmd_phrase_lower);
                    return Some((&cmd_list.path, cmd));
                }
                
                if score > best_score {
                    best_score = score;
                    result = Some((&cmd_list.path, cmd));
                }
            }
        }
    }

    // a close fixed phrase ("выключи звук") beats a template ("выключи {app}")
    if best_score < 90.0 {
        if let Some((_, (path, cmd))) = template {
            info!("Template match: '{}' -> cmd '{}'", phrase, cmd.id);
            return Some((path, cmd));
        }
    }

    if let Some((_, cmd)) = result {
        info!("Fuzzy match: '{}' -> cmd '{}' (score: {:.1}%)", phrase, cmd.id, best_score);
    } else {
        debug!("No match for '{}' (best: {:.1}%)", phrase, best_score);
    }
    
    result
}


// "громкость {percent}" matches "громкость пятьдесят" (and "джарис громкость пятьдесят": one
// leftover word before is allowed, a misheard wake word); returns the literal length
fn template_match(phrase: &str, template: &str) -> Option<usize> {
    let open = template.find('{')?;
    let close = template.rfind('}')?;
    let prefix: Vec<&str> = template[..open].split_whitespace().collect();
    let suffix: Vec<&str> = template[close + 1..].split_whitespace().collect();
    if prefix.is_empty() {
        return None;
    }
    let words: Vec<&str> = phrase.split_whitespace().collect();
    for skip in 0..=1 {
        let rest = words.get(skip..)?;
        if rest.len() > prefix.len() + suffix.len()
            && rest[..prefix.len()] == prefix[..]
            && rest[rest.len() - suffix.len()..] == suffix[..]
        {
            return Some(prefix.iter().chain(suffix.iter()).map(|w| w.chars().count()).sum());
        }
    }
    None
}

fn word_overlap_score(input_words: &[&str], cmd_words: &[&str]) -> f64 {
    if input_words.is_empty() || cmd_words.is_empty() {
        return 0.0;
    }

    let mut matched = 0.0;
    
    // pre-compute cmd word chars to avoid repeated allocations
    let cmd_word_chars: Vec<Vec<char>> = cmd_words
        .iter()
        .map(|w| w.chars().collect())
        .collect();
    
    for input_word in input_words {
        let input_chars: Vec<char> = input_word.chars().collect();
        
        let best_word_match = cmd_word_chars
            .iter()
            .map(|cw| ratio(&input_chars, cw))
            .fold(0.0_f64, f64::max);
        
        if best_word_match > 70.0 {
            matched += best_word_match / 100.0;
        }
    }

    let max_words = input_words.len().max(cmd_words.len()) as f64;
    (matched / max_words) * 100.0
}




pub fn execute_exe(exe: &str, args: &[String]) -> std::io::Result<Child> {
    Command::new(exe).args(args).spawn()
}

pub fn execute_cli(cmd: &str, args: &[String]) -> std::io::Result<Child> {
    debug!("Spawning: cmd /C {} {:?}", cmd, args);

    if cfg!(target_os = "windows") {
        Command::new("cmd").arg("/C").arg(cmd).args(args).spawn()
    } else {
        Command::new("sh").arg("-c").arg(cmd).args(args).spawn()
    }
}

pub fn execute_command(cmd_path: &PathBuf, cmd_config: &JCommand, phrase: Option<&str>, slots: Option<&HashMap<String, SlotValue>>) -> Result<bool, String> {
    // execute command by the type
    match cmd_config.cmd_type.as_str() {

        // BRUH
        "voice" => Ok(true),
        
        // LUA command
        #[cfg(feature = "lua")]
        "lua" => {
            execute_lua_command(cmd_path, cmd_config, phrase, slots)
        }

        // AutoHotkey command
        // @TODO: Consider adding ahk source files execution?
        "ahk" => {
            let exe_path_absolute = Path::new(&cmd_config.exe_path);
            let exe_path_local = cmd_path.join(&cmd_config.exe_path);

            let exe_path = if exe_path_absolute.exists() {
                exe_path_absolute
            } else {
                exe_path_local.as_path()
            };

            execute_exe(exe_path.to_str().unwrap(), &cmd_config.exe_args)
                .map(|_| true)
                .map_err(|e| format!("AHK process spawn error: {}", e))
        }
        
        // CLI command type
        // @TODO: Consider security restrictions
        "cli" => {
            execute_cli(&cmd_config.cli_cmd, &cmd_config.cli_args)
                .map(|_| true)
                .map_err(|e| format!("CLI command error: {}", e))
        }
        
        // native PC action (open/close apps, games, volume, files...)
        "action" => {
            let templates = cmd_config.get_phrases(&i18n::get_language());
            crate::actions::from_voice_command(&cmd_config.action, phrase.unwrap_or(""), &templates, &cmd_config.args)
                .and_then(|a| a.run())
                .map(|outcome| outcome.chain)
                .map_err(|e| e.to_string())
        }

        // TERMINATOR command (T1000)
        "terminate" => {
            std::thread::sleep(Duration::from_secs(2));
            std::process::exit(0);
        }
        
        // STOP CHANING
        "stop_chaining" => Ok(false),

        // other
        _ => {
            error!("Command type unknown: {}", cmd_config.cmd_type);
            Err(format!("Command type unknown: {}", cmd_config.cmd_type).into())
        }
    }
}

// look up a command by its ID
pub fn get_command_by_id<'a>(
    commands: &'a [JCommandsList],
    id: &str,
) -> Option<(&'a PathBuf, &'a JCommand)> {
    for cmd_list in commands {
        for cmd in &cmd_list.commands {
            if cmd.id == id {
                return Some((&cmd_list.path, cmd));
            }
        }
    }
    None
}

pub fn list_paths(commands: &[JCommandsList]) -> Vec<&Path> {
    commands.iter().map(|x| x.path.as_path()).collect()
}

#[cfg(feature = "lua")]
fn execute_lua_command(
    cmd_path: &PathBuf,
    cmd_config: &JCommand,
    phrase: Option<&str>,
    slots: Option<&HashMap<String, SlotValue>>
) -> Result<bool, String> {
    // get script path

    let script_name = if cmd_config.script.is_empty() {
        "script.lua"
    } else {
        &cmd_config.script
    };
    
    let script_path = cmd_path.join(script_name);
    
    if !script_path.exists() {
        return Err(format!("Lua script not found: {}", script_path.display()));
    }
    
    // parse sandbox level
    let sandbox = SandboxLevel::from_str(&cmd_config.sandbox);

    // create context
    let context = CommandContext {
        phrase: phrase.unwrap_or("").to_string(),
        command_id: cmd_config.id.clone(),
        command_path: cmd_path.clone(),
        language: i18n::get_language(),
        slots: slots.map(|s| s.clone()),
    };
    
    // get timeout
    let timeout = Duration::from_millis(cmd_config.timeout);
    
    info!("Executing Lua command: {} (sandbox: {:?}, timeout: {:?})", 
          cmd_config.id, sandbox, timeout);
    
    // execute
    match lua::execute(&script_path, context, sandbox, timeout) {
        Ok(result) => {
            info!("Lua command {} completed (chain: {})", cmd_config.id, result.chain);
            Ok(result.chain)
        }
        Err(e) => {
            error!("Lua command {} failed: {}", cmd_config.id, e);
            Err(e.to_string())
        }
    }
}
#[cfg(test)]
mod template_tests {
    use super::{fetch_command, template_match};
    use crate::JCommandsList;

    fn bundled_packs() -> Vec<JCommandsList> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/commands");
        let mut packs = Vec::new();
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let file = entry.path().join("command.toml");
            if let Ok(text) = std::fs::read_to_string(&file) {
                let mut list: JCommandsList = toml::from_str(&text).unwrap();
                list.path = entry.path();
                packs.push(list);
            }
        }
        packs
    }

    // spoken phrases (as Whisper gives them) against the real packs: short commands must not
    // swallow questions meant for the LLM
    #[test]
    fn real_phrases_find_the_right_command() {
        crate::i18n::init("ru");
        let packs = bundled_packs();
        let id = |phrase: &str| fetch_command(phrase, &packs).map(|(_, c)| c.id.clone());
        let cases: &[(&str, Option<&str>)] = &[
            ("так закрой вкладку", Some("close_tab")),
            ("откроем компьютер", Some("open_app")),
            ("закрой телеграм", Some("close_app")),
            ("сколько времени", Some("tell_time")),
            ("поставь таймер на пять минут", Some("set_timer")),
            ("разбуди меня в семь утра", Some("set_alarm")),
            ("напомни через час позвонить маме", Some("set_reminder")),
            ("включи музыку", Some("media_play_pause")),
            ("включи песню группа крови", Some("music_search")),
            ("включи на ютубе котиков", Some("youtube_search")),
            ("сверни окно", Some("window_minimize")),
            ("напечатай привет как дела", Some("type_text")),
            ("сколько места на диске", Some("info_disk")),
            ("запиши заметку купить хлеб", Some("note_add")),
            ("выключи звук", Some("mute")),
            ("отмени таймер", Some("timers_cancel")),
            ("что такое время", None),
            ("кто написал войну и мир", None),
            ("расскажи как дела у тебя", None),
            ("сколько будет семь умножить на восемь", None),
        ];
        let wrong: Vec<String> = cases
            .iter()
            .filter(|(p, want)| id(p).as_deref() != *want)
            .map(|(p, want)| format!("«{}»: {:?}, want {:?}", p, id(p), want))
            .collect();
        assert!(wrong.is_empty(), "{:#?}", wrong);
    }

    #[test]
    fn templates_match_by_their_literal_words() {
        assert!(template_match("громкость пятьдесят", "громкость {percent}").is_some());
        assert!(template_match("джарис открой мои файлы", "открой {app}").is_some());
        assert!(template_match("громкость на двадцать процентов", "громкость на {percent} процентов").is_some());
        // the longer literal wins
        assert!(template_match("открой папку загрузки", "открой папку {folder}") > template_match("открой папку загрузки", "открой {app}"));
        assert_eq!(template_match("громкость", "громкость {percent}"), None); // nothing in the slot
        assert_eq!(template_match("тише", "тише на {percent}"), None);
        assert_eq!(template_match("ну давай открой браузер", "открой {app}"), None); // two words before
        assert_eq!(template_match("открой браузер", "открой браузер"), None); // not a template
    }
}
