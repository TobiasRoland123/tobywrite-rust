use super::palette::Palette;
use gtk::{pango, prelude::*};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag};
use std::{cell::RefCell, ops::Range};

pub fn css(palette: &Palette, size: i32) -> String {
    format!(
        r#"
@define-color tw_background {background};
@define-color tw_foreground {foreground};
@define-color tw_muted {muted};
@define-color tw_border {border};
@define-color tw_selection {selection};
@define-color tw_selection_foreground {selection_foreground};
@define-color tw_accent {accent};
@define-color tw_accent_foreground {accent_foreground};
@define-color tw_code_background {code_background};
@define-color tw_quote {quote};
@define-color tw_focus {focus};

window {{ background-color: @tw_background; background-image: none; color: @tw_foreground; }}
popover, popover > contents, tooltip {{ background-color: @tw_background; background-image: none; color: @tw_foreground; border-color: @tw_border; }}
popover > contents {{ border: 1px solid @tw_border; }}
headerbar {{ background-color: @tw_background; background-image: none; color: @tw_foreground; border-bottom: 1px solid @tw_border; }}

button, entry, searchentry, spinbutton, dropdown, switch, checkbutton, radiobutton {{ color: @tw_foreground; }}
button {{ background-color: alpha(@tw_foreground, 0.035); background-image: none; border-color: alpha(@tw_foreground, 0.14); }}
button:hover {{ background-color: alpha(@tw_foreground, 0.09); background-image: none; border-color: alpha(@tw_foreground, 0.24); }}
button:active, button:checked {{ background-color: alpha(@tw_accent, 0.2); background-image: none; border-color: @tw_accent; color: @tw_foreground; }}
button:focus-visible, entry:focus-visible, searchentry:focus-visible, spinbutton:focus-visible, dropdown:focus-visible, switch:focus-visible, checkbutton:focus-visible, radiobutton:focus-visible {{ outline: 2px solid @tw_accent; outline-offset: 1px; }}
button:disabled, entry:disabled, searchentry:disabled, spinbutton:disabled, dropdown:disabled, switch:disabled, checkbutton:disabled, radiobutton:disabled {{ color: @tw_muted; }}
button.suggested-action, button.suggested-action label {{ background-color: @tw_accent; background-image: none; border-color: @tw_accent; color: @tw_accent_foreground; }}
button.suggested-action:hover, button.suggested-action:hover label {{ background-color: @tw_accent; background-image: none; color: @tw_accent_foreground; }}

entry, searchentry, spinbutton, dropdown {{ background-color: @tw_background; background-image: none; border-color: @tw_border; }}
entry:focus, searchentry:focus, spinbutton:focus, dropdown:focus {{ background-color: @tw_background; background-image: none; border-color: @tw_accent; box-shadow: none; outline: 2px solid @tw_accent; outline-offset: 1px; }}
entry placeholder, searchentry placeholder {{ color: @tw_muted; }}
entry selection, searchentry selection {{ background-color: @tw_selection; color: @tw_selection_foreground; }}
switch > slider {{ background-color: @tw_background; background-image: none; border-color: @tw_border; }}
switch:checked > slider {{ background-color: @tw_accent; background-image: none; border-color: @tw_accent; }}
list, listview {{ background-color: @tw_background; background-image: none; color: @tw_foreground; }}
list row, listview row {{ background-image: none; color: @tw_foreground; }}
list row:hover, listview row:hover {{ background-color: alpha(@tw_foreground, 0.06); background-image: none; }}
list row:selected, listview row:selected {{ background-color: @tw_selection; background-image: none; color: @tw_selection_foreground; }}
list row:selected label, listview row:selected label {{ color: @tw_selection_foreground; opacity: 1; }}
modelbutton:hover, modelbutton:focus {{ background-color: @tw_selection; background-image: none; color: @tw_selection_foreground; }}
modelbutton:hover label, modelbutton:focus label {{ color: @tw_selection_foreground; }}
separator {{ background-color: @tw_border; }}
.dim-label, label.accelerator {{ color: @tw_muted; opacity: 1; }}
.writer list row:selected label.library-path {{ color: @tw_selection_foreground; opacity: 1; }}

.writer headerbar {{ background-color: @tw_background; background-image: none; color: @tw_muted; border-bottom: 1px solid @tw_border; box-shadow: none; min-height: 46px; padding: 0 12px; }}
.writer headerbar button {{ background-color: transparent; background-image: none; box-shadow: none; border-color: transparent; color: @tw_muted; }}
.writer headerbar button:hover {{ background-color: alpha(@tw_foreground, 0.08); background-image: none; color: @tw_foreground; }}
.writer headerbar button:checked {{ background-color: alpha(@tw_accent, 0.2); background-image: none; border-color: @tw_accent; color: @tw_foreground; }}
.writer textview, .writer textview text {{ background-color: @tw_background; color: @tw_foreground; font-family: 'iA Writer Mono S', monospace; font-size: {size}px; caret-color: @tw_foreground; }}
.writer textview text selection {{ background-color: @tw_selection; color: @tw_selection_foreground; }}
.writer .status {{ color: @tw_muted; font-size: 11px; padding: 9px 22px; border-top: 1px solid @tw_border; }}
.writer .document-title {{ color: @tw_foreground; font-size: 12px; font-weight: 500; }}
.writer .document-path {{ font-size: 10px; color: @tw_muted; }}
.writer .find-bar {{ padding: 8px 18px; border-bottom: 1px solid @tw_border; }}
.writer scrolledwindow {{ border: 0; }}
.writer .library-path {{ font-size: 11px; color: @tw_muted; }}
.writer .help-text {{ color: @tw_muted; }}
"#,
        background = palette.background,
        foreground = palette.foreground,
        muted = palette.muted,
        border = palette.border,
        selection = palette.selection,
        selection_foreground = palette.selection_foreground,
        accent = palette.accent,
        accent_foreground = palette.accent_foreground,
        code_background = palette.code_background,
        quote = palette.quote,
        focus = palette.focus,
    )
}

