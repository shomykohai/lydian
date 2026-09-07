use crate::models::Searchable;
use crate::utils::string::normalize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
enum Closeness {
    Exact = 0,
    Prefix = 1,
    WordPrefix = 2,
    Substring = 3,
    Miss = 4,
}

impl Closeness {
    fn of(text: &str, term: &str) -> Self {
        if text == term {
            Self::Exact
        } else if text.starts_with(term) {
            Self::Prefix
        } else if text.split_whitespace().any(|word| word.starts_with(term)) {
            Self::WordPrefix
        } else if text.contains(term) {
            Self::Substring
        } else {
            Self::Miss
        }
    }

    fn nearest(label: &str, artist: Option<&str>, term: &str) -> Self {
        let closeness = Self::of(label, term);

        artist.map_or(closeness, |artist| closeness.min(Self::of(artist, term)))
    }

    const fn penalty(self) -> u32 {
        self as u32
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Relevance {
    phrase: Closeness,
    missed: u32,
    penalty: u32,
    sort_key: String,
}

impl Relevance {
    fn of(item: &impl Searchable, query: &str, tokens: &[&str]) -> Self {
        let label = normalize(item.label());
        let credited = item.artist().map(normalize);
        let credited = credited.as_deref();

        let mut missed = 0;
        let mut penalty = 0;

        for term in tokens {
            let closeness = Closeness::nearest(&label, credited, term);

            missed += u32::from(closeness == Closeness::Miss);
            penalty += closeness.penalty();
        }

        Self {
            phrase: Closeness::nearest(&label, credited, query),
            missed,
            penalty,
            sort_key: normalize(item.sort_key()),
        }
    }
}

pub fn order_by_relevance<T: Searchable>(items: &mut [T], query: &str) {
    let query = normalize(query.trim());
    let tokens: Vec<&str> = query.split_whitespace().collect();

    if tokens.is_empty() {
        items.sort_by_cached_key(|item| normalize(item.sort_key()));
        return;
    }

    items.sort_by_cached_key(|item| Relevance::of(item, &query, &tokens));
}
