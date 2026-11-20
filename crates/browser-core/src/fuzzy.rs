//! Small subsequence matcher for the omnibox and command palette. Higher is
//! better; `None` means not every query character was found in order.

const MATCH: i32 = 16;
const CONSECUTIVE: i32 = 24;
const WORD_START: i32 = 20;
const FIRST_CHAR: i32 = 12;
const GAP: i32 = 1;

pub fn score(query: &str, candidate: &str) -> Option<i32> {
    let query: Vec<char> = query
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|c| !c.is_whitespace())
        .collect();
    if query.is_empty() {
        return Some(0);
    }
    let chars: Vec<char> = candidate.chars().collect();
    let mut total = 0;
    let mut next = 0;
    let mut previous: Option<usize> = None;
    for (i, c) in chars.iter().enumerate() {
        if next == query.len() {
            break;
        }
        if !c.to_lowercase().eq(std::iter::once(query[next])) {
            continue;
        }
        total += MATCH + bonus(&chars, i, previous);
        previous = Some(i);
        next += 1;
    }
    (next == query.len()).then_some(total)
}

fn bonus(chars: &[char], i: usize, previous: Option<usize>) -> i32 {
    let mut bonus = 0;
    match previous {
        Some(p) if p + 1 == i => bonus += CONSECUTIVE,
        Some(p) => bonus -= GAP * (i - p - 1).min(16) as i32,
        None => bonus -= GAP * i.min(16) as i32,
    }
    if i == 0 {
        bonus += FIRST_CHAR;
    }
    if i == 0 || !chars[i - 1].is_alphanumeric() {
        bonus += WORD_START;
    }
    bonus
}

/// Best score across several fields, e.g. title and URL.
pub fn best(query: &str, fields: &[&str]) -> Option<i32> {
    fields.iter().filter_map(|f| score(query, f)).max()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_order() {
        assert!(score("gh", "GitHub").is_some());
        assert!(score("hg", "GitHub").is_none());
        assert_eq!(score("", "anything"), Some(0));
    }

    #[test]
    fn prefers_prefix_and_word_starts() {
        let prefix = score("rust", "rust-lang.org");
        let buried = score("rust", "trustpilot.com");
        assert!(prefix > buried);
        let words = score("nt", "New Tab");
        let middle = score("nt", "Moment");
        assert!(words > middle);
    }

    #[test]
    fn best_takes_strongest_field() {
        assert_eq!(
            best("wiki", &["Umbrella", "en.wikipedia.org"]),
            score("wiki", "en.wikipedia.org")
        );
    }
}
