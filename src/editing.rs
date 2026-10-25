use std::{ops::Range, sync::OnceLock};

use regex::Regex;
use unicode_segmentation::UnicodeSegmentation;
use url::Url;

/// Offsets count Unicode scalar values, matching GTK TextBuffer offsets.
/// `selection` describes the selection after applying the replacement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Edit {
    pub range: Range<usize>,
    pub replacement: String,
    pub selection: Range<usize>,
}

pub fn wrap(text: &str, selection: Range<usize>, marker: &str) -> Edit {
    let range = clamped_range(text, selection);
    let selected = scalar_slice(text, range.clone());
    let marker_len = marker.chars().count();
    let selected_len = range.len();
    if marker.is_empty() {
        return Edit {
            range: range.clone(),
            replacement: selected.into(),
            selection: range,
        };
    }
    if selected_len >= marker_len * 2 && selected.starts_with(marker) && selected.ends_with(marker)
    {
        let content = &selected[marker.len()..selected.len() - marker.len()];
        return Edit {
            range: range.clone(),
            replacement: content.into(),
            selection: range.start..range.start + selected_len - 2 * marker_len,
        };
    }
    let text_len = text.chars().count();
    if range.start >= marker_len
        && marker_len <= text_len.saturating_sub(range.end)
        && scalar_slice(text, range.start - marker_len..range.start) == marker
        && scalar_slice(text, range.end..range.end + marker_len) == marker
    {
        let start = range.start - marker_len;
        return Edit {
            range: start..range.end + marker_len,
            replacement: selected.into(),
            selection: start..start + selected_len,
        };
    }
    Edit {
        range: range.clone(),
        replacement: format!("{marker}{selected}{marker}"),
        selection: range.start + marker_len..range.start + marker_len + selected_len,
    }
}

pub fn insert_link(text: &str, selection: Range<usize>, clipboard_url: Option<&str>) -> Edit {
    let range = clamped_range(text, selection);
    let selected = scalar_slice(text, range.clone());
    let label = if selected.is_empty() {
        "link text"
    } else {
        selected
    };
    let label = label
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]");
    let url = clipboard_url.and_then(accepted_url);
    let destination = escape_destination(url.unwrap_or("https://"));
    let replacement = format!("[{label}]({destination})");
    let selection = if range.is_empty() {
        range.start + 1..range.start + 1 + label.chars().count()
    } else if url.is_none() {
        let start = range.start + label.chars().count() + 3;
        start..start + destination.chars().count()
    } else {
        let end = range.start + replacement.chars().count();
        end..end
    };
    Edit {
        range,
        replacement,
        selection,
    }
}

/// Accept only an explicit HTTP(S) URL with an authority and a host.
/// Checking the authority before parsing avoids accepting repairs such as
/// `https:///missing-host` or a backslash disguised as a slash.
pub fn accepted_url(text: &str) -> Option<&str> {
    let candidate = text.trim();
    if candidate.chars().any(char::is_control) || candidate.contains('\\') {
        return None;
    }
    let (scheme, rest) = candidate.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.chars().any(char::is_whitespace) {
        return None;
    }
    let parsed = Url::parse(candidate).ok()?;
    parsed.host_str().filter(|host| !host.is_empty())?;
    Some(candidate)
}

fn escape_destination(destination: &str) -> String {
    let mut escaped = String::new();
    for c in destination.chars() {
        if matches!(c, '\\' | '(' | ')') {
            escaped.push('\\');
            escaped.push(c);
        } else if c.is_whitespace() || matches!(c, '<' | '>' | '"') {
            let mut bytes = [0; 4];
            for byte in c.encode_utf8(&mut bytes).as_bytes() {
                use std::fmt::Write;
                write!(escaped, "%{byte:02X}").expect("writing to a String cannot fail");
            }
        } else {
            escaped.push(c);
        }
    }
    escaped
}

