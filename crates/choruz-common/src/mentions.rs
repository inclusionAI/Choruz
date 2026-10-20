//! Case-insensitive mention token matching shared by routing and event delivery.

/// Match a complete mention, preferring longer known names at the same position.
/// Names may contain spaces; hyphens and underscores remain token characters.
pub fn contains_mention(content: &str, mention: &str, candidates: &[String]) -> bool {
    let content = content.to_lowercase();
    let mention = mention.to_lowercase();
    if mention.is_empty() {
        return false;
    }
    let candidates: Vec<String> = candidates.iter().map(|name| name.to_lowercase()).collect();
    content
        .match_indices(&format!("@{mention}"))
        .any(|(start, _)| {
            matches_at(&content, &mention, start)
                && !candidates
                    .iter()
                    .filter(|name| name.len() > mention.len())
                    .any(|name| matches_at(&content, name, start))
        })
}

fn matches_at(content: &str, mention: &str, start: usize) -> bool {
    let pattern = format!("@{mention}");
    if !content[start..].starts_with(&pattern) {
        return false;
    }
    let token = |ch: char| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_';
    content[..start]
        .chars()
        .next_back()
        .is_none_or(|ch| !token(ch))
        && content[start + pattern.len()..]
            .chars()
            .next()
            .is_none_or(|ch| !token(ch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_tokens_keep_boundaries_and_longest_names() {
        assert!(contains_mention("@backend dev, review", "Backend Dev", &[]));
        assert!(!contains_mention("email@backend", "backend", &[]));
        assert!(!contains_mention("@Backend-Devops", "backend", &[]));
        assert!(!contains_mention(
            "@BACKEND DEV review",
            "backend",
            &["Backend Dev".into()]
        ));
        assert!(contains_mention("@ÜNIT, review", "ünit", &[]));
    }
}
