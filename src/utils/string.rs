use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

pub fn normalize(s: &str) -> String {
    s.to_lowercase().replace(['’', '‘', '`'], "'").replace(['“', '”'], "\"")
}

const LETTERS: &[(char, &str)] = &[
    ('ø', "o"),
    ('æ', "ae"),
    ('œ', "oe"),
    ('ß', "ss"),
    ('þ', "th"),
    ('ð', "d"),
    ('đ', "d"),
    ('ł', "l"),
    ('ħ', "h"),
    ('ŧ', "t"),
    ('ı', "i"),
    ('ŋ', "n"),
    ('ə', "e"),
];

const SYMBOLS: &[(char, char)] = &[('!', 'i'), ('¡', 'i'), ('$', 's'), ('@', 'a')];

const fn is_latin(c: char) -> bool {
    c.is_ascii_alphabetic() || matches!(c as u32, 0x00C0..=0x024F | 0x1E00..=0x1EFF)
}

pub fn canonical(s: &str) -> String {
    let lowered = s.to_lowercase();
    let decomposed: Vec<char> = lowered.nfd().collect();

    let first = decomposed.iter().position(|c| c.is_alphanumeric());
    let last = decomposed.iter().rposition(|c| c.is_alphanumeric());

    let mut folded = String::with_capacity(lowered.len());
    let mut base_is_latin = false;

    for (at, &c) in decomposed.iter().enumerate() {
        if is_combining_mark(c) {
            if !base_is_latin {
                folded.push(c);
            }
            continue;
        }

        base_is_latin = is_latin(c);

        if let Some(&(_, to)) = LETTERS.iter().find(|(from, _)| *from == c) {
            folded.push_str(to);
            continue;
        }

        if let Some(&(_, to)) = SYMBOLS.iter().find(|(from, _)| *from == c) {
            let inner = at > 0
                && decomposed[at - 1].is_alphanumeric()
                && decomposed.get(at + 1).is_some_and(|next| next.is_alphanumeric());

            if inner {
                folded.push(to);
                continue;
            }
        }

        if c == '&' && first.is_some_and(|f| f < at) && last.is_some_and(|l| l > at) {
            folded.push_str("and");
            continue;
        }

        folded.push(c);
    }

    let composed: String = folded.nfc().collect();
    let key: String = composed.chars().filter(|c| c.is_alphanumeric()).collect();

    /* "!!!" and "+/-" are real bands whose whole name is punctuation. */
    if key.is_empty() { composed } else { key }
}

pub fn is_similar(a: &str, b: &str) -> bool {
    canonical(a) == canonical(b)
}

/*
 * Sanitize a str to so that we don't get sub folders for tracks with slashes in their title.
 */
pub fn path_segment(s: &str) -> String {
    let flattened = s
        .chars()
        .map(|c| if std::path::is_separator(c) || c.is_control() { '_' } else { c })
        .collect::<String>();

    match flattened.trim().trim_end_matches('.').trim_end() {
        "" => "_".to_string(),
        segment => segment.to_string(),
    }
}