pub fn continue_list(text: &str, selection: Range<usize>) -> Option<Edit> {
    let range = clamped_range(text, selection);
    if !range.is_empty() {
        return None;
    }
    let caret = scalar_to_byte(text, range.start);
    let line_start = text[..caret]
        .rfind(['\r', '\n'])
        .map_or(0, |index| index + 1);
    if inside_fence(&text[..line_start]) {
        return None;
    }
    let line_end = text[caret..]
        .find(['\r', '\n'])
        .map_or(text.len(), |index| caret + index);
    let prefix = &text[line_start..caret];
    let suffix = &text[caret..line_end];
    static UNORDERED: OnceLock<Regex> = OnceLock::new();
    static ORDERED: OnceLock<Regex> = OnceLock::new();
    let unordered = UNORDERED.get_or_init(|| Regex::new(r"^([ \t]*)([-+*])([ \t]+)(.*)$").unwrap());
    let ordered =
        ORDERED.get_or_init(|| Regex::new(r"^([ \t]*)([0-9]+)([.)])([ \t]+)(.*)$").unwrap());
    let (indentation, marker, content) = if let Some(captures) = unordered.captures(prefix) {
        (
            captures[1].to_owned(),
            captures[2].to_owned(),
            captures[4].to_owned(),
        )
    } else {
        let captures = ordered.captures(prefix)?;
        (
            captures[1].to_owned(),
            format!("{}{}", increment_decimal(&captures[2]), &captures[3]),
            captures[5].to_owned(),
        )
    };
    static TASK: OnceLock<Regex> = OnceLock::new();
    let task_regex = TASK.get_or_init(|| Regex::new(r"^\[([ xX])\](?:[ \t]+|$)(.*)$").unwrap());
    let full_content = format!("{content}{suffix}");
    let task = task_regex.captures(&full_content);
    let empty = task.as_ref().map_or_else(
        || full_content.trim().is_empty(),
        |captures| captures[2].trim().is_empty(),
    );
    let newline = if text.contains("\r\n") {
        "\r\n"
    } else if text.contains('\r') {
        "\r"
    } else {
        "\n"
    };
    if empty {
        let start = text[..line_start].chars().count();
        let end = start + newline.chars().count();
        Some(Edit {
            range: start..range.start,
            replacement: newline.into(),
            selection: end..end,
        })
    } else {
        let task_marker = if task.is_some() { "[ ] " } else { "" };
        let replacement = format!("{newline}{indentation}{marker} {task_marker}");
        let end = range.start + replacement.chars().count();
        Some(Edit {
            range,
            replacement,
            selection: end..end,
        })
    }
}

pub fn word_count(text: &str) -> usize {
    static WORDS: OnceLock<Regex> = OnceLock::new();
    WORDS
        .get_or_init(|| {
            Regex::new(r"[\p{L}\p{N}][\p{L}\p{N}\p{M}]*(?:['’\-][\p{L}\p{N}][\p{L}\p{N}\p{M}]*)*")
                .unwrap()
        })
        .find_iter(text)
        .count()
}

fn scalar_to_byte(text: &str, offset: usize) -> usize {
    text.char_indices()
        .nth(offset)
        .map_or(text.len(), |(index, _)| index)
}

fn scalar_slice(text: &str, range: Range<usize>) -> &str {
    &text[scalar_to_byte(text, range.start)..scalar_to_byte(text, range.end)]
}

fn clamped_range(text: &str, requested: Range<usize>) -> Range<usize> {
    let len = text.chars().count();
    let start = requested.start.min(len);
    let end = requested.end.min(len).max(start);
    let mut safe_start = start;
    let mut safe_end = end;
    let mut position = 0;
    for grapheme in text.graphemes(true) {
        let next = position + grapheme.chars().count();
        if start > position && start < next {
            safe_start = position;
        }
        if end > position && end < next {
            safe_end = next;
        }
        position = next;
    }
    if start == end {
        safe_end = safe_start;
    }
    safe_start..safe_end
}

fn inside_fence(prior_lines: &str) -> bool {
    let mut active: Option<(char, usize)> = None;
    for line in prior_lines.split(['\r', '\n']) {
        let spaces = line.bytes().take_while(|&b| b == b' ').count();
        if spaces > 3 {
            continue;
        }
        let rest = &line[spaces..];
        let Some(character @ ('`' | '~')) = rest.chars().next() else {
            continue;
        };
        let length = rest.chars().take_while(|&c| c == character).count();
        if length < 3 {
            continue;
        }
        let suffix = &rest[length..];
        match active {
            Some((open, required))
                if open == character && length >= required && suffix.trim().is_empty() =>
            {
                active = None
            }
            None if character != '`' || !suffix.contains('`') => active = Some((character, length)),
            _ => {}
        }
    }
    active.is_some()
}

