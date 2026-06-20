use strsim::jaro_winkler;

pub fn normalize(s: &str) -> String {
    s.to_lowercase().replace(['’', '‘', '`'], "'").replace(['“', '”'], "\"")
}

pub fn is_similar(a: &str, b: &str) -> bool {
    let a_norm = normalize(a);
    let b_norm = normalize(b);

    jaro_winkler(&a_norm, &b_norm) > 0.9
}
