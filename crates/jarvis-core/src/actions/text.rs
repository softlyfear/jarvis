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

// Spoken cardinals through 999. Volume and brightness clamp the result; timers and
// alarms need the same words the assistant itself speaks ("двенадцать", "двести").
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
        "одиннадцать" => 11,
        "двенадцать" => 12,
        "тринадцать" => 13,
        "четырнадцать" => 14,
        "пятнадцать" => 15,
        "шестнадцать" => 16,
        "семнадцать" => 17,
        "восемнадцать" => 18,
        "девятнадцать" => 19,
        "двадцать" => 20,
        "тридцать" => 30,
        "сорок" => 40,
        "пятьдесят" => 50,
        "шестьдесят" => 60,
        "семьдесят" => 70,
        "восемьдесят" => 80,
        "девяносто" => 90,
        "сто" => 100,
        "двести" => 200,
        "триста" => 300,
        "четыреста" => 400,
        "пятьсот" => 500,
        "шестьсот" => 600,
        "семьсот" => 700,
        "восемьсот" => 800,
        "девятьсот" => 900,
        _ => return None,
    };
    Some(n)
}

// "двадцать пять" -> 25, "сто двадцать пять" -> 125. Clock pairs such as 7 and 30
// are not a compound: only a round ten absorbs a unit, and only a round hundred
// absorbs a remainder below 100.
pub fn absorb_compound(base: u32, next: u32) -> Option<u32> {
    // "двадцать ноль пять" is 20:05, not 25. Leave the zero so the clock parser can see it.
    if next == 0 {
        return None;
    }
    if (20..100).contains(&base) && base % 10 == 0 && next < 10 {
        return Some(base + next);
    }
    if (100..1000).contains(&base) && base % 100 == 0 && next < 100 {
        return Some(base + next);
    }
    // "сто двадцать пять" is absorbed as 120, then 5
    if (100..1000).contains(&base) && (20..100).contains(&(base % 100)) && base % 10 == 0 && next < 10 {
        return Some(base + next);
    }
    None
}

