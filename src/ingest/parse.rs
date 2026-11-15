//! Parsing a Telegram message into URLs, tags, Pavel's note and share text.
//!
//! Pure functions only: no I/O, so everything here is unit-tested directly.

use url::Url;

/// The pieces of one Telegram message.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedMessage {
    /// `http`/`https` URLs, deduplicated, in message order.
    pub urls: Vec<Url>,
    /// `#word` tags, lowercased, without `#`, deduplicated, in message order.
    pub tags: Vec<String>,
    /// Text of `>>` lines (marker and tags stripped), joined with `\n`.
    pub note: Option<String>,
    /// Everything else, minus URLs and tags; `None` if nothing is left.
    pub share_text: Option<String>,
}

/// Splits a message into URLs, tags, note and share text.
///
/// Rules:
/// - `#word` anywhere (including inside `>>` lines) is a tag.
/// - A line is a note line if it starts with `>>` once its URLs and tags are
///   removed, so `https://x.com >> check costs` puts "check costs" in the note.
/// - Words that parse as `http`/`https` URLs are URLs; other schemes are text.
/// - What remains outside note lines is share text.
pub fn parse_message(text: &str) -> ParsedMessage {
    let mut tags: Vec<String> = Default::default();
    let mut urls: Vec<Url> = Default::default();
    let mut note_lines: Vec<String> = Default::default();
    let mut share_lines: Vec<String> = Default::default();
    for line in text.lines() {
        let words = line.split_whitespace();
        let mut line_words: Vec<&str> = Default::default();
        for word in words {
            if let Some(tag) = word.strip_prefix("#") {
                let tag = tag.to_lowercase();
                if !tags.contains(&tag) && !tag.is_empty() {
                    tags.push(tag);
                }
            } else {
                if let Ok(url) = Url::parse(word)
                    && matches!(url.scheme(), "http" | "https")
                {
                    if !urls.contains(&url) {
                        urls.push(url);
                    }
                } else {
                    line_words.push(word);
                }
            }
        }
        let joined_line = line_words.join(" ");
        if let Some(note) = joined_line.strip_prefix(">>") {
            let note = note.trim();
            if !note.is_empty() {
                note_lines.push(note.to_string());
            }
        } else if !joined_line.is_empty() {
            share_lines.push(joined_line);
        }
    }
    let share_text = (!share_lines.is_empty()).then(|| share_lines.join("\n"));
    let note = (!note_lines.is_empty()).then(|| note_lines.join("\n"));

    ParsedMessage {
        urls,
        tags,
        note,
        share_text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_share_text_note_and_tags() {
        let p = parse_message(
            "Big HF incident - TechSite https://example.com/x\n\
             >> not urgent, but check #AI-News #hf-incident",
        );
        assert_eq!(p.urls[0].as_str(), "https://example.com/x");
        assert_eq!(p.tags, vec!["ai-news", "hf-incident"]);
        assert_eq!(p.note.as_deref(), Some("not urgent, but check"));
        assert_eq!(p.share_text.as_deref(), Some("Big HF incident - TechSite"));
    }

    #[test]
    fn bare_url_has_no_note_or_share_text() {
        let p = parse_message("https://example.com/a");
        assert_eq!(p.urls.len(), 1);
        assert!(p.tags.is_empty());
        assert_eq!(p.note, None);
        assert_eq!(p.share_text, None);
    }

    #[test]
    fn no_url_returns_empty_urls() {
        let p = parse_message("hello there #diy");
        assert!(p.urls.is_empty());
        assert_eq!(p.tags, vec!["diy"]);
        assert_eq!(p.share_text.as_deref(), Some("hello there"));
    }

    #[test]
    fn two_urls_both_extracted() {
        let p = parse_message("https://a.example/1 and https://b.example/2");
        let urls: Vec<&str> = p.urls.iter().map(Url::as_str).collect();
        assert_eq!(urls, vec!["https://a.example/1", "https://b.example/2"]);
        assert_eq!(p.share_text.as_deref(), Some("and"));
    }

    #[test]
    fn ignores_non_http_schemes() {
        let p = parse_message("ftp://files.example/x");
        assert!(p.urls.is_empty());
        assert_eq!(p.share_text.as_deref(), Some("ftp://files.example/x"));
    }

    #[test]
    fn duplicate_tags_and_urls_collapse() {
        let p = parse_message("https://a.example/1 #ai https://a.example/1 #AI");
        assert_eq!(p.urls.len(), 1);
        assert_eq!(p.tags, vec!["ai"]);
    }

    #[test]
    fn multiple_note_lines_join_with_newline() {
        let p = parse_message("https://a.example/1\n  >> first\n>>second");
        assert_eq!(p.note.as_deref(), Some("first\nsecond"));
    }

    #[test]
    fn tags_keep_message_order() {
        let p = parse_message("hey there #zebra #apple");
        assert_eq!(p.tags, vec!["zebra", "apple"]);
    }
    #[test]
    fn empty_tag_not_recorded() {
        let p = parse_message(" # ");
        assert!(p.tags.is_empty());
    }

    #[test]
    fn note_marker_after_url_and_tag_on_same_line() {
        // Pavel's rule: `>>` counts once URLs and tags are removed, so a link
        // and its note can share one line.
        let p = parse_message("https://x.com/a #ai >> check costs");
        assert_eq!(p.urls.len(), 1);
        assert_eq!(p.tags, vec!["ai"]);
        assert_eq!(p.note.as_deref(), Some("check costs"));
        assert_eq!(p.share_text, None);
    }
}