pub struct Styler {
    tags: Vec<gtk::TextTag>,
    marker: gtk::TextTag,
    focus: gtk::TextTag,
    marker_ranges: RefCell<Vec<Range<i32>>>,
}

impl Styler {
    pub fn new(buffer: &gtk::TextBuffer) -> Self {
        let heading = gtk::TextTag::builder()
            .name("md-heading")
            .weight(700)
            .pixels_above_lines(9)
            .pixels_below_lines(4)
            .build();
        let strong = gtk::TextTag::builder()
            .name("md-strong")
            .weight(700)
            .build();
        let emphasis = gtk::TextTag::builder()
            .name("md-emphasis")
            .style(pango::Style::Italic)
            .build();
        let code = gtk::TextTag::builder().name("md-code").build();
        let quote = gtk::TextTag::builder()
            .name("md-quote")
            .style(pango::Style::Italic)
            .build();
        let link = gtk::TextTag::builder()
            .name("md-link")
            .underline(pango::Underline::Single)
            .build();
        let strike = gtk::TextTag::builder()
            .name("md-strike")
            .strikethrough(true)
            .build();
        let marker = gtk::TextTag::builder().name("md-marker").build();
        let focus = gtk::TextTag::builder().name("md-focus").build();
        let tags = vec![
            heading,
            strong,
            emphasis,
            code,
            quote,
            link,
            strike,
            marker.clone(),
            focus.clone(),
        ];
        for tag in &tags {
            buffer.tag_table().add(tag);
        }
        Self {
            tags,
            marker,
            focus,
            marker_ranges: RefCell::new(Vec::new()),
        }
    }