fn increment_decimal(value: &str) -> String {
    let canonical = value.trim_start_matches('0');
    let mut digits = if canonical.is_empty() {
        vec![b'0']
    } else {
        canonical.as_bytes().to_vec()
    };
    for digit in digits.iter_mut().rev() {
        if *digit != b'9' {
            *digit += 1;
            return String::from_utf8(digits).unwrap();
        }
        *digit = b'0';
    }
    digits.insert(0, b'1');
    String::from_utf8(digits).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapping_toggles_selected_or_adjacent_markers_and_empty_pairs() {
        assert_eq!(
            wrap("hello", 0..5, "**"),
            Edit {
                range: 0..5,
                replacement: "**hello**".into(),
                selection: 2..7
            }
        );
        for selection in [0..9, 2..7] {
            assert_eq!(
                wrap("**hello**", selection, "**"),
                Edit {
                    range: 0..9,
                    replacement: "hello".into(),
                    selection: 0..5
                }
            );
        }
        assert_eq!(
            wrap("****", 2..2, "**"),
            Edit {
                range: 0..4,
                replacement: "".into(),
                selection: 0..0
            }
        );
        assert_eq!(wrap("**", 0..2, "**").replacement, "******");
        assert_eq!(wrap("abc", 1..1, "`").selection, 2..2);
        assert_eq!(
            wrap("abc", 1..2, ""),
            Edit {
                range: 1..2,
                replacement: "b".into(),
                selection: 1..2
            }
        );
    }

    #[test]
    fn scalar_offsets_and_graphemes_are_safe_for_emoji_and_accents() {
        assert_eq!(wrap("😀 café", 0..1, "*").selection, 1..2);
        let emoji = "a👩🏽‍💻b";
        let edit = wrap(emoji, 2..3, "**");
        assert_eq!(edit.range, 1..5);
        assert_eq!(edit.replacement, "**👩🏽‍💻**");
        assert_eq!(edit.selection, 3..7);
        assert_eq!(wrap(emoji, 3..3, "*").range, 1..1);
        assert_eq!(wrap("e\u{301}lan", 1..2, "*").replacement, "*e\u{301}*");
        assert_eq!(wrap("é", 0..usize::MAX, "*").range, 0..1);
        assert_eq!(wrap("é", usize::MAX..usize::MAX, "*").range, 1..1);
        assert_eq!(wrap("abc", Range { start: 2, end: 1 }, "*").range, 2..2);
    }

    #[test]
    fn links_escape_markdown_and_choose_the_correct_selection() {
        let label = "[read] \\me";
        let edit = insert_link(
            label,
            0..label.chars().count(),
            Some("https://example.com/a (b)"),
        );
        assert_eq!(
            edit.replacement,
            "[\\[read\\] \\\\me](https://example.com/a%20\\(b\\))"
        );
        assert!(edit.selection.is_empty());
        assert_eq!(
            insert_link("", 0..0, Some("ftp://bad")),
            Edit {
                range: 0..0,
                replacement: "[link text](https://)".into(),
                selection: 1..10
            }
        );
        assert_eq!(insert_link("Hi title", 3..8, None).selection, 11..19);
        assert_eq!(
            insert_link("😀", 0..1, Some("https://example.com")).selection,
            24..24
        );
        assert_eq!(
            insert_link("title", 0..5, Some("https://example.com/<tag>\"")).replacement,
            "[title](https://example.com/%3Ctag%3E%22)"
        );
    }

    #[test]
    fn urls_require_an_explicit_safe_http_authority() {
        for candidate in [
            "https:///missing-host",
            "https://",
            "https:example.com",
            "javascript:alert(1)",
            "https://example.com/a\nb",
            "https://example.com/a\tb",
            "https://example.com\\evil",
            "https://exa mple.com",
        ] {
            assert_eq!(accepted_url(candidate), None, "accepted {candidate}");
        }
        assert_eq!(
            accepted_url("  HTTPS://example.com/a (b)  "),
            Some("HTTPS://example.com/a (b)")
        );
    }

    fn at_end(text: &str) -> Option<Edit> {
        let end = text.chars().count();
        continue_list(text, end..end)
    }

    #[test]
    fn lists_continue_indentation_tasks_and_unbounded_numbers() {
        assert_eq!(at_end("  9. first").unwrap().replacement, "\n  10. ");
        for marker in ["- [x] done", "+ [X] done", "* [ ] done"] {
            assert!(at_end(marker).unwrap().replacement.ends_with(" [ ] "));
        }
        assert_eq!(
            at_end("\t9999999999999999999999999999999999) item")
                .unwrap()
                .replacement,
            "\n\t10000000000000000000000000000000000) "
        );
        assert_eq!(at_end("0009. item").unwrap().replacement, "\n10. ");
        assert_eq!(at_end("- 😀").unwrap().selection, 6..6);
        assert_eq!(at_end("- first\r\n- second").unwrap().replacement, "\r\n- ");
        assert_eq!(at_end("- first\r- second").unwrap().replacement, "\r- ");
    }

    #[test]
    fn empty_items_exit_but_midline_items_continue() {
        assert_eq!(
            at_end("- ").unwrap(),
            Edit {
                range: 0..2,
                replacement: "\n".into(),
                selection: 1..1
            }
        );
        assert_eq!(at_end("  2. [x] ").unwrap().replacement, "\n");
        assert_eq!(continue_list("- text", 2..2).unwrap().replacement, "\n- ");
        assert_eq!(
            continue_list("- [x] text", 2..2).unwrap().replacement,
            "\n- [ ] "
        );
        assert!(continue_list("- selected", 0..2).is_none());
        assert!(at_end("ordinary paragraph").is_none());
    }

    #[test]
    fn fences_obey_character_length_indentation_and_closing_rules() {
        for text in [
            "```\n- code",
            "~~~rust\n- code",
            "   ````\n```\n- code",
            "```\n~~~\n- code",
            "```\n``` still code\n- code",
        ] {
            assert!(at_end(text).is_none(), "continued in {text:?}");
        }
        for text in [
            "    ```\n- item",
            "```\ncode\n```\n- item",
            "~~~\ncode\n~~~~\n- item",
            "```not`a`fence\n- item",
        ] {
            assert!(at_end(text).is_some(), "did not continue in {text:?}");
        }
    }

    #[test]
    fn words_include_unicode_and_internal_apostrophes_or_hyphens() {
        assert_eq!(word_count("one two-three don't 42"), 4);
        assert_eq!(word_count("你好 世界 ** * _ [] ()"), 2);
        assert_eq!(word_count("😀 --- ..."), 0);
        assert_eq!(word_count("e\u{301}lan l’esprit ひらがな"), 3);
    }
}
