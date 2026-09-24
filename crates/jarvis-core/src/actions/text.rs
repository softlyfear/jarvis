// Text helpers for matching spoken (Russian, Vosk output) names against
// program, game, process and file names (mostly Latin).

use seqdiff::ratio;

// lowercase, ё -> е, punctuation -> space, collapse spaces, number words -> digits
pub fn normalize(input: &str) -> String {
    let lower = input.to_lowercase().replace('ё', "е");
    let cleaned: String = lower
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();

    cleaned
        .split_whitespace()
        .map(|w| number_word(w).map(|n| n.to_string()).unwrap_or_else(|| w.to_string()))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn has_cyrillic(s: &str) -> bool {
    s.chars().any(|c| matches!(c, 'а'..='я' | 'А'..='Я' | 'ё' | 'Ё'))
}

// simple phonetic transliteration, tuned for brand names ("дискорд" -> "diskord")
pub fn translit(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 2);
    for c in input.chars() {
        let s = match c {
            'а' => "a", 'б' => "b", 'в' => "v", 'г' => "g", 'д' => "d",
            'е' => "e", 'ё' => "e", 'ж' => "zh", 'з' => "z", 'и' => "i",
            'й' => "y", 'к' => "k", 'л' => "l", 'м' => "m", 'н' => "n",
            'о' => "o", 'п' => "p", 'р' => "r", 'с' => "s", 'т' => "t",
            'у' => "u", 'ф' => "f", 'х' => "h", 'ц' => "ts", 'ч' => "ch",
            'ш' => "sh", 'щ' => "sch", 'ъ' => "", 'ы' => "y", 'ь' => "",
            'э' => "e", 'ю' => "yu", 'я' => "ya",
            _ => {
                out.push(c);
                continue;
            }
        };
        out.push_str(s);
    }
    out
}

// Latin spelling variants that commonly differ from a phonetic transliteration
fn latin_variants(s: &str) -> Vec<String> {
    let mut v = vec![s.to_string()];
    let swaps = [
        ("k", "c"), ("ks", "x"), ("i", "ee"), ("i", "y"), ("f", "ph"),
        ("v", "w"), ("u", "oo"), ("e", "a"), ("ay", "i"), ("ey", "a"), ("h", "ch"),
    ];
    for (from, to) in swaps {
        if s.contains(from) {
            v.push(s.replace(from, to));
        }
    }
    v
}

fn char_ratio(a: &str, b: &str) -> f64 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    ratio(&a, &b)
}

// 0..100, how well a spoken name matches a candidate name
pub fn similarity(spoken: &str, candidate: &str) -> f64 {
    let spoken = normalize(spoken);
    let candidate = normalize(candidate);
    if spoken.is_empty() || candidate.is_empty() {
        return 0.0;
    }
    if spoken == candidate {
        return 100.0;
    }

    let mut spoken_forms = vec![spoken.clone()];
    if has_cyrillic(&spoken) && !has_cyrillic(&candidate) {
        spoken_forms = latin_variants(&translit(&spoken));
    }
    let candidate_cmp = if has_cyrillic(&candidate) && !has_cyrillic(&spoken) {
        translit(&candidate)
    } else {
        candidate.clone()
    };

    let cand_words: Vec<&str> = candidate_cmp.split_whitespace().collect();
    let mut best: f64 = 0.0;

    for form in &spoken_forms {
        let compact_form = form.replace(' ', "");
        best = best.max(char_ratio(form, &candidate_cmp));
        best = best.max(char_ratio(&compact_form, &candidate_cmp.replace(' ', "")));

        // spoken name may be part of a longer title: "дота" -> "Dota 2", "хром" -> "Google Chrome"
        for i in 0..cand_words.len() {
            for j in i + 1..=cand_words.len() {
                let part = cand_words[i..j].join(" ");
                let r = char_ratio(form, &part).max(char_ratio(&compact_form, &part.replace(' ', "")));
                // a partial match is less certain than a full one, a leading part more than an inner one
                let weight = if i == 0 { 0.97 } else { 0.93 };
                best = best.max(r * weight);
            }
        }

        // abbreviations: "гта" -> "Grand Theft Auto"
        if cand_words.len() >= 2 && compact_form.len() >= 2 {
            let initials: String = cand_words
                .iter()
                .filter_map(|w| w.chars().next())
                .collect();
            if initials == compact_form || initials.starts_with(&compact_form) && compact_form.len() >= 3 {
                best = best.max(90.0);
            }
        }
    }

    best
}