    /// Parsing produces character offsets once; moving the caret only updates dimming.
    pub fn parse(
        &self,
        buffer: &gtk::TextBuffer,
        source: &str,
        palette: &Palette,
        show_syntax: bool,
        focus_mode: bool,
    ) {
        let start = buffer.start_iter();
        let end = buffer.end_iter();
        for tag in &self.tags {
            buffer.remove_tag(tag, &start, &end);
        }
        let bytes: Vec<usize> = source
            .char_indices()
            .map(|(i, _)| i)
            .chain(std::iter::once(source.len()))
            .collect();
        let char_at = |byte| bytes.partition_point(|&b| b < byte) as i32;
        let mut markers = Vec::new();
        let mut text_ranges = Vec::<Range<usize>>::new();
        let mut code_ranges = Vec::<Range<usize>>::new();
        let mut constructs = Vec::<(Range<usize>, usize)>::new();
        // Large documents remain editable without parsing megabytes on each keystroke.
        if source.len() <= 1_000_000 {
            for (event, range) in Parser::new_ext(
                source,
                Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_TABLES,
            )
            .into_offset_iter()
            {
                match event {
                    Event::Start(tag) => {
                        if let Tag::CodeBlock(kind) = &tag {
                            code_ranges.push(range.clone());
                            if matches!(kind, CodeBlockKind::Fenced(_)) {
                                for marker in fence_markers(source, range.clone()) {
                                    markers.push(char_at(marker.start)..char_at(marker.end));
                                }
                            }
                        }
                        let index = match tag {
                            Tag::Heading { .. } => Some(0),
                            Tag::Strong => Some(1),
                            Tag::Emphasis => Some(2),
                            Tag::CodeBlock(_) => Some(3),
                            Tag::BlockQuote(_) => Some(4),
                            Tag::Link { .. } | Tag::Image { .. } => Some(5),
                            Tag::Strikethrough => Some(6),
                            _ => None,
                        };
                        if let Some(index) = index {
                            constructs.push((range, index));
                        }
                    }
                    Event::Text(_) => text_ranges.push(range),
                    Event::Code(_) => {
                        constructs.push((range.clone(), 3));
                        let raw = &source[range.clone()];
                        let count = raw.bytes().take_while(|&b| b == b'`').count();
                        if count > 0 && range.len() >= count * 2 {
                            markers.push(char_at(range.start)..char_at(range.start + count));
                            markers.push(char_at(range.end - count)..char_at(range.end));
                        }
                    }
                    Event::Rule => {
                        constructs.push((range.clone(), 6));
                        markers.push(char_at(range.start)..char_at(range.end));
                    }
                    _ => {}
                }
            }
            // Style the complete source span. Markers retain their source characters.
            for (range, index) in &constructs {
                buffer.apply_tag(
                    &self.tags[*index],
                    &buffer.iter_at_offset(char_at(range.start)),
                    &buffer.iter_at_offset(char_at(range.end)),
                );
                if matches!(index, 1 | 2 | 5 | 6) {
                    let first = text_ranges.partition_point(|inner| inner.start < range.start);
                    let end = text_ranges.partition_point(|inner| inner.end <= range.end);
                    if first < end {
                        let first = &text_ranges[first];
                        let last = &text_ranges[end - 1];
                        if first.start > range.start {
                            markers.push(char_at(range.start)..char_at(first.start));
                        }
                        if last.end < range.end {
                            markers.push(char_at(last.end)..char_at(range.end));
                        }
                    }
                }
            }
            let mut offset = 0;
            for line in source.split_inclusive('\n') {
                let code_index = code_ranges.partition_point(|range| range.end <= offset);
                if code_ranges
                    .get(code_index)
                    .is_some_and(|range| range.start < offset + line.len())
                {
                    offset += line.len();
                    continue;
                }
                let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
                let rest = &line[indent..];
                let prefix = if rest.starts_with('#') {
                    let n = rest.bytes().take_while(|&b| b == b'#').count();
                    if n <= 6 && rest.as_bytes().get(n) == Some(&b' ') {
                        n + 1
                    } else {
                        0
                    }
                } else if ["> ", "- ", "* ", "+ "]
                    .iter()
                    .any(|prefix| rest.starts_with(prefix))
                {
                    2
                } else if rest.starts_with("```") || rest.starts_with("~~~") {
                    rest.trim_end().len()
                } else {
                    let n = rest.bytes().take_while(u8::is_ascii_digit).count();
                    if n > 0 && (rest[n..].starts_with(". ") || rest[n..].starts_with(") ")) {
                        n + 2
                    } else {
                        0
                    }
                };
                if prefix > 0 {
                    markers.push(char_at(offset + indent)..char_at(offset + indent + prefix));
                }
                offset += line.len();
            }
        }
        self.tags[3].set_background(Some(&palette.code_background));
        self.tags[3].set_foreground(Some(&palette.foreground));
        self.tags[4].set_foreground(Some(&palette.quote));
        self.tags[5].set_foreground(Some(&palette.accent));
        self.marker.set_foreground(Some(&palette.focus));
        self.focus.set_foreground(Some(&palette.focus));
        *self.marker_ranges.borrow_mut() = markers;
        self.caret(buffer, show_syntax, focus_mode);
    }

