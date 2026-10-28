//! Small, dependency-free Markdown renderer for agent messages.
//!
//! The product renders model output as Markdown (paragraphs, lists, fenced code
//! with a copy affordance). A single wrapping label showed backticks and lost
//! the code card entirely, which is especially visible for programming answers.

use gtk::prelude::*;

use crate::widgets::{class, icon_button, text, wrap_column};

pub fn render(value: &str) -> gtk::Box {
    // `wrap_column` reports a 1px minimum. A text view's own minimum is the
    // longest line, which the message scroller would otherwise scroll sideways.
    let root = wrap_column(9);
    class(&root, "eb-markdown");

    // Text stays in one selectable TextView until a fenced code card, so a drag
    // can cross wrapped lines and paragraphs. Paragraphs are separate buffer
    // lines with a 12px gap, not a blank line.
    for chunk in parse(value) {
        match chunk {
            Chunk::Text(segs) => {
                let view = markdown_view();
                fill_view(&view, &segs);
                root.pack_start(&view, false, false, 0);
            }
            Chunk::Code { language, body } => {
                root.pack_start(&code_block(&language, &body), false, false, 0);
            }
        }
    }
    root
}

/// Body copy is 15px in a 26px line box. Live agent paragraphs add 12px under
/// every paragraph except the last. GTK has no `line-height`, so the line box
/// is split into above / inside-wrap / below pixels on the text view.
const BODY_LINE_PX: i32 = 26;
const PARAGRAPH_GAP: i32 = 12;

/// One run of consecutive markdown text lines.
enum Seg {
    Para(String),
    Heading(usize, String),
    Item(String, String),
}

enum Chunk {
    Text(Vec<Seg>),
    Code { language: String, body: String },
}

/// Replace the buffer of an existing markdown text view. Returns false when
/// `value` contains a fence, so the caller rebuilds the code card instead.
pub fn write_markdown_text(view: &gtk::TextView, value: &str) -> bool {
    let chunks = parse(value);
    if chunks
        .iter()
        .any(|chunk| matches!(chunk, Chunk::Code { .. }))
    {
        return false;
    }
    let segs = chunks.into_iter().fold(Vec::new(), |mut all, chunk| {
        if let Chunk::Text(segs) = chunk {
            all.extend(segs);
        }
        all
    });
    fill_view(view, &segs);
    true
}

/// Pad a 15px text view out to the measured 26px line box.
///
/// The stylesheet may not have resolved the 15px font until the view is
/// realized, so callers also connect this to `realize`. It refreshes the
/// paragraph-gap tag when that tag is already on the buffer.
pub fn apply_line_box(view: &gtk::TextView) {
    let extra = line_extra(view);
    let above = extra / 2;
    let below = extra - above;
    view.set_pixels_above_lines(above);
    view.set_pixels_below_lines(below);
    view.set_pixels_inside_wrap(extra);
    let Some(buffer) = view.buffer() else {
        return;
    };
    let Some(table) = buffer.tag_table() else {
        return;
    };
    let Some(tag) = table.lookup("eb-md-gap") else {
        return;
    };
    tag.set_pixels_below_lines(below + PARAGRAPH_GAP);
    tag.set_pixels_below_lines_set(true);
}

fn line_extra(view: &gtk::TextView) -> i32 {
    let font = view.style_context().font(gtk::StateFlags::NORMAL);
    let metrics = view.pango_context().metrics(Some(&font), None);
    let natural = (metrics.ascent() + metrics.descent()) / gtk::pango::SCALE;
    if (12..=BODY_LINE_PX).contains(&natural) {
        BODY_LINE_PX - natural
    } else {
        8
    }
}

fn markdown_view() -> gtk::TextView {
    let view = gtk::TextView::new();
    view.set_editable(false);
    view.set_cursor_visible(false);
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.set_left_margin(2);
    view.set_right_margin(2);
    class(&view, "eb-md-view");
    view.connect_realize(apply_line_box);
    view
}

fn fill_view(view: &gtk::TextView, segs: &[Seg]) {
    apply_line_box(view);
    let below = view.pixels_below_lines();
    let buffer = view.buffer().expect("text view has a buffer");
    buffer.set_text("");
    let gap = paragraph_gap_tag(&buffer, below + PARAGRAPH_GAP);
    for (index, seg) in segs.iter().enumerate() {
        let start_offset = buffer.end_iter().offset();
        let mut iter = buffer.end_iter();
        buffer.insert_markup(&mut iter, &seg_markup(seg));
        let end_offset = iter.offset();
        let last = index + 1 == segs.len();
        let tight_list = matches!(seg, Seg::Item(_, _))
            && segs
                .get(index + 1)
                .is_some_and(|next| matches!(next, Seg::Item(_, _)));
        if !last && !tight_list {
            let start = buffer.iter_at_offset(start_offset);
            let end = buffer.iter_at_offset(end_offset);
            buffer.apply_tag(&gap, &start, &end);
        }
        if !last {
            let mut iter = buffer.end_iter();
            buffer.insert(&mut iter, "\n");
        }
    }
}

