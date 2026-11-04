//! Rich text model: a plain `String` plus formatting marks over char ranges.
//!
//! Keeping the text plain makes it trivial to bind to an immediate-mode text
//! field; marks are re-aligned on every edit by [`RichText::set_plain`].

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MarkKind {
    Bold,
    Italic,
    Strike,
    Code,
    Color([u8; 3]),
    Link(String),
}

fn same_variant(a: &MarkKind, b: &MarkKind) -> bool {
    std::mem::discriminant(a) == std::mem::discriminant(b)
}

/// A formatting mark over `[start, end)` measured in `char`s.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mark {
    pub start: usize,
    pub end: usize,
    pub kind: MarkKind,
}

/// A run of text with a uniform set of marks.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub text: String,
    pub marks: Vec<MarkKind>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RichText {
    pub text: String,
    #[serde(default)]
    pub marks: Vec<Mark>,
}

impl RichText {
    pub fn new(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            marks: Vec::new(),
        }
    }

    pub fn char_len(&self) -> usize {
        self.text.chars().count()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Replace the text, shifting / trimming marks so they stay attached to
    /// the characters they covered. Uses a common prefix/suffix diff.
    pub fn set_plain(&mut self, new: &str) {
        if self.text == new {
            return;
        }
        let old: Vec<char> = self.text.chars().collect();
        let newc: Vec<char> = new.chars().collect();
        let max_prefix = old.len().min(newc.len());
        let mut p = 0;
        while p < max_prefix && old[p] == newc[p] {
            p += 1;
        }
        let mut s = 0;
        while s < max_prefix - p && old[old.len() - 1 - s] == newc[newc.len() - 1 - s] {
            s += 1;
        }
        let removed_end = old.len() - s; // old index where the untouched suffix begins
        let ins = newc.len() - p - s;
        let delta = newc.len() as isize - old.len() as isize;
        let map = |i: usize| -> usize {
            if i <= p {
                i
            } else if i >= removed_end {
                (i as isize + delta) as usize
            } else {
                p + ins
            }
        };
        for m in &mut self.marks {
            m.start = map(m.start);
            m.end = map(m.end);
        }
        self.marks.retain(|m| m.start < m.end);
        self.text = new.to_owned();
    }

    /// Split at char index `at`; `self` keeps the head, the tail is returned
    /// (with its marks re-based to 0).
    pub fn split_off(&mut self, at: usize) -> Self {
        let chars: Vec<char> = self.text.chars().collect();
        let at = at.min(chars.len());
        let tail_text: String = chars[at..].iter().collect();
        let head_text: String = chars[..at].iter().collect();
        let mut tail_marks = Vec::new();
        for m in &self.marks {
            if m.end > at {
                tail_marks.push(Mark {
                    start: m.start.max(at) - at,
                    end: m.end - at,
                    kind: m.kind.clone(),
                });
            }
        }
        self.marks.retain(|m| m.start < at);
        for m in &mut self.marks {
            m.end = m.end.min(at);
        }
        self.text = head_text;
        Self {
            text: tail_text,
            marks: tail_marks,
        }
    }

    /// Append another rich text (marks are shifted).
    pub fn push_rich(&mut self, other: &Self) {
        append(self, other.clone(), None);
    }

    /// Toggle `kind` over `[start, end)`: if the whole range already has it,
    /// remove it; otherwise apply it to the whole range.
    pub fn toggle(&mut self, start: usize, end: usize, kind: MarkKind) {
        let n = self.char_len();
        let (s, e) = (start.min(end).min(n), start.max(end).min(n));
        if s == e {
            return;
        }
        let mut cov = vec![false; n];
        for m in self.marks.iter().filter(|m| same_variant(&m.kind, &kind)) {
            for c in cov.iter_mut().take(m.end.min(n)).skip(m.start) {
                *c = true;
            }
        }
        let all = cov[s..e].iter().all(|c| *c);
        self.marks.retain(|m| !same_variant(&m.kind, &kind));
        for c in &mut cov[s..e] {
            *c = !all;
        }
        let mut i = 0;
        while i < n {
            if cov[i] {
                let st = i;
                while i < n && cov[i] {
                    i += 1;
                }
                self.marks.push(Mark {
                    start: st,
                    end: i,
                    kind: kind.clone(),
                });
            } else {
                i += 1;
            }
        }
    }

    /// Split into runs with uniform formatting.
    pub fn segments(&self) -> Vec<Segment> {
        let chars: Vec<char> = self.text.chars().collect();
        let n = chars.len();
        let mut cuts = vec![0, n];
        for m in &self.marks {
            cuts.push(m.start.min(n));
            cuts.push(m.end.min(n));
        }
        cuts.sort_unstable();
        cuts.dedup();
        let mut out = Vec::new();
        for w in cuts.windows(2) {
            let (a, b) = (w[0], w[1]);
            if a == b {
                continue;
            }
            out.push(Segment {
                text: chars[a..b].iter().collect(),
                marks: self
                    .marks
                    .iter()
                    .filter(|m| m.start <= a && m.end >= b)
                    .map(|m| m.kind.clone())
                    .collect(),
            });
        }
        out
    }

    /// Page ids referenced through `page:<uuid>` links (mentions).
    pub fn linked_pages(&self) -> Vec<uuid::Uuid> {
        self.marks
            .iter()
            .filter_map(|m| match &m.kind {
                MarkKind::Link(u) => u.strip_prefix("page:").and_then(|s| s.parse().ok()),
                _ => None,
            })
            .collect()
    }

    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        for seg in self.segments() {
            let mut s = seg.text.clone();
            for m in &seg.marks {
                s = match m {
                    MarkKind::Code => format!("`{s}`"),
                    MarkKind::Bold => format!("**{s}**"),
                    MarkKind::Italic => format!("*{s}*"),
                    MarkKind::Strike => format!("~~{s}~~"),
                    MarkKind::Link(u) => format!("[{s}]({u})"),
                    MarkKind::Color(_) => s,
                };
            }
            out.push_str(&s);
        }
        out
    }

    /// Parse inline markdown (`**b**`, `*i*`, `` `c` ``, `~~s~~`, `[t](url)`).
    pub fn from_markdown(src: &str) -> Self {
        let chars: Vec<char> = src.chars().collect();
        let mut out = Self::default();
        let mut i = 0;
        while i < chars.len() {
            let rest = &chars[i..];
            let starts = |p: &str| p.chars().enumerate().all(|(k, c)| rest.get(k) == Some(&c));
            if starts("**") {
                if let Some(j) = find(&chars, i + 2, "**").filter(|j| *j > i + 2) {
                    append(&mut out, Self::from_markdown(&collect(&chars[i + 2..j])), Some(MarkKind::Bold));
                    i = j + 2;
                    continue;
                }
            } else if starts("~~") {
                if let Some(j) = find(&chars, i + 2, "~~").filter(|j| *j > i + 2) {
                    append(&mut out, Self::from_markdown(&collect(&chars[i + 2..j])), Some(MarkKind::Strike));
                    i = j + 2;
                    continue;
                }
            } else if starts("`") {
                if let Some(j) = find(&chars, i + 1, "`").filter(|j| *j > i + 1) {
                    append(&mut out, Self::new(&collect(&chars[i + 1..j])), Some(MarkKind::Code));
                    i = j + 1;
                    continue;
                }
            } else if starts("*") {
                if let Some(j) = find(&chars, i + 1, "*").filter(|j| *j > i + 1) {
                    append(&mut out, Self::from_markdown(&collect(&chars[i + 1..j])), Some(MarkKind::Italic));
                    i = j + 1;
                    continue;
                }
            } else if starts("[") {
                if let Some(k) = find(&chars, i + 1, "](") {
                    if let Some(m) = find(&chars, k + 2, ")") {
                        let url = collect(&chars[k + 2..m]);
                        append(
                            &mut out,
                            Self::from_markdown(&collect(&chars[i + 1..k])),
                            Some(MarkKind::Link(url)),
                        );
                        i = m + 1;
                        continue;
                    }
                }
            }
            out.text.push(chars[i]);
            i += 1;
        }
        out
    }
}