    pub fn caret(&self, buffer: &gtk::TextBuffer, show_syntax: bool, focus_mode: bool) {
        let start = buffer.start_iter();
        let end = buffer.end_iter();
        buffer.remove_tag(&self.marker, &start, &end);
        buffer.remove_tag(&self.focus, &start, &end);
        let cursor = buffer.iter_at_offset(buffer.cursor_position());
        let mut paragraph_start = cursor;
        paragraph_start.set_line_offset(0);
        let mut paragraph_end = cursor;
        if !paragraph_end.ends_line() {
            paragraph_end.forward_to_line_end();
        }
        // Paragraph focus includes wrapped and successive nonblank source lines.
        // At a newline, forward_to_line_end() skips to the following line's end,
        // so leave blank lines in place when checking paragraph boundaries.
        while paragraph_start.line() > 0 {
            let mut previous = paragraph_start;
            previous.backward_line();
            previous.set_line_offset(0);
            let mut previous_end = previous;
            if !previous_end.ends_line() {
                previous_end.forward_to_line_end();
            }
            if buffer
                .text(&previous, &previous_end, true)
                .trim()
                .is_empty()
            {
                break;
            }
            paragraph_start = previous;
        }
        loop {
            let mut next = paragraph_end;
            if !next.forward_line() {
                break;
            }
            let mut next_end = next;
            if !next_end.ends_line() {
                next_end.forward_to_line_end();
            }
            if buffer.text(&next, &next_end, true).trim().is_empty() {
                break;
            }
            paragraph_end = next_end;
        }
        if !show_syntax {
            for range in self.marker_ranges.borrow().iter() {
                if range.end <= paragraph_start.offset() || range.start >= paragraph_end.offset() {
                    // Cursor notifications can precede the debounced reparse.
                    let end = range.end.min(buffer.char_count());
                    if range.start < end {
                        buffer.apply_tag(
                            &self.marker,
                            &buffer.iter_at_offset(range.start),
                            &buffer.iter_at_offset(end),
                        );
                    }
                }
            }
        }
        if focus_mode {
            buffer.apply_tag(&self.focus, &start, &paragraph_start);
            buffer.apply_tag(&self.focus, &paragraph_end, &end);
        }
    }
}

fn fence_markers(source: &str, range: Range<usize>) -> Vec<Range<usize>> {
    let raw = &source[range.clone()];
    let first_end = raw.find('\n').unwrap_or(raw.len());
    let opening = raw[..first_end].trim_start();
    let Some(character) = opening.chars().next() else {
        return Vec::new();
    };
    let count = opening.chars().take_while(|&c| c == character).count();
    let mut markers = Vec::with_capacity(2);
    markers.push(range.start..range.start + first_end);
    let trimmed = raw.trim_end_matches('\n');
    if let Some(last_start) = trimmed.rfind('\n') {
        let closing = trimmed[last_start + 1..].trim();
        if closing.len() >= count && closing.chars().all(|c| c == character) {
            markers.push(range.start + last_start + 1..range.start + trimmed.len());
        }
    }
    markers
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_fence_delimiters_fade_inside_code_blocks() {
        let source = "```md\n# literal\n- literal\n```\n";
        let ranges = fence_markers(source, 0..source.len());
        let markers: Vec<_> = ranges.iter().map(|range| &source[range.clone()]).collect();
        assert_eq!(markers, ["```md", "```"]);
        let unclosed = "~~~\n# literal\n";
        let markers = fence_markers(unclosed, 0..unclosed.len());
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0], 0..3);
    }
}