fn paragraph_gap_tag(buffer: &gtk::TextBuffer, below: i32) -> gtk::TextTag {
    let table = buffer
        .tag_table()
        .expect("text buffer has a tag table");
    if let Some(tag) = table.lookup("eb-md-gap") {
        tag.set_pixels_below_lines(below);
        tag.set_pixels_below_lines_set(true);
        return tag;
    }
    let tag = gtk::TextTag::new(Some("eb-md-gap"));
    tag.set_pixels_below_lines(below);
    tag.set_pixels_below_lines_set(true);
    table.add(&tag);
    tag
}

fn seg_markup(seg: &Seg) -> String {
    match seg {
        Seg::Para(value) => inline(value),
        Seg::Heading(level, value) => {
            // CSS already records h1 at 20px and h2 at 17px. Pango absolute
            // size is 1024ths of a point; at 96dpi a pixel is 0.75pt.
            let px = match level {
                1 => 20,
                2 => 17,
                _ => 15,
            };
            let size = px * 72 * gtk::pango::SCALE / 96;
            format!(
                "<span size=\"{size}\" weight=\"600\">{}</span>",
                inline(value)
            )
        }
        Seg::Item(marker, value) => format!("{}  {}", escape(marker), inline(value)),
    }
}

fn parse(value: &str) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let mut segs = Vec::new();
    let mut paragraph = Vec::new();
    let mut code: Option<(String, Vec<String>)> = None;
    let lines: Vec<&str> = value.lines().collect();

    for (position, line) in lines.iter().copied().enumerate() {
        if code.is_some() {
            if line.trim_start().starts_with("```") {
                if let Some((language, body)) = code.take() {
                    chunks.push(Chunk::Code {
                        language,
                        body: body.join("\n"),
                    });
                }
            } else if let Some((_, body)) = code.as_mut() {
                body.push(line.to_string());
            }
            continue;
        }

        let trimmed = line.trim_end();
        if let Some(language) = trimmed.trim_start().strip_prefix("```") {
            flush_text(&mut chunks, &mut segs, &mut paragraph);
            code = Some((language.trim().to_string(), Vec::new()));
            continue;
        }
        if trimmed.trim().is_empty() {
            // A blank line between list items stays tight. Anywhere else it
            // ends the paragraph.
            let next_content = lines[position + 1..]
                .iter()
                .find(|candidate| !candidate.trim().is_empty());
            let continues_list = next_content.is_some_and(|candidate| is_list_item(candidate));
            if continues_list && paragraph.is_empty() {
                continue;
            }
            flush_paragraph(&mut segs, &mut paragraph);
            continue;
        }
        if let Some((level, rest)) = heading(trimmed) {
            flush_paragraph(&mut segs, &mut paragraph);
            segs.push(Seg::Heading(level, rest.trim().to_string()));
            continue;
        }
        if let Some(rest) = trimmed
            .trim_start()
            .strip_prefix("- ")
            .or_else(|| trimmed.trim_start().strip_prefix("* "))
        {
            flush_paragraph(&mut segs, &mut paragraph);
            segs.push(Seg::Item("•".into(), rest.trim().to_string()));
            continue;
        }
        if let Some((number, rest)) = numbered(trimmed.trim_start()) {
            flush_paragraph(&mut segs, &mut paragraph);
            segs.push(Seg::Item(number, rest.trim().to_string()));
            continue;
        }
        paragraph.push(trimmed.trim().to_string());
    }

    if let Some((language, body)) = code.take() {
        flush_text(&mut chunks, &mut segs, &mut paragraph);
        chunks.push(Chunk::Code {
            language,
            body: body.join("\n"),
        });
    }
    flush_text(&mut chunks, &mut segs, &mut paragraph);
    chunks
}

fn is_list_item(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("- ") || line.starts_with("* ") || numbered(line).is_some()
}

fn flush_paragraph(segs: &mut Vec<Seg>, paragraph: &mut Vec<String>) {
    if paragraph.is_empty() {
        return;
    }
    let value = paragraph.join(" ");
    paragraph.clear();
    segs.push(Seg::Para(value));
}

fn flush_text(chunks: &mut Vec<Chunk>, segs: &mut Vec<Seg>, paragraph: &mut Vec<String>) {
    flush_paragraph(segs, paragraph);
    if !segs.is_empty() {
        chunks.push(Chunk::Text(std::mem::take(segs)));
    }
}

