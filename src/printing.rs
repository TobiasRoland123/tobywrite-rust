use anyhow::{Result, ensure};
use gtk::{pango, prelude::*};
use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use std::{cell::RefCell, path::Path, rc::Rc};

/// Render supported Markdown as Pango markup; embedded HTML remains literal.
pub fn print_markup(markdown: &str) -> String {
    let mut out = String::new();
    let mut lists: Vec<Option<u64>> = Vec::new();
    for event in Parser::new_ext(markdown, crate::export::options()) {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    let size = match level as u8 {
                        1 => 22,
                        2 => 18,
                        3 => 15,
                        _ => 12,
                    };
                    out.push_str(&format!(
                        "<span weight=\"bold\" size=\"{}\">",
                        size * pango::SCALE
                    ));
                }
                Tag::Strong => out.push_str("<b>"),
                Tag::Emphasis => out.push_str("<i>"),
                Tag::Strikethrough => out.push_str("<s>"),
                Tag::CodeBlock(_) => {
                    out.push_str("<span font_family=\"monospace\" size=\"small\">")
                }
                Tag::BlockQuote(_) => out.push_str("<span foreground=\"#555555\"><i>"),
                Tag::Link { .. } => out.push_str("<span underline=\"single\">"),
                Tag::List(start) => lists.push(start),
                Tag::Item => {
                    out.push_str(&"  ".repeat(lists.len().saturating_sub(1)));
                    if let Some(Some(number)) = lists.last_mut() {
                        out.push_str(&format!("{number}. "));
                        *number = number.saturating_add(1);
                    } else {
                        out.push_str("• ");
                    }
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Heading(_) => out.push_str("</span>\n\n"),
                TagEnd::Strong => out.push_str("</b>"),
                TagEnd::Emphasis => out.push_str("</i>"),
                TagEnd::Strikethrough => out.push_str("</s>"),
                TagEnd::CodeBlock => out.push_str("</span>\n"),
                TagEnd::BlockQuote(_) => out.push_str("</i></span>\n"),
                TagEnd::Link => out.push_str("</span>"),
                TagEnd::Paragraph => out.push_str("\n\n"),
                TagEnd::Item => out.push('\n'),
                TagEnd::List(_) => {
                    lists.pop();
                    out.push('\n');
                }
                TagEnd::TableCell => out.push_str("    "),
                TagEnd::TableRow | TagEnd::TableHead => out.push('\n'),
                _ => {}
            },
            Event::Text(text)
            | Event::Html(text)
            | Event::InlineHtml(text)
            | Event::InlineMath(text)
            | Event::DisplayMath(text)
            | Event::FootnoteReference(text) => out.push_str(&crate::export::escape(&text)),
            Event::Code(text) => {
                out.push_str(&format!("<tt>{}</tt>", crate::export::escape(&text)))
            }
            Event::SoftBreak => out.push(' '),
            Event::HardBreak => out.push('\n'),
            Event::Rule => out.push_str("\n────────────────────────────────\n\n"),
            Event::TaskListMarker(done) => out.push_str(if done { "[x] " } else { "[ ] " }),
        }
    }
    out.trim_end().to_string()
}

#[derive(Clone)]
struct PrintLine {
    line: pango::LayoutLine,
    height: f64,
    baseline: f64,
}