// "пятьдесят" -> 50, "50" -> 50, "сто" -> 100 (0..100 range, enough for volume)
pub fn number_word(w: &str) -> Option<u32> {
    if let Ok(n) = w.parse::<u32>() {
        return Some(n);
    }
    let n = match w {
        "ноль" | "нуль" => 0,
        "один" | "одна" | "одну" | "первый" => 1,
        "два" | "две" | "второй" => 2,
        "три" | "третий" => 3,
        "четыре" => 4,
        "пять" => 5,
        "шесть" => 6,
        "семь" => 7,
        "восемь" => 8,
        "девять" => 9,
        "десять" => 10,
        "пятнадцать" => 15,
        "двадцать" => 20,
        "тридцать" => 30,
        "сорок" => 40,
        "пятьдесят" => 50,
        "шестьдесят" => 60,
        "семьдесят" => 70,
        "восемьдесят" => 80,
        "девяносто" => 90,
        "сто" => 100,
        _ => return None,
    };
    Some(n)
}

// first number in a phrase, "двадцать пять" -> 25
pub fn extract_number(text: &str) -> Option<u32> {
    let words: Vec<String> = text
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_string())
        .collect();

    for (i, w) in words.iter().enumerate() {
        if let Some(n) = number_word(w) {
            // compound: "двадцать пять"
            if n >= 20 && n % 10 == 0 && n < 100 {
                if let Some(next) = words.get(i + 1).and_then(|x| number_word(x)) {
                    if next < 10 {
                        return Some(n + next);
                    }
                }
            }
            return Some(n);
        }
    }
    None
}

// Whisper often hears a command verb in another form: "откроем компьютер", "закрою телеграм"
fn imperative(w: &str) -> Option<&'static str> {
    let v = match w {
        "откроем" | "открою" | "откроешь" | "открывай" | "откройка" => "открой",
        "закроем" | "закрою" | "закроешь" | "закрывай" | "закройка" => "закрой",
        "запустим" | "запущу" | "запустишь" | "запускай" => "запусти",
        "включим" | "включу" | "включишь" | "включай" => "включи",
        "выключим" | "выключу" | "выключишь" | "выключай" => "выключи",
        _ => return None,
    };
    Some(v)
}

// words said before a command that no command starts with: "так, закрой телеграм"
const FILLER_WORDS: &[&str] = &["так", "ну", "а", "эй", "слушай", "ладно", "короче", "окей", "ок", "пожалуйста"];

// the phrase as command templates expect it: lowercase, fillers before the command dropped,
// the verb in the imperative ("так закрою телеграм" -> "закрой телеграм")
pub fn tidy_command(phrase: &str) -> String {
    let lower = phrase.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    let start = words.iter().position(|w| !FILLER_WORDS.contains(&w.trim_matches(|c: char| !c.is_alphanumeric()))).unwrap_or(words.len());
    words[start..]
        .iter()
        .map(|w| imperative(w.trim_matches(|c: char| !c.is_alphanumeric())).unwrap_or(w))
        .collect::<Vec<_>>()
        .join(" ")
}

// leading command words dropped when extracting the object of a command
const LEADING_WORDS: &[&str] = &[
    "открой", "открыть", "откройте", "запусти", "запустить", "включи", "включить", "вруби",
    "закрой", "закрыть", "выключи", "выключить", "вырубить", "выруби", "заверши", "завершить",
    "убей", "поиграть", "поиграем", "играть", "давай", "пожалуйста", "мне", "игру", "игра",
    "в", "во", "приложение", "программу", "программа", "папку", "папка", "сайт", "найди",
    "удали", "удалить", "файл", "покажи", "перейди", "на", "хочу", "можешь", "сейчас",
    "open", "launch", "start", "run", "close", "kill", "play", "the", "game", "app",
];