fn heading(line: &str) -> Option<(usize, &str)> {
    let hashes = line.chars().take_while(|ch| *ch == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = line[hashes..].strip_prefix(' ')?;
    Some((hashes, rest))
}

fn numbered(line: &str) -> Option<(String, &str)> {
    let dot = line.find(". ")?;
    let prefix = &line[..dot];
    if prefix.is_empty() || !prefix.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some((format!("{prefix}."), &line[dot + 2..]))
}

fn code_block(language: &str, code: &str) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    class(&card, "eb-code-block");
    card.set_hexpand(true);

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    class(&header, "eb-code-header");
    header.set_margin_top(8);
    header.set_margin_bottom(4);
    header.set_margin_start(12);
    header.set_margin_end(8);
    let language = if language.is_empty() { "code" } else { language };
    header.pack_start(&text(language, "eb-code-language"), false, false, 0);

    let copy = icon_button("⧉", "eb-icon-mini", "Copy code");
    let code_value = code.to_string();
    copy.connect_clicked(move |button| {
        let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
        clipboard.set_text(&code_value);
        clipboard.store();
        crate::widgets::flash(button, "✓", "Copied");
    });
    header.pack_end(&copy, false, false, 0);
    card.pack_start(&header, false, false, 0);

    let label = gtk::Label::new(None);
    label.set_markup(&highlight(code, language));
    label.set_selectable(true);
    label.set_xalign(0.0);
    label.set_line_wrap(false);
    label.set_margin_top(4);
    label.set_margin_bottom(12);
    label.set_margin_start(16);
    label.set_margin_end(16);
    class(&label, "eb-code");

    let scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    scroll.set_propagate_natural_width(false);
    // The label does not wrap; the card scrolls it. Without a content minimum
    // the long line becomes the card's minimum and the transcript scrolls too.
    scroll.set_min_content_width(1);
    scroll.set_hexpand(true);
    scroll.set_max_content_width(crate::spec::MESSAGE_WIDTH_CLOSED);
    let lines = code.lines().count().max(1) as i32;
    let height = (lines * 20 + 24).clamp(48, 360);
    scroll.set_min_content_height(height);
    scroll.set_size_request(-1, height);
    scroll.add(&label);
    card.pack_start(&scroll, true, true, 0);
    card
}

