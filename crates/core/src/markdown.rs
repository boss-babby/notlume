//! Markdown shortcuts while typing, plus Markdown import / export.

use crate::block::{Block, BlockKind};
use crate::richtext::RichText;

/// Detect a markdown shortcut at the start of a block's text.
/// Returns the new kind and how many leading chars to strip.
pub fn detect_shortcut(text: &str) -> Option<(BlockKind, usize)> {
    if text == "---" {
        return Some((BlockKind::Divider, 3));
    }
    if text == "```" {
        return Some((BlockKind::Code, 3));
    }
    const TABLE: &[(&str, BlockKind)] = &[
        ("### ", BlockKind::Heading(3)),
        ("## ", BlockKind::Heading(2)),
        ("# ", BlockKind::Heading(1)),
        ("- [ ] ", BlockKind::Todo),
        ("[] ", BlockKind::Todo),
        ("[ ] ", BlockKind::Todo),
        ("- ", BlockKind::Bulleted),
        ("* ", BlockKind::Bulleted),
        ("1. ", BlockKind::Numbered),
        ("> ", BlockKind::Quote),
        ("| ", BlockKind::Callout),
    ];
    TABLE
        .iter()
        .find(|(p, _)| text.starts_with(p))
        .map(|(p, k)| (*k, p.chars().count()))
}

pub fn blocks_to_markdown(blocks: &[Block], depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    let mut num = 0;
    for b in blocks {
        if b.kind == BlockKind::Numbered {
            num += 1;
        } else {
            num = 0;
        }
        let t = b.text.to_markdown();
        match b.kind {
            BlockKind::Paragraph => out.push_str(&format!("{pad}{t}\n\n")),
            BlockKind::Heading(n) => out.push_str(&format!("{}{} {t}\n\n", pad, "#".repeat(n.clamp(1, 3) as usize))),
            BlockKind::Bulleted | BlockKind::Toggle => out.push_str(&format!("{pad}- {t}\n")),
            BlockKind::Numbered => out.push_str(&format!("{pad}{num}. {t}\n")),
            BlockKind::Todo => out.push_str(&format!("{pad}- [{}] {t}\n", if b.checked { "x" } else { " " })),
            BlockKind::Quote => out.push_str(&format!("{pad}> {t}\n\n")),
            BlockKind::Callout => out.push_str(&format!("{pad}> {} {t}\n\n", b.emoji)),
            BlockKind::Code => out.push_str(&format!("{pad}```{}\n{}\n{pad}```\n\n", b.language, b.text.text)),
            BlockKind::Divider => out.push_str(&format!("{pad}---\n\n")),
            BlockKind::Image => out.push_str(&format!("{pad}![]({})\n\n", b.url)),
            BlockKind::Table => {
                for (i, row) in b.table.iter().enumerate() {
                    out.push_str(&format!("{pad}| {} |\n", row.join(" | ")));
                    if i == 0 {
                        out.push_str(&format!("{pad}|{}\n", " --- |".repeat(row.len())));
                    }
                }
                out.push('\n');
            }
        }
        // Children are indented one level under their parent.
        blocks_to_markdown(&b.children, depth + 1, out);
    }
}

