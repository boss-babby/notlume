//! A tiny dependency-free syntax highlighter for code blocks. It tokenises
//! comments, strings, numbers and keywords for C-like / Python-like languages,
//! which covers the overwhelmingly common cases without pulling in syntect.

use crate::theme::Palette;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontFamily, FontId};

const KEYWORDS: &[&str] = &[
    "fn", "let", "mut", "pub", "struct", "enum", "impl", "trait", "use", "mod", "match", "if", "else", "for", "while",
    "loop", "return", "break", "continue", "const", "static", "async", "await", "move", "self", "Self", "true",
    "false", "def", "class", "import", "from", "as", "in", "not", "and", "or", "None", "function", "var", "new",
    "this", "null", "undefined", "typeof", "export", "default", "try", "catch", "finally", "throw", "type",
    "interface", "extends", "int", "float", "bool", "void", "char", "string", "where", "unsafe", "dyn",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tok {
    Plain,
    Keyword,
    Str,
    Num,
    Comment,
}

fn comment_prefix(lang: &str) -> &'static str {
    match lang {
        "python" | "py" | "sh" | "bash" | "yaml" | "toml" | "ruby" => "#",
        "sql" | "lua" => "--",
        _ => "//",
    }
}

fn tokenize(src: &str, lang: &str) -> Vec<(String, Tok)> {
    let cp: Vec<char> = comment_prefix(lang).chars().collect();
    let chars: Vec<char> = src.chars().collect();
    let mut out: Vec<(String, Tok)> = Vec::new();
    let mut push = |s: String, t: Tok| {
        if let Some(last) = out.last_mut() {
            if last.1 == t && t == Tok::Plain {
                last.0.push_str(&s);
                return;
            }
        }
        out.push((s, t));
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if chars[i..].starts_with(&cp) {
            let end = chars[i..].iter().position(|&x| x == '\n').map_or(chars.len(), |p| i + p);
            push(chars[i..end].iter().collect(), Tok::Comment);
            i = end;
        } else if c == '"' || c == '\'' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != c && chars[j] != '\n' {
                if chars[j] == '\\' {
                    j += 1;
                }
                j += 1;
            }
            let end = (j + 1).min(chars.len());
            push(chars[i..end].iter().collect(), Tok::Str);
            i = end;
        } else if c.is_ascii_digit() {
            let mut j = i;
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '.' || chars[j] == '_') {
                j += 1;
            }
            push(chars[i..j].iter().collect(), Tok::Num);
            i = j;
        } else if c.is_alphabetic() || c == '_' {
            let mut j = i;
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            let word: String = chars[i..j].iter().collect();
            let t = if KEYWORDS.contains(&word.as_str()) { Tok::Keyword } else { Tok::Plain };
            push(word, t);
            i = j;
        } else {
            push(c.to_string(), Tok::Plain);
            i += 1;
        }
    }
    out
}

/// Build a layout job for a code block.
pub fn highlight(text: &str, lang: &str, size: f32, pal: &Palette, wrap: f32) -> LayoutJob {
    let dark = pal.is_dark();
    let pick = |l: [u8; 3], d: [u8; 3]| {
        let c = if dark { d } else { l };
        Color32::from_rgb(c[0], c[1], c[2])
    };
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    let font = FontId::new(size, FontFamily::Monospace);
    for (s, t) in tokenize(text, &lang.to_lowercase()) {
        let color = match t {
            Tok::Plain => pal.text,
            Tok::Keyword => pick([175, 56, 130], [255, 123, 114]),
            Tok::Str => pick([24, 128, 56], [165, 214, 167]),
            Tok::Num => pick([180, 90, 0], [255, 190, 110]),
            Tok::Comment => pal.dim,
        };
        job.append(&s, 0.0, TextFormat::simple(font.clone(), color));
    }
    if job.sections.is_empty() {
        job.append("", 0.0, TextFormat::simple(font, pal.text));
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_rust() {
        let t = tokenize("let x = \"hi\"; // c\n42", "rust");
        assert!(t.iter().any(|(s, k)| s == "let" && *k == Tok::Keyword));
        assert!(t.iter().any(|(s, k)| s == "\"hi\"" && *k == Tok::Str));
        assert!(t.iter().any(|(s, k)| s == "// c" && *k == Tok::Comment));
        assert!(t.iter().any(|(s, k)| s == "42" && *k == Tok::Num));
        let joined: String = t.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(joined, "let x = \"hi\"; // c\n42");
    }
}