fn collect(c: &[char]) -> String {
    c.iter().collect()
}

fn find(chars: &[char], from: usize, pat: &str) -> Option<usize> {
    let p: Vec<char> = pat.chars().collect();
    if p.is_empty() || chars.len() < p.len() {
        return None;
    }
    (from..=chars.len() - p.len()).find(|&i| chars[i..i + p.len()] == p[..])
}

fn append(out: &mut RichText, inner: RichText, kind: Option<MarkKind>) {
    let offset = out.char_len();
    let len = inner.char_len();
    out.text.push_str(&inner.text);
    for m in inner.marks {
        out.marks.push(Mark {
            start: m.start + offset,
            end: m.end + offset,
            kind: m.kind,
        });
    }
    if let Some(kind) = kind {
        out.marks.push(Mark {
            start: offset,
            end: offset + len,
            kind,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_follow_edits() {
        let mut t = RichText::new("hello world");
        t.toggle(6, 11, MarkKind::Bold);
        t.set_plain("oh hello world");
        assert_eq!(t.marks[0].start, 9);
        assert_eq!(t.marks[0].end, 14);
        t.set_plain("oh hello");
        assert!(t.marks.is_empty());
    }

    #[test]
    fn toggle_twice_removes() {
        let mut t = RichText::new("abcdef");
        t.toggle(1, 4, MarkKind::Italic);
        assert_eq!(t.marks.len(), 1);
        t.toggle(1, 4, MarkKind::Italic);
        assert!(t.marks.is_empty());
    }

    #[test]
    fn markdown_roundtrip() {
        let t = RichText::from_markdown("a **bold** and `code` and [x](http://e.co)");
        assert_eq!(t.text, "a bold and code and x");
        assert_eq!(t.marks.len(), 3);
        assert_eq!(t.to_markdown(), "a **bold** and `code` and [x](http://e.co)");
    }

    #[test]
    fn typing_inside_bold_extends_it() {
        let mut t = RichText::new("abc");
        t.toggle(0, 3, MarkKind::Bold);
        t.set_plain("abXc");
        assert_eq!(t.marks[0].end, 4);
    }
}
