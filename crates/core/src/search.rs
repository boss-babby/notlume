//! Fuzzy matching for the command palette and a small inverted index for
//! full-text search. The index is intentionally lightweight so that it works
//! identically on native and WebAssembly (see ADR-0001 on why not tantivy).

use crate::block::plain_text;
use crate::page::Page;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Subsequence fuzzy score. `None` when `query` is not a subsequence.
/// Rewards consecutive runs, word starts and early matches.
pub fn fuzzy_score(query: &str, candidate: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let cand: Vec<char> = candidate.chars().flat_map(char::to_lowercase).collect();
    let mut score = 0;
    let mut ci = 0;
    let mut prev_match: Option<usize> = None;
    for qc in query.chars().flat_map(char::to_lowercase) {
        let found = (ci..cand.len()).find(|&i| cand[i] == qc)?;
        score += 10;
        if prev_match == Some(found.wrapping_sub(1)) {
            score += 15;
        }
        if found == 0 || !cand[found - 1].is_alphanumeric() {
            score += 12;
        }
        score -= (found as i32 - ci as i32).min(8);
        prev_match = Some(found);
        ci = found + 1;
    }
    Some(score - (cand.len() as i32 / 8))
}

#[derive(Default)]
pub struct SearchIndex {
    postings: HashMap<String, HashSet<Uuid>>,
    docs: HashMap<Uuid, Vec<String>>,
    titles: HashMap<Uuid, String>,
}

fn tokenize(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect()
}

impl SearchIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn remove(&mut self, id: Uuid) {
        if let Some(tokens) = self.docs.remove(&id) {
            for t in tokens {
                if let Some(set) = self.postings.get_mut(&t) {
                    set.remove(&id);
                }
            }
        }
        self.titles.remove(&id);
    }

    pub fn index_page(&mut self, page: &Page) {
        self.remove(page.id);
        let mut body = String::new();
        plain_text(&page.blocks, &mut body);
        let mut tokens = tokenize(&page.title);
        tokens.extend(tokenize(&body));
        tokens.sort_unstable();
        tokens.dedup();
        for t in &tokens {
            self.postings.entry(t.clone()).or_default().insert(page.id);
        }
        self.docs.insert(page.id, tokens);
        self.titles.insert(page.id, page.title.clone());
    }

    /// Every query token must prefix-match some token in the page.
    /// Title matches are boosted.
    pub fn query(&self, q: &str) -> Vec<(Uuid, i32)> {
        let qt = tokenize(q);
        if qt.is_empty() {
            return Vec::new();
        }
        let mut result: Option<HashSet<Uuid>> = None;
        for t in &qt {
            let mut hit = HashSet::new();
            for (tok, ids) in &self.postings {
                if tok.starts_with(t.as_str()) {
                    hit.extend(ids.iter().copied());
                }
            }
            result = Some(match result {
                None => hit,
                Some(prev) => prev.intersection(&hit).copied().collect(),
            });
        }
        let mut out: Vec<(Uuid, i32)> = result
            .unwrap_or_default()
            .into_iter()
            .map(|id| {
                let title_score = self
                    .titles
                    .get(&id)
                    .and_then(|t| fuzzy_score(q, t))
                    .map(|s| s + 50)
                    .unwrap_or(0);
                (id, title_score)
            })
            .collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{Block, BlockKind};

    #[test]
    fn fuzzy() {
        assert!(fuzzy_score("gst", "Getting Started").is_some());
        assert!(fuzzy_score("xyz", "Getting Started").is_none());
        assert!(fuzzy_score("get", "Getting Started") > fuzzy_score("gtd", "Getting Started"));
    }

    #[test]
    fn index_roundtrip() {
        let mut idx = SearchIndex::new();
        let mut p = Page::new("Roadmap", None);
        p.blocks.push(Block::with_text(BlockKind::Paragraph, "quarterly planning notes"));
        idx.index_page(&p);
        assert_eq!(idx.query("plan notes").len(), 1);
        assert_eq!(idx.query("zzz").len(), 0);
        idx.remove(p.id);
        assert!(idx.query("roadmap").is_empty());
    }
}