fn operation(markdown: &str, title: &str) -> gtk::PrintOperation {
    let op = gtk::PrintOperation::new();
    op.set_job_name(title);
    op.set_unit(gtk::Unit::Points);
    op.set_allow_async(false);
    let setup = gtk::PageSetup::new();
    setup.set_top_margin(18.0, gtk::Unit::Mm);
    setup.set_bottom_margin(18.0, gtk::Unit::Mm);
    setup.set_left_margin(20.0, gtk::Unit::Mm);
    setup.set_right_margin(20.0, gtk::Unit::Mm);
    op.set_default_page_setup(Some(&setup));
    let pages: Rc<RefCell<Vec<Vec<PrintLine>>>> = Rc::default();
    let markup = print_markup(markdown);
    let page_data = pages.clone();
    op.connect_begin_print(move |op, context| {
        let layout = context.create_pango_layout();
        layout.set_font_description(Some(&pango::FontDescription::from_string("Sans 11")));
        layout.set_width((context.width() * f64::from(pango::SCALE)) as i32);
        layout.set_wrap(pango::WrapMode::WordChar);
        layout.set_markup(&markup);
        let mut pages = vec![Vec::new()];
        let mut y = 0.0;
        for line in layout.lines_readonly() {
            let logical = line.extents().1;
            let height = (f64::from(logical.height()) / f64::from(pango::SCALE)).max(13.0) * 1.3;
            if y + height > context.height() - 28.0 && !pages.last().unwrap().is_empty() {
                pages.push(Vec::new());
                y = 0.0;
            }
            let baseline = -f64::from(logical.y()) / f64::from(pango::SCALE);
            pages.last_mut().unwrap().push(PrintLine {
                line,
                height,
                baseline,
            });
            y += height;
        }
        op.set_n_pages(pages.len() as i32);
        *page_data.borrow_mut() = pages;
    });
    let title = title.to_string();
    op.connect_draw_page(move |_, context, page| {
        let pages = pages.borrow();
        let Some(lines) = pages.get(page as usize) else {
            return;
        };
        let cr = context.cairo_context();
        cr.set_source_rgb(0.13, 0.13, 0.13);
        let mut y = 0.0;
        for line in lines {
            cr.move_to(0.0, y + line.baseline);
            pangocairo::functions::show_layout_line(&cr, &line.line);
            y += line.height;
        }
        let footer = context.create_pango_layout();
        footer.set_font_description(Some(&pango::FontDescription::from_string("Sans 8")));
        footer.set_text(&format!("{title}  ·  {} / {}", page + 1, pages.len()));
        footer.set_width((context.width() * f64::from(pango::SCALE)) as i32);
        footer.set_ellipsize(pango::EllipsizeMode::Middle);
        cr.set_source_rgb(0.4, 0.4, 0.4);
        cr.move_to(0.0, context.height() - 12.0);
        pangocairo::functions::show_layout(&cr, &footer);
    });
    op
}

pub fn print(parent: &impl IsA<gtk::Window>, markdown: &str, title: &str) -> Result<()> {
    operation(markdown, title).run(gtk::PrintOperationAction::PrintDialog, Some(parent))?;
    Ok(())
}

pub fn export_pdf(markdown: &str, title: &str, path: &Path) -> Result<()> {
    export_pdf_to(markdown, title, path, false)
}

pub fn export_pdf_to(markdown: &str, title: &str, path: &Path, overwrite: bool) -> Result<()> {
    ensure!(
        overwrite || !path.exists(),
        "{} already exists",
        path.display()
    );
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = tempfile::NamedTempFile::new_in(parent)?;
    let op = operation(markdown, title);
    op.set_export_filename(temporary.path());
    let result = op.run(gtk::PrintOperationAction::Export, None::<&gtk::Window>)?;
    ensure!(
        result == gtk::PrintOperationResult::Apply,
        "PDF export was cancelled"
    );
    let bytes = std::fs::read(temporary.path())?;
    ensure!(
        bytes.starts_with(b"%PDF-"),
        "The printer did not produce a PDF"
    );
    crate::storage::save_export_bytes(path, &bytes, overwrite)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_markdown_is_valid_pango_markup() {
        let markup = print_markup(
            "# Heading\n\n> **Bold & *italic***\n\n1. One\n2. Two\n\n- [x] Done\n\n```rust\nx < y\n```\n\n---\n\n<script>literal</script>",
        );
        let (_, plain, _) = pango::parse_markup(&markup, '\0').unwrap();
        assert!(plain.contains("Bold & italic"));
        assert!(plain.contains("1. One"));
        assert!(plain.contains("2. Two"));
        assert!(plain.contains("<script>literal</script>"));
        assert!(!plain.contains("```"));
    }
}
