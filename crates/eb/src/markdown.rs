//! Small, dependency-free Markdown renderer for agent messages.
//!
//! The product renders model output as Markdown (paragraphs, lists, fenced code
//! with a copy affordance). A single wrapping label showed backticks and lost
//! the code card entirely, which is especially visible for programming answers.

use gtk::prelude::*;

use crate::widgets::{class, icon_button, text};

pub fn render(value: &str) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 9);
    class(&root, "eb-markdown");

    let mut paragraph: Vec<String> = Vec::new();
    let mut code: Option<(String, Vec<String>)> = None;

    for line in value.lines() {
        if let Some((_language, body)) = &mut code {
            if line.trim_start().starts_with("```") {
                flush_paragraph(&root, &mut paragraph);
                let (language, body) = code.take().unwrap();
                root.pack_start(&code_block(&language, &body.join("\n")), false, false, 0);
            } else {
                body.push(line.to_string());
            }
            continue;
        }

        let trimmed = line.trim_end();
        if let Some(language) = trimmed.trim_start().strip_prefix("```") {
            flush_paragraph(&root, &mut paragraph);
            code = Some((language.trim().to_string(), Vec::new()));
            continue;
        }
        if trimmed.trim().is_empty() {
            flush_paragraph(&root, &mut paragraph);
            continue;
        }
        if let Some((level, rest)) = heading(trimmed) {
            flush_paragraph(&root, &mut paragraph);
            let label = gtk::Label::new(None);
            label.set_selectable(true);
    label.set_markup(&inline(rest));
            label.set_line_wrap(true);
            label.set_width_chars(1);
            label.set_xalign(0.0);
            let class_name = if level <= 2 { "eb-md-h1" } else { "eb-md-h2" };
            class(&label, class_name);
            root.pack_start(&label, false, false, 0);
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            flush_paragraph(&root, &mut paragraph);
            root.pack_start(&bullet("*", rest), false, false, 0);
            continue;
        }
        if let Some((number, rest)) = numbered(trimmed) {
            flush_paragraph(&root, &mut paragraph);
            root.pack_start(&bullet(&number, rest), false, false, 0);
            continue;
        }
        paragraph.push(trimmed.trim().to_string());
    }

    if let Some((language, body)) = code {
        root.pack_start(&code_block(&language, &body.join("\n")), false, false, 0);
    }
    flush_paragraph(&root, &mut paragraph);
    root
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

fn flush_paragraph(root: &gtk::Box, lines: &mut Vec<String>) {
    if lines.is_empty() {
        return;
    }
    let value = lines.join(" ");
    lines.clear();
    let label = gtk::Label::new(None);
    label.set_selectable(true);
    label.set_markup(&inline(&value));
    label.set_line_wrap(true);
    label.set_width_chars(1);
    label.set_xalign(0.0);
    class(&label, "eb-md-text");
    root.pack_start(&label, false, false, 0);
}

fn bullet(marker: &str, value: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    class(&row, "eb-md-bullet");
    let bullet = text(marker, "eb-md-marker");
    bullet.set_valign(gtk::Align::Start);
    row.pack_start(&bullet, false, false, 0);
    let label = gtk::Label::new(None);
    label.set_selectable(true);
    label.set_markup(&inline(value));
    label.set_line_wrap(true);
    label.set_width_chars(1);
    label.set_xalign(0.0);
    class(&label, "eb-md-text");
    row.pack_start(&label, true, true, 0);
    row
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
                    "<span font_family=\"monospace\" foreground=\"#c7254e\">{}</span>",
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
                index = end + 1;
                continue;
            }
        }
        if chars[index] == '*' {
            if let Some(end) = chars[index + 1..].iter().position(|ch| *ch == '*') {
                let end = index + 1 + end;
                let italic: String = chars[index + 1..end].iter().collect();
                out.push_str(&format!("<i>{}</i>", escape(&italic)));
                index = end + 1;
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
}
