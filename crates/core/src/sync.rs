//! Conflict-free page merging (state-based, last-writer-wins per block).
//!
//! `merge_pages` is commutative, associative (for the block set) and
//! idempotent, so replicas converge regardless of the order in which they
//! exchange state. Top-level blocks are the unit of merging. Block *order*
//! follows the replica with the newer page timestamp; blocks that only exist
//! on the other side are inserted after their predecessor. For character-level
//! merging of the same paragraph, swap this module for `yrs` (ADR-0001).

use crate::page::Page;
use std::collections::HashMap;
use uuid::Uuid;

pub fn merge_pages(local: &Page, remote: &Page) -> Page {
    debug_assert_eq!(local.id, remote.id);
    let (base, other) = if remote.updated > local.updated { (remote, local) } else { (local, remote) };
    let mut out = base.clone();

    // tombstones: union, keep newest
    let mut tombs: HashMap<Uuid, u64> = base.tombstones.clone();
    for (id, ts) in &other.tombstones {
        let e = tombs.entry(*id).or_insert(0);
        *e = (*e).max(*ts);
    }

    for (idx, ob) in other.blocks.iter().enumerate() {
        match out.blocks.iter().position(|b| b.id == ob.id) {
            Some(pos) => {
                if ob.ts > out.blocks[pos].ts {
                    out.blocks[pos] = ob.clone();
                }
            }
            None => {
                let dead = tombs.get(&ob.id).is_some_and(|t| *t >= ob.ts);
                if dead {
                    continue;
                }
                let insert_at = idx
                    .checked_sub(1)
                    .and_then(|p| out.blocks.iter().position(|b| b.id == other.blocks[p].id))
                    .map_or(0, |p| p + 1);
                out.blocks.insert(insert_at.min(out.blocks.len()), ob.clone());
            }
        }
    }
    out.blocks.retain(|b| !tombs.get(&b.id).is_some_and(|t| *t >= b.ts));
    out.tombstones = tombs;
    out.updated = base.updated.max(other.updated);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{Block, BlockKind};

    fn ids(p: &Page) -> Vec<Uuid> {
        let mut v: Vec<Uuid> = p.blocks.iter().map(|b| b.id).collect();
        v.sort();
        v
    }

    #[test]
    fn concurrent_inserts_converge() {
        let mut a = Page::new("p", None);
        a.blocks.clear();
        a.updated = 10;
        let mut b = a.clone();
        a.blocks.push(Block::with_text(BlockKind::Paragraph, "from a"));
        a.updated = 20;
        b.blocks.push(Block::with_text(BlockKind::Paragraph, "from b"));
        b.updated = 30;
        let m1 = merge_pages(&a, &b);
        let m2 = merge_pages(&b, &a);
        assert_eq!(ids(&m1), ids(&m2));
        assert_eq!(m1.blocks.len(), 2);
        let again = merge_pages(&m1, &a);
        assert_eq!(ids(&again), ids(&m1));
    }

    #[test]
    fn newer_block_wins_and_delete_sticks() {
        let mut a = Page::new("p", None);
        a.blocks = vec![Block::with_text(BlockKind::Paragraph, "x")];
        a.blocks[0].ts = 100;
        let mut b = a.clone();
        b.blocks[0].text.set_plain("y");
        b.blocks[0].ts = 200;
        b.updated = 200;
        let m = merge_pages(&a, &b);
        assert_eq!(m.blocks[0].text.text, "y");

        let mut c = a.clone();
        let dead = c.blocks.remove(0);
        c.tombstones.insert(dead.id, 300);
        c.updated = 300;
        let m = merge_pages(&a, &c);
        assert!(m.blocks.is_empty());
        let m = merge_pages(&c, &a);
        assert!(m.blocks.is_empty());
    }
}