/// Escape plain text for Pango, then recognize **bold**, *italic* and
/// `inline code`. Deliberately small; links stay plain text.
fn inline(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '`' {
            if let Some(end) = chars[index + 1..].iter().position(|ch| *ch == '`') {
                let end = index + 1 + end;
                let code: String = chars[index + 1..end].iter().collect();
                out.push_str(&format!(
                    "<span font_family=\"monospace\" background=\"#efefef\">{}</span>",
                    escape(&code)
                ));
                index = end + 1;
                continue;
            }
        }
        if chars[index..].starts_with(&['*', '*']) {
            if let Some(end) = chars[index + 2..].iter().position(|ch| *ch == '*') {
                let end = index + 2 + end;
                let bold: String = chars[index + 2..end].iter().collect();
                out.push_str(&format!("<b>{}</b>", escape(&bold)));
                index = end + 2;
                continue;
            }
        }
        if chars[index] == '*' {
            if let Some(end) = chars[index + 1..].iter().position(|ch| *ch == '*') {
                let end = index + 1 + end;
                let italic: String = chars[index + 1..end].iter().collect();
                out.push_str(&format!("<i>{}</i>", escape(&italic)));
                index = end + 2;
                continue;
            }
        }
        out.push_str(&escape(&chars[index].to_string()));
        index += 1;
    }
    out
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn highlight(code: &str, language: &str) -> String {
    let java = matches!(language.to_ascii_lowercase().as_str(), "java");
    let keywords: &[&str] = if java {
        &[
            "abstract", "boolean", "break", "class", "continue", "else", "final",
            "for", "if", "import", "int", "new", "private", "public", "return",
            "static", "String", "super", "this", "void", "while",
        ]
    } else {
        &["if", "then", "else", "for", "do", "done", "echo", "cd", "export", "set"]
    };
    let chars: Vec<char> = code.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index..].starts_with(&['/', '/']) {
            let end = chars[index..]
                .iter()
                .position(|ch| *ch == '\n')
                .map(|offset| index + offset)
                .unwrap_or(chars.len());
            let comment: String = chars[index..end].iter().collect();
            out.push_str(&format!(
                "<span foreground=\"#6a737d\">{}</span>",
                escape(&comment)
            ));
            index = end;
            continue;
        }
        if chars[index] == '"' {
            let mut end = index + 1;
            while end < chars.len() {
                if chars[end] == '\\' {
                    end += 2;
                    continue;
                }
                if chars[end] == '"' {
                    end += 1;
                    break;
                }
                end += 1;
            }
            let string: String = chars[index..end.min(chars.len())].iter().collect();
            out.push_str(&format!(
                "<span foreground=\"#032f62\">{}</span>",
                escape(&string)
            ));
            index = end;
            continue;
        }
        if chars[index].is_alphabetic() || chars[index] == '_' {
            let mut end = index + 1;
            while end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_') {
                end += 1;
            }
            let word: String = chars[index..end].iter().collect();
            if keywords.contains(&word.as_str()) {
                out.push_str(&format!(
                    "<span foreground=\"#d73a49\">{}</span>",
                    escape(&word)
                ));
            } else {
                out.push_str(&escape(&word));
            }
            index = end;
            continue;
        }
        out.push_str(&escape(&chars[index].to_string()));
        index += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_code_and_emphasis_are_markup() {
        let value = inline("run `cargo test` and **ship**");
        assert!(value.contains("<span"));
        assert!(value.contains("<b>ship</b>"));
        assert!(!value.contains('`'));
    }

    #[test]
    fn plain_text_is_escaped_and_headings_parse() {
        assert!(inline("<tag> & text").contains("&lt;tag&gt; &amp; text"));
        assert_eq!(heading("## Plan"), Some((2, "Plan")));
        assert_eq!(heading("not a heading"), None);
    }

    #[test]
    fn headings_use_the_css_sizes() {
        let h1 = seg_markup(&Seg::Heading(1, "Title".into()));
        let h2 = seg_markup(&Seg::Heading(2, "Title".into()));
        assert!(h1.contains("size=\"15360\""), "{h1}");
        assert!(h2.contains("size=\"13056\""), "{h2}");
        assert!(h1.contains("weight=\"600\""));
    }

    #[test]
    fn paragraph_gap_is_on_every_paragraph_but_the_last_and_list_stays_tight() {
        if crate::widgets::try_init_gtk_for_test().is_none() {
            eprintln!("skipping: gtk needs the main thread and a display");
            return;
        }
        let root = render("one\n\ntwo");
        let view = root
            .children()
            .into_iter()
            .find_map(|child| child.downcast::<gtk::TextView>().ok())
            .expect("paragraphs share one text view");
        let buffer = view.buffer().unwrap();
        let start = buffer.start_iter();
        assert!(
            start.tags().iter().any(|tag| tag.name().as_deref() == Some("eb-md-gap")),
            "the first paragraph keeps the 12px gap"
        );
        let mut end = buffer.end_iter();
        assert!(end.backward_char());
        assert!(
            !end.tags().iter().any(|tag| tag.name().as_deref() == Some("eb-md-gap")),
            "the last paragraph has no trailing gap"
        );

        let listed = render("- a\n\n- b");
        let list_view = listed
            .children()
            .into_iter()
            .find_map(|child| child.downcast::<gtk::TextView>().ok())
            .expect("a tight list is one text view");
        let list_buffer = list_view.buffer().unwrap();
        let mut walk = list_buffer.start_iter();
        let mut gapped = false;
        loop {
            if walk.tags().iter().any(|tag| tag.name().as_deref() == Some("eb-md-gap")) {
                gapped = true;
                break;
            }
            if !walk.forward_char() {
                break;
            }
        }
        assert!(!gapped, "consecutive list items stay on the line box");
    }
}

/// Every markdown run must render exactly once, in order: this guards the
    /// streaming incremental renderer against double-packing.
    #[test]
    fn render_renders_each_run_once_in_order() {
        if crate::widgets::try_init_gtk_for_test().is_none() {
            eprintln!("skipping: gtk needs the main thread and a display");
            return;
        }
        let value = "Here's the Java Quine:\n\n**How it works:**\n\n1. **char q = 34** — stores\n2. **String[] a** — contains\n\n- `10` → newline\n- `34` → `\"`\n\nThis produces the exact source.";
        let root = render(value);
        let mut texts: Vec<String> = Vec::new();
        for child in root.children() {
            if let Ok(view) = child.downcast::<gtk::TextView>() {
                let buffer = view.buffer().unwrap();
                texts.push(
                    buffer
                        .text(&buffer.start_iter(), &buffer.end_iter(), false)
                        .map(|t| t.to_string())
                        .unwrap_or_default(),
                );
            }
        }
        let joined = texts.join("|");
        eprintln!("runs: {texts:?}");
        let count = |needle: &str| joined.matches(needle).count();
        assert_eq!(count("Here's the Java Quine"), 1, "paragraph duplicated");
        assert_eq!(count("How it works"), 1, "heading duplicated");
        assert_eq!(count("char q = 34"), 1, "item duplicated");
        assert_eq!(count("This produces"), 1, "closing duplicated");
        // order: paragraph before heading before closing
        let first = joined.find("Here's the Java Quine").unwrap();
        let heading = joined.find("How it works").unwrap();
        let closing = joined.find("This produces").unwrap();
        assert!(first < heading && heading < closing, "runs out of order");
    }

