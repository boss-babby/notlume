//! The block tree.

use crate::richtext::RichText;
use crate::time::now_ms;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockKind {
    Paragraph,
    Heading(u8),
    Bulleted,
    Numbered,
    Todo,
    Toggle,
    Quote,
    Callout,
    Code,
    Divider,
    Image,
    Table,
}

impl BlockKind {
    pub fn is_list(self) -> bool {
        matches!(self, Self::Bulleted | Self::Numbered | Self::Todo)
    }
    pub fn has_text(self) -> bool {
        !matches!(self, Self::Divider | Self::Image | Self::Table)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub id: Uuid,
    pub kind: BlockKind,
    #[serde(default)]
    pub text: RichText,
    #[serde(default)]
    pub checked: bool,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub emoji: String,
    #[serde(default)]
    pub table: Vec<Vec<String>>,
    #[serde(default)]
    pub children: Vec<Block>,
    /// Last-modified time (ms), used for last-writer-wins sync.
    #[serde(default)]
    pub ts: u64,
}

impl Block {
    pub fn new(kind: BlockKind) -> Self {
        let table = if kind == BlockKind::Table {
            vec![vec![String::new(); 3]; 3]
        } else {
            Vec::new()
        };
        Self {
            id: Uuid::new_v4(),
            kind,
            text: RichText::default(),
            checked: false,
            language: if kind == BlockKind::Code { "rust".into() } else { String::new() },
            url: String::new(),
            emoji: if kind == BlockKind::Callout { "💡".into() } else { String::new() },
            table,
            children: Vec::new(),
            ts: now_ms(),
        }
    }

    pub fn with_text(kind: BlockKind, text: &str) -> Self {
        let mut b = Self::new(kind);
        b.text = RichText::new(text);
        b
    }

    pub fn touch(&mut self) {
        self.ts = now_ms();
    }
}

pub fn find(blocks: &[Block], id: Uuid) -> Option<&Block> {
    for b in blocks {
        if b.id == id {
            return Some(b);
        }
        if let Some(f) = find(&b.children, id) {
            return Some(f);
        }
    }
    None
}

pub fn find_mut(blocks: &mut [Block], id: Uuid) -> Option<&mut Block> {
    for b in blocks {
        if b.id == id {
            return Some(b);
        }
        if let Some(f) = find_mut(&mut b.children, id) {
            return Some(f);
        }
    }
    None
}

/// Total number of blocks including nested children.
pub fn count(blocks: &[Block]) -> usize {
    blocks.iter().map(|b| 1 + count(&b.children)).sum()
}

/// Plain text of the whole tree (used for search and PDF export).
pub fn plain_text(blocks: &[Block], out: &mut String) {
    for b in blocks {
        out.push_str(&b.text.text);
        out.push('\n');
        for row in &b.table {
            out.push_str(&row.join(" "));
            out.push('\n');
        }
        plain_text(&b.children, out);
    }
}