// first number in a phrase, "двадцать пять" -> 25, "сто двадцать" -> 120
pub fn extract_number(text: &str) -> Option<u32> {
    let words: Vec<String> = text
        .to_lowercase()
        .replace('ё', "е")
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_string())
        .collect();

    let mut i = 0;
    while i < words.len() {
        if let Some(mut n) = number_word(&words[i]) {
            i += 1;
            while i < words.len() {
                let Some(next) = number_word(&words[i]) else { break };
                let Some(joined) = absorb_compound(n, next) else { break };
                n = joined;
                i += 1;
            }
            return Some(n);
        }
        i += 1;
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

// How Vosk and Whisper spelled "Джарвис" in real logs ("Дарвис, покажи рабочий стол",
// "Чарли, закрой телеграм"); Whisper also splits it: "дар из", "дары с".
const ADDRESS_VARIANTS: &[&str] = &[
    "джарис", "жарвис", "сарвис", "гарвиц", "дарвис", "чарвис", "чарвиз", "чарвес", "чарлис",
    "чарли", "чарльз", "дарвина", "джары", "джанис", "jarvis",
    "джари", "джарви", "дчарлис", "чарлиз",
];
const ADDRESS_PAIRS: &[(&str, &str)] = &[("дар", "из"), ("дары", "с")];

fn is_address_word(word: &str) -> bool {
    let lang = crate::i18n::get_language();
    let wake = crate::config::get_wake_phrases(&lang).iter().chain(crate::config::get_wake_phrases("ru"));
    if wake.clone().any(|w| *w == word) || ADDRESS_VARIANTS.contains(&word) {
        return true;
    }
    // Other spellings end like the name ("джаррис", "жарвиз"); this keeps out "гарри",
    // "дари", "джарвиса" ("окно джарвиса") and ordinary words.
    let chars: Vec<char> = word.chars().collect();
    chars.len() >= 5
        && matches!(chars.last(), Some('с' | 'з' | 'ц'))
        && wake.map(|w| seqdiff::ratio(&w.chars().collect::<Vec<_>>(), &chars)).any(|r| r >= 80.0)
}

fn clean_word(word: &str) -> &str {
    word.trim_matches(|c: char| !c.is_alphanumeric())
}

// how many words at the start of `words` say "Джарвис"
fn address_len(words: &[&str]) -> usize {
    match words {
        [first, second, ..] if ADDRESS_PAIRS.contains(&(clean_word(first), clean_word(second))) => 2,
        // the name torn in two at its end: "джари с"
        [first, second, ..] if matches!(clean_word(second), "с" | "з" | "ис" | "из")
            && is_address_word(&format!("{}{}", clean_word(first), clean_word(second))) => 2,
        [first, ..] if is_address_word(clean_word(first)) => 1,
        _ => 0,
    }
}

// the lowercased phrase without its first `n` words, punctuation of the rest kept
fn skip_words(lower: &str, n: usize) -> &str {
    let mut rest = lower.trim();
    for _ in 0..n {
        let word = rest.split_whitespace().next().unwrap_or("");
        rest = rest[word.len()..].trim_start();
    }
    rest
}

// Remove an address only at the beginning, on word boundaries.
pub fn strip_address(phrase: &str) -> String {
    let lower = phrase.to_lowercase();
    let mut rest = lower.trim();
    loop {
        let words: Vec<&str> = rest.split_whitespace().collect();
        let n = address_len(&words);
        if n == 0 { return rest.to_string(); }
        rest = skip_words(rest, n);
    }
}

// The command in the phrase that woke Jarvis up: words before the address were said to
// someone else ("ничего не произошло, Джарвис, закрой телеграм"). None without an address.
pub fn after_address(phrase: &str) -> Option<String> {
    let lower = phrase.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    let start = (0..words.len()).find(|&i| address_len(&words[i..]) > 0)?;
    Some(strip_address(skip_words(&lower, start)))
}

// words said before a command that no command starts with: "так, закрой телеграм"
const FILLER_WORDS: &[&str] = &["так", "ну", "а", "эй", "слушай", "ладно", "короче", "окей", "ок", "пожалуйста"];

// the phrase as command templates expect it: lowercase, fillers before the command dropped,
// the verb in the imperative ("так закрою телеграм" -> "закрой телеграм")
pub fn tidy_command(phrase: &str) -> String {
    let lower = strip_address(phrase);
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
        assert_eq!(extract_number("громкость двенадцать"), Some(12));
        assert_eq!(extract_number("громкость одиннадцать"), Some(11));
        assert_eq!(extract_number("яркость девятнадцать"), Some(19));
        assert_eq!(extract_number("сто двадцать пять"), Some(125));
        assert_eq!(extract_number("двести пятнадцать"), Some(215));
        // a clock pair is not a compound number
        assert_eq!(extract_number("7 30"), Some(7));
        assert_eq!(number_word("шестнадцать"), Some(16));
        assert_eq!(number_word("девятьсот"), Some(900));
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
    fn addresses_are_removed_only_at_the_start() {
        assert_eq!(strip_address("Жарвис закрой steam"), "закрой steam");
        assert_eq!(strip_address("Джарис отключись"), "отключись");
        assert_eq!(strip_address("джарвис напечатай джарвис тест"), "напечатай джарвис тест");
        assert_eq!(strip_address("открой окно джарвиса"), "открой окно джарвиса");
        assert_eq!(strip_address("покажи рабочий стол"), "покажи рабочий стол");
        assert_eq!(extract_object("жарвис закрой steam", &["закрой {app}".into()]), "steam");
    }

    #[test]
    fn misheard_addresses_from_logs_are_removed() {
        for (heard, command) in [
            ("дарвис покажи рабочий стол", "покажи рабочий стол"),
            ("чарвис закрой steam", "закрой steam"),
            ("чарли закрой телеграмм", "закрой телеграмм"),
            ("молодец чарвиз", "молодец чарвиз"),
            ("дар из как твои дела", "как твои дела"),
            ("Чарлис, привет!", "привет!"),
            ("Чарлиз, привет!", "привет!"),
            ("джаррис открой блокнот", "открой блокнот"),
        ] {
            assert_eq!(strip_address(heard), command, "{}", heard);
        }
        for phrase in ["гарри поттер", "дари мне цветы", "парис хилтон", "давай поиграем", "джаз"] {
            assert_eq!(strip_address(phrase), phrase);
        }
    }

    #[test]
    fn words_before_the_address_are_not_the_command() {
        assert_eq!(after_address("ничего не произошло джарвис закрой телеграм").as_deref(), Some("закрой телеграм"));
        assert_eq!(after_address("слушай чарвис, открой блокнот").as_deref(), Some("открой блокнот"));
        assert_eq!(after_address("джарвис напечатай джарвис тест").as_deref(), Some("напечатай джарвис тест"));
        assert_eq!(after_address("молодец джарвис").as_deref(), Some(""));
        assert_eq!(after_address("джари с каким способом ты можешь").as_deref(), Some("каким способом ты можешь"));
        assert_eq!(after_address("джанис").as_deref(), Some(""));
        assert_eq!(after_address("вы с ним знакомы"), None);
        assert_eq!(after_address("открой окно джарвиса"), None);
        assert_eq!(after_address("включи гарри поттера"), None);
    }

    #[test]
    fn commands_are_tidied() {
        assert_eq!(after_address("джари закрой терминал").as_deref(), Some("закрой терминал"));
        assert_eq!(after_address("джарви введи команду").as_deref(), Some("введи команду"));
        assert_eq!(after_address("дчарлис какая была команда").as_deref(), Some("какая была команда"));
        assert_eq!(tidy_command("Так, закрою Телеграм"), "закрой телеграм");
        assert_eq!(tidy_command("откроем компьютер"), "открой компьютер");
        assert_eq!(tidy_command("ну а запускай дота два"), "запусти дота два");
        // a filler inside the phrase stays: it may be part of a name
        assert_eq!(tidy_command("открой так"), "открой так");
        assert_eq!(tidy_command("  "), "");
    }
}
