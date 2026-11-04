//! HTML, JSON and PDF export.

use crate::block::{plain_text, Block, BlockKind};
use crate::page::Page;
use crate::richtext::{MarkKind, RichText};

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn rich_html(t: &RichText) -> String {
    let mut out = String::new();
    for seg in t.segments() {
        let mut s = esc(&seg.text);
        for m in &seg.marks {
            s = match m {
                MarkKind::Bold => format!("<strong>{s}</strong>"),
                MarkKind::Italic => format!("<em>{s}</em>"),
                MarkKind::Strike => format!("<del>{s}</del>"),
                MarkKind::Code => format!("<code>{s}</code>"),
                MarkKind::Link(u) => format!("<a href=\"{}\">{s}</a>", esc(u)),
                MarkKind::Color([r, g, b]) => format!("<span style=\"color:rgb({r},{g},{b})\">{s}</span>"),
            };
        }
        out.push_str(&s);
    }
    out
}

fn blocks_html(blocks: &[Block], out: &mut String) {
    let mut open: Option<&str> = None;
    for b in blocks {
        let want = match b.kind {
            BlockKind::Bulleted | BlockKind::Todo => Some("ul"),
            BlockKind::Numbered => Some("ol"),
            _ => None,
        };
        if open != want {
            if let Some(tag) = open {
                out.push_str(&format!("</{tag}>\n"));
            }
            if let Some(tag) = want {
                out.push_str(&format!("<{tag}>\n"));
            }
            open = want;
        }
        let t = rich_html(&b.text);
        match b.kind {
            BlockKind::Paragraph => out.push_str(&format!("<p>{t}</p>\n")),
            BlockKind::Heading(n) => {
                let n = n.clamp(1, 3);
                out.push_str(&format!("<h{n}>{t}</h{n}>\n"));
            }
            BlockKind::Bulleted | BlockKind::Numbered => out.push_str(&format!("<li>{t}</li>\n")),
            BlockKind::Todo => out.push_str(&format!(
                "<li><input type=\"checkbox\" disabled{}> {t}</li>\n",
                if b.checked { " checked" } else { "" }
            )),
            BlockKind::Toggle => {
                out.push_str(&format!("<details><summary>{t}</summary>\n"));
                blocks_html(&b.children, out);
                out.push_str("</details>\n");
                continue;
            }
            BlockKind::Quote => out.push_str(&format!("<blockquote>{t}</blockquote>\n")),
            BlockKind::Callout => out.push_str(&format!("<aside>{} {t}</aside>\n", b.emoji)),
            BlockKind::Code => out.push_str(&format!(
                "<pre><code class=\"language-{}\">{}</code></pre>\n",
                esc(&b.language),
                esc(&b.text.text)
            )),
            BlockKind::Divider => out.push_str("<hr>\n"),
            BlockKind::Image => out.push_str(&format!("<img src=\"{}\" alt=\"\">\n", esc(&b.url))),
            BlockKind::Table => {
                out.push_str("<table>\n");
                for (i, row) in b.table.iter().enumerate() {
                    let tag = if i == 0 { "th" } else { "td" };
                    out.push_str("<tr>");
                    for c in row {
                        out.push_str(&format!("<{tag}>{}</{tag}>", esc(c)));
                    }
                    out.push_str("</tr>\n");
                }
                out.push_str("</table>\n");
            }
        }
        if b.kind != BlockKind::Toggle {
            blocks_html(&b.children, out);
        }
    }
    if let Some(tag) = open {
        out.push_str(&format!("</{tag}>\n"));
    }
}

pub fn page_to_html(page: &Page) -> String {
    let mut body = String::new();
    blocks_html(&page.blocks, &mut body);
    format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\"><title>{t}</title>\
<style>body{{font-family:system-ui,sans-serif;max-width:760px;margin:48px auto;line-height:1.6;color:#222}}\
pre{{background:#f4f4f2;padding:12px;border-radius:6px}}aside{{background:#f4f4f2;padding:12px;border-radius:6px}}\
td,th{{border:1px solid #ddd;padding:4px 8px}}</style></head><body>\n<h1>{i} {t}</h1>\n{body}</body></html>\n",
        t = esc(&page.title),
        i = page.icon,
    )
}

