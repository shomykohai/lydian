use strsim::jaro_winkler;

pub fn normalize(s: &str) -> String {
    s.to_lowercase().replace(['’', '‘', '`'], "'").replace(['“', '”'], "\"")
}

pub fn is_opt_similar(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (Some(s1), Some(s2)) => is_similar(s1, s2),
        (None, None) => true,
        _ => false,
    }
}

pub fn is_similar(a: &str, b: &str) -> bool {
    let a_norm = normalize(a);
    let b_norm = normalize(b);

    jaro_winkler(&a_norm, &b_norm) > 0.9
}
