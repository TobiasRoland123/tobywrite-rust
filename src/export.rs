//! Offline, script-free HTML export. Source HTML is shown literally and images
//! become their alt text, so opening an export never fetches document resources.
use anyhow::Result;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::path::Path;

pub fn options() -> Options {
    Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS
}

pub fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn render_html(markdown: &str, title: &str) -> String {
    let mut events = Vec::new();
    for event in Parser::new_ext(markdown, options()) {
        match event {
            Event::Html(text) | Event::InlineHtml(text) => events.push(Event::Text(text)),
            Event::Start(Tag::Image { .. }) | Event::End(TagEnd::Image) => {}
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                let destination = if crate::editing::accepted_url(&dest_url).is_some() {
                    dest_url
                } else {
                    "#".into()
                };
                events.push(Event::Start(Tag::Link {
                    link_type,
                    dest_url: destination,
                    title,
                    id,
                }));
            }
            other => events.push(other),
        }
    }
    let mut body = String::new();
    pulldown_cmark::html::push_html(&mut body, events.into_iter());
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'\">\n<title>{}</title>\n<style>body{{max-width:46rem;margin:4rem auto;padding:0 1.5rem;color:#282a2e;background:#faf9f6;font:18px/1.7 system-ui,sans-serif}}h1,h2,h3,h4,h5,h6{{line-height:1.3}}a{{color:#416c8f}}pre{{overflow-wrap:anywhere;white-space:pre-wrap;background:#eeede9;padding:1rem;border-radius:6px}}code{{font-family:monospace}}blockquote{{margin-left:0;padding-left:1.4rem;border-left:3px solid #bbb;color:#62656b}}table{{border-collapse:collapse}}td,th{{border:1px solid #bbb;padding:.4rem .8rem}}img{{max-width:100%}}hr{{border:0;border-top:1px solid #bbb;margin:2rem 0}}@media print{{body{{margin:0;max-width:none;background:white;font-size:11pt}}a{{color:inherit}}pre,blockquote{{break-inside:avoid}}}}</style></head><body>\n{body}</body></html>\n",
        escape(title)
    )
}

pub fn save_html(markdown: &str, title: &str, path: &Path, overwrite: bool) -> Result<()> {
    let mut document = crate::storage::Document::new();
    document.set_text(render_html(markdown, title));
    document.save(path, overwrite)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exports_formatting_without_executing_source_or_fetching_images() {
        let html = render_html(
            "# Hello\n\n**Bold** and *italic*.\n\n<script>alert(1)</script>\n\n![photo](https://example.com/tracker.png)\n\n[bad](javascript:alert%281%29)",
            "<title>",
        );
        assert!(html.contains("<h1>Hello</h1>"));
        assert!(html.contains("<strong>Bold</strong>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>"));
        assert!(!html.contains("<img"));
        assert!(!html.contains("javascript:"));
        assert!(html.contains("&lt;title&gt;"));
    }
    #[test]
    fn refuses_to_replace_an_export_without_consent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.html");
        save_html("first", "Note", &path, false).unwrap();
        assert!(save_html("second", "Note", &path, false).is_err());
        assert!(std::fs::read_to_string(path).unwrap().contains("first"));
    }
}