pub fn page_to_json(page: &Page) -> String {
    serde_json::to_string_pretty(page).unwrap_or_default()
}

/// Render a page as a simple text-only PDF (Helvetica, A4).
/// Non-ASCII characters are replaced by `?` because the base-14 fonts only
/// cover WinAnsi; embedding a TTF subset is listed in docs/ROADMAP.md.
pub fn page_to_pdf(page: &Page) -> Vec<u8> {
    let mut text = String::new();
    plain_text(&page.blocks, &mut text);
    text_to_pdf(&page.title, &text)
}

pub fn text_to_pdf(title: &str, text: &str) -> Vec<u8> {
    const COLS: usize = 92;
    const ROWS: usize = 52;
    let mut lines: Vec<String> = vec![title.to_owned(), String::new()];
    for raw in text.lines() {
        let chars: Vec<char> = raw.chars().collect();
        if chars.is_empty() {
            lines.push(String::new());
        }
        for chunk in chars.chunks(COLS) {
            lines.push(chunk.iter().collect());
        }
    }
    let pages: Vec<&[String]> = lines.chunks(ROWS).collect();

    fn add_obj(buf: &mut Vec<u8>, offsets: &mut Vec<usize>, body: &str) {
        offsets.push(buf.len());
        buf.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", offsets.len(), body).as_bytes());
    }
    fn pdf_escape(s: &str) -> String {
        s.chars()
            .map(|c| match c {
                '(' => "\\(".to_owned(),
                ')' => "\\)".to_owned(),
                '\\' => "\\\\".to_owned(),
                c if c.is_ascii() && !c.is_ascii_control() => c.to_string(),
                _ => "?".to_owned(),
            })
            .collect()
    }

    let mut buf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", 4 + 2 * i)).collect();
    add_obj(&mut buf, &mut offsets, "<< /Type /Catalog /Pages 2 0 R >>");
    add_obj(
        &mut buf,
        &mut offsets,
        &format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()),
    );
    add_obj(&mut buf, &mut offsets, "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    for (i, page_lines) in pages.iter().enumerate() {
        add_obj(
            &mut buf,
            &mut offsets,
            &format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
                5 + 2 * i
            ),
        );
        let mut content = String::from("BT /F1 11 Tf 50 800 Td 15 TL\n");
        for l in page_lines.iter() {
            content.push_str(&format!("({}) Tj T*\n", pdf_escape(l)));
        }
        content.push_str("ET");
        add_obj(
            &mut buf,
            &mut offsets,
            &format!("<< /Length {} >>\nstream\n{}\nendstream", content.len(), content),
        );
    }
    let xref_pos = buf.len();
    let mut xref = format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1);
    for o in &offsets {
        xref.push_str(&format!("{o:010} 00000 n \n"));
    }
    xref.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n",
        offsets.len() + 1
    ));
    buf.extend_from_slice(xref.as_bytes());
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_has_structure() {
        let pdf = text_to_pdf("T (x)", "hello\nworld é");
        let s = String::from_utf8_lossy(&pdf);
        assert!(s.starts_with("%PDF-1.4"));
        assert!(s.contains("startxref"));
        assert!(s.contains("T \\(x\\)"));
        assert!(s.trim_end().ends_with("%%EOF"));
    }

    #[test]
    fn html_lists_are_grouped() {
        let mut p = Page::new("t", None);
        p.blocks = vec![
            Block::with_text(BlockKind::Bulleted, "a"),
            Block::with_text(BlockKind::Bulleted, "b"),
        ];
        let h = page_to_html(&p);
        assert_eq!(h.matches("<ul>").count(), 1);
    }
}
