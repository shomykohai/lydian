// src/utils/metadata.rs

use std::sync::OnceLock;

use regex::Regex;

use crate::utils::string::{is_similar, normalize};

struct MetadataFilters {
    /// Matches anything in brackets or parentheses
    /// I.e. "[Explicit]", "(Live version)", "(2020 Remastered)"
    noise_brackets: Regex,
    /// Matches terms like "Remastered"
    noise_terms: Regex,
    /// Just in case
    whitespace: Regex,
}

fn filters() -> &'static MetadataFilters {
    static INSTANCE: OnceLock<MetadataFilters> = OnceLock::new();
    INSTANCE.get_or_init(|| MetadataFilters {
        noise_brackets: Regex::new(
            r"(?i)[\[\(](?:\d{4}\s+)?(?:remaster(?:ed)?|explicit|live|mono|stereo|bonus(?:\s+track)?|edition|version|single|ep)[\]\)]"
        ).unwrap(),
        noise_terms: Regex::new(r"(?i)\b(?:remaster(?:ed)?|explicit|bonus track)\b").unwrap(),
        whitespace: Regex::new(r"\s+").unwrap(),
    })
}

fn clean_title(title: &str) -> String {
    let f = filters();
    let cleaned = f.noise_brackets.replace_all(title, "");
    let cleaned = f.noise_terms.replace_all(&cleaned, "");
    let normalized = normalize(&cleaned);
    f.whitespace.replace_all(&normalized, " ").trim().to_string()
}

pub fn compare_titles(a: &str, b: &str) -> bool {
    let clean_a = clean_title(a);
    let clean_b = clean_title(b);

    is_similar(&clean_a, &clean_b)
}

pub fn compare_artists(a: Option<&str>, b: Option<&str>) -> bool {
    let (Some(artist_a), Some(artist_b)) = (a, b) else {
        return a.is_none() && b.is_none();
    };

    let norm_a = normalize(artist_a);
    let norm_b = normalize(artist_b);

    if is_similar(&norm_a, &norm_b) {
        return true;
    }

    if norm_a.contains(&norm_b) || norm_b.contains(&norm_a) {
        return true;
    }

    // Sometimes some artists are listed together, because some
    // services are so nice to combine them into one string.
    let splitters = [" & ", " feat. ", " ft. ", ", ", " and ", " x "];
    let mut tokens_a = vec![norm_a.as_str()];
    let mut tokens_b = vec![norm_b.as_str()];

    for sep in splitters {
        tokens_a = tokens_a.into_iter().flat_map(|s| s.split(sep)).collect();
        tokens_b = tokens_b.into_iter().flat_map(|s| s.split(sep)).collect();
    }

    tokens_a.iter().any(|t_a| tokens_b.iter().any(|t_b| t_a.trim() == t_b.trim()))
}
