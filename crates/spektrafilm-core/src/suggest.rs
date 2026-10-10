pub(crate) const MAX_SUGGESTIONS: usize = 3;

/// Find nearby canonical identifiers without accepting or rewriting the input.
pub(crate) fn closest(
    input: &str,
    candidates: impl Iterator<Item = String>,
    limit: usize,
) -> Vec<String> {
    if limit == 0 {
        return Vec::new();
    }
    let input: Vec<char> = input.chars().collect();
    let maximum = if input.len() <= 4 { 1 } else { 2 };
    let mut previous = vec![0; input.len() + 1];
    let mut current = vec![0; input.len() + 1];
    let mut matches = Vec::new();
    for candidate in candidates {
        if input.len().abs_diff(candidate.chars().count()) > maximum {
            continue;
        }
        for (index, cell) in previous.iter_mut().enumerate() {
            *cell = index;
        }
        for (index, character) in candidate.chars().enumerate() {
            current[0] = index + 1;
            for (column, expected) in input.iter().enumerate() {
                current[column + 1] = (current[column] + 1)
                    .min(previous[column + 1] + 1)
                    .min(previous[column] + usize::from(character != *expected));
            }
            std::mem::swap(&mut previous, &mut current);
        }
        let distance = previous[input.len()];
        if distance <= maximum {
            let entry = (distance, candidate);
            let index = matches.binary_search(&entry).unwrap_or_else(|index| index);
            if index < limit {
                matches.insert(index, entry);
                matches.truncate(limit);
            }
        }
    }
    matches
        .into_iter()
        .map(|(_, candidate)| candidate)
        .collect()
}

pub(crate) fn error_suffix(candidates: &[String], guidance: &str) -> String {
    if candidates.is_empty() {
        guidance.to_owned()
    } else {
        format!("did you mean {}?", candidates.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suggestions(input: &str, candidates: &[&str]) -> Vec<String> {
        closest(
            input,
            candidates.iter().map(|value| (*value).to_owned()),
            MAX_SUGGESTIONS,
        )
    }

    #[test]
    fn short_inputs_allow_only_one_edit() {
        assert_eq!(
            suggestions("cat", &["cart", "cost", "bat"]),
            ["bat", "cart"]
        );
        assert_eq!(suggestions("abcd", &["abef", "abcde"]), ["abcde"]);
        assert_eq!(suggestions("abcde", &["abcef"]), ["abcef"]);
    }

    #[test]
    fn matches_sort_by_distance_then_spelling_and_stop_at_three() {
        assert_eq!(
            suggestions("grain", &["grains", "grainy", "brain", "grain", "train"]),
            ["grain", "brain", "grains"]
        );
        assert!(closest("grain", ["grain".to_owned()].into_iter(), 0).is_empty());
        assert!(suggestions("unrelated", &["grain", "camera"]).is_empty());
    }

    #[test]
    fn distance_counts_unicode_characters_and_empty_inputs() {
        assert_eq!(suggestions("é", &["è", ""]), ["", "è"]);
        assert_eq!(suggestions("", &["a", "ab"]), ["a"]);
    }

    #[test]
    fn suffix_uses_candidates_or_discovery_guidance() {
        assert_eq!(
            error_suffix(&suggestions("cat", &["bat", "cart"]), "discover"),
            "did you mean bat, cart?"
        );
        assert_eq!(error_suffix(&[], "discover"), "discover");
    }
}