// "запусти игру дота два" + templates ["запусти игру {game}"] -> "дота два"
pub fn extract_object(phrase: &str, templates: &[String]) -> String {
    let phrase_n = normalize(&tidy_command(phrase));

    for t in templates {
        let Some(open) = t.find('{') else { continue };
        let Some(close) = t.find('}') else { continue };
        if close < open {
            continue;
        }
        let prefix = normalize(&t[..open]);
        let suffix = normalize(&t[close + 1..]);
        if prefix.is_empty() {
            continue;
        }
        if let Some(rest) = phrase_n.strip_prefix(&prefix) {
            let rest = rest.trim();
            let rest = if !suffix.is_empty() {
                rest.strip_suffix(&suffix).unwrap_or(rest).trim()
            } else {
                rest
            };
            if !rest.is_empty() {
                return rest.to_string();
            }
        }
    }

    let words: Vec<&str> = phrase_n.split_whitespace().collect();
    let start = words
        .iter()
        .position(|w| !LEADING_WORDS.contains(w))
        .unwrap_or(words.len());
    words[start..].join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_handles_case_punctuation_and_numbers() {
        assert_eq!(normalize("  Дота, ДВА! "), "дота 2");
        assert_eq!(normalize("Ёлка"), "елка");
    }

    #[test]
    fn similarity_matches_transliterated_brands() {
        assert!(similarity("дискорд", "Discord") >= 80.0, "{}", similarity("дискорд", "Discord"));
        assert!(similarity("телеграм", "Telegram") >= 80.0);
        assert!(similarity("дота два", "Dota 2") >= 80.0);
        assert!(similarity("дота", "Dota 2") >= 80.0);
        assert!(similarity("майнкрафт", "Minecraft Launcher") >= 75.0, "{}", similarity("майнкрафт", "Minecraft Launcher"));
        assert!(similarity("гта", "Grand Theft Auto V") >= 85.0);
        assert!(similarity("дискорд", "Steam") < 60.0);
        assert!(similarity("ворд", "Microsoft Word") >= 85.0);
        assert!(similarity("хром", "Google Chrome") >= 80.0, "{}", similarity("хром", "Google Chrome"));
        assert!(similarity("дота", "Dead Cells") < 70.0);
        assert!(similarity("телеграм", "Steam") < 70.0);
        assert!(similarity("блокнот", "блокнот") == 100.0);
    }

    #[test]
    fn numbers_are_extracted() {
        assert_eq!(extract_number("громкость пятьдесят процентов"), Some(50));
        assert_eq!(extract_number("громкость двадцать пять"), Some(25));
        assert_eq!(extract_number("громкость 70"), Some(70));
        assert_eq!(extract_number("сделай громче"), None);
    }

    #[test]
    fn object_is_extracted_by_template_or_leading_words() {
        let t = vec!["запусти игру {game}".to_string(), "поиграем в {game}".to_string()];
        assert_eq!(extract_object("Запусти игру Дота два", &t), "дота 2");
        assert_eq!(extract_object("поиграем в майнкрафт", &t), "майнкрафт");
        assert_eq!(extract_object("давай открой пожалуйста телеграм", &[]), "телеграм");
        assert_eq!(extract_object("открой", &[]), "");
        // from a log: the filler before the command went into the program name
        let close = vec!["закрой {app}".to_string()];
        assert_eq!(extract_object("так  закрой телеграм", &close), "телеграм");
        assert_eq!(extract_object("откроем компьютер", &["открой {app}".to_string()]), "компьютер");
    }

    #[test]
    fn commands_are_tidied() {
        assert_eq!(tidy_command("Так, закрою Телеграм"), "закрой телеграм");
        assert_eq!(tidy_command("откроем компьютер"), "открой компьютер");
        assert_eq!(tidy_command("ну а запускай дота два"), "запусти дота два");
        // a filler inside the phrase stays: it may be part of a name
        assert_eq!(tidy_command("открой так"), "открой так");
        assert_eq!(tidy_command("  "), "");
    }
}