/// Parse Markdown into blocks. Unknown constructs become paragraphs.
pub fn markdown_to_blocks(src: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let lines: Vec<&str> = src.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        i += 1;
        if trimmed.is_empty() {
            continue;
        }
        if let Some(lang) = trimmed.strip_prefix("```") {
            let mut code = Vec::new();
            while i < lines.len() && !lines[i].trim().starts_with("```") {
                code.push(lines[i]);
                i += 1;
            }
            i += 1; // closing fence
            let mut b = Block::with_text(BlockKind::Code, &code.join("\n"));
            b.language = if lang.is_empty() { "text".into() } else { lang.to_owned() };
            blocks.push(b);
        } else if trimmed == "---" || trimmed == "***" {
            blocks.push(Block::new(BlockKind::Divider));
        } else if let Some((lvl, rest)) = heading(trimmed) {
            blocks.push(rich(BlockKind::Heading(lvl), rest));
        } else if let Some(rest) = trimmed.strip_prefix("- [ ] ") {
            blocks.push(rich(BlockKind::Todo, rest));
        } else if let Some(rest) = trimmed.strip_prefix("- [x] ").or_else(|| trimmed.strip_prefix("- [X] ")) {
            let mut b = rich(BlockKind::Todo, rest);
            b.checked = true;
            blocks.push(b);
        } else if let Some(rest) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            blocks.push(rich(BlockKind::Bulleted, rest));
        } else if let Some(rest) = numbered(trimmed) {
            blocks.push(rich(BlockKind::Numbered, rest));
        } else if let Some(rest) = trimmed.strip_prefix("> ") {
            blocks.push(rich(BlockKind::Quote, rest));
        } else if let Some(url) = trimmed.strip_prefix("![").and_then(|r| r.split_once("](")).and_then(|(_, u)| u.strip_suffix(')')) {
            let mut b = Block::new(BlockKind::Image);
            b.url = url.to_owned();
            blocks.push(b);
        } else if trimmed.starts_with('|') {
            let mut rows: Vec<Vec<String>> = Vec::new();
            i -= 1;
            while i < lines.len() && lines[i].trim().starts_with('|') {
                let cells: Vec<String> = lines[i]
                    .trim()
                    .trim_matches('|')
                    .split('|')
                    .map(|c| c.trim().to_owned())
                    .collect();
                if !cells.iter().all(|c| c.chars().all(|ch| ch == '-' || ch == ':') && !c.is_empty()) {
                    rows.push(cells);
                }
                i += 1;
            }
            let mut b = Block::new(BlockKind::Table);
            b.table = rows;
            blocks.push(b);
        } else {
            blocks.push(rich(BlockKind::Paragraph, trimmed));
        }
    }
    blocks
}

fn rich(kind: BlockKind, md: &str) -> Block {
    let mut b = Block::new(kind);
    b.text = RichText::from_markdown(md);
    b
}

fn heading(s: &str) -> Option<(u8, &str)> {
    let n = s.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&n) && s[n..].starts_with(' ') {
        Some((n.min(3) as u8, s[n..].trim_start()))
    } else {
        None
    }
}

fn numbered(s: &str) -> Option<&str> {
    let digits = s.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && s[digits..].starts_with(". ") {
        Some(&s[digits + 2..])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts() {
        assert_eq!(detect_shortcut("## hi"), Some((BlockKind::Heading(2), 3)));
        assert_eq!(detect_shortcut("[] task"), Some((BlockKind::Todo, 3)));
        assert_eq!(detect_shortcut("- x"), Some((BlockKind::Bulleted, 2)));
        assert_eq!(detect_shortcut("plain"), None);
        assert_eq!(detect_shortcut("---"), Some((BlockKind::Divider, 3)));
    }

    #[test]
    fn import_export_roundtrip() {
        let md = "# Title\n\nhello **world**\n\n- [x] done\n- item\n\n```rust\nlet a = 1;\n```\n\n> quote\n\n";
        let blocks = markdown_to_blocks(md);
        assert_eq!(blocks.len(), 6);
        assert_eq!(blocks[0].kind, BlockKind::Heading(1));
        assert!(blocks[2].checked);
        let mut out = String::new();
        blocks_to_markdown(&blocks, 0, &mut out);
        assert!(out.contains("# Title"));
        assert!(out.contains("**world**"));
        assert!(out.contains("```rust\nlet a = 1;\n```"));
    }

    #[test]
    fn table_import() {
        let blocks = markdown_to_blocks("| a | b |\n| --- | --- |\n| 1 | 2 |\n");
        assert_eq!(blocks[0].table.len(), 2);
        assert_eq!(blocks[0].table[1], vec!["1", "2"]);
    }
}
