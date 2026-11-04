//! Undo / redo with burst coalescing.
//!
//! A "burst" is a run of edits with no pause longer than `gap_ms`; it is a
//! single undo step. Cloning only happens at burst boundaries, so typing in a
//! very large document does not clone it per keystroke.

#[derive(Clone, Debug)]
pub struct History<T: Clone> {
    undo: Vec<T>,
    redo: Vec<T>,
    baseline: T,
    burst: bool,
    last_change: u64,
    gap_ms: u64,
    cap: usize,
}

impl<T: Clone> History<T> {
    pub fn new(initial: T) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            baseline: initial,
            burst: false,
            last_change: 0,
            gap_ms: 700,
            cap: 200,
        }
    }

    pub fn reset(&mut self, initial: T) {
        *self = Self::new(initial);
    }

    /// Call after the document changed.
    pub fn mark_changed(&mut self, now_ms: u64) {
        if !self.burst {
            self.undo.push(self.baseline.clone());
            if self.undo.len() > self.cap {
                self.undo.remove(0);
            }
            self.redo.clear();
            self.burst = true;
        }
        self.last_change = now_ms;
    }

    /// Call every frame; closes the burst after a pause.
    pub fn settle(&mut self, current: &T, now_ms: u64) {
        if self.burst && now_ms.saturating_sub(self.last_change) >= self.gap_ms {
            self.baseline = current.clone();
            self.burst = false;
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self, current: &T) -> Option<T> {
        let prev = self.undo.pop()?;
        self.redo.push(current.clone());
        self.baseline = prev.clone();
        self.burst = false;
        Some(prev)
    }

    pub fn redo(&mut self, current: &T) -> Option<T> {
        let next = self.redo.pop()?;
        self.undo.push(current.clone());
        self.baseline = next.clone();
        self.burst = false;
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_redo_cycle() {
        let mut h = History::new(0);
        // burst 1: 0 -> 1
        h.mark_changed(0);
        h.settle(&1, 1000);
        // burst 2: 1 -> 2
        h.mark_changed(1100);
        assert_eq!(h.undo(&2), Some(1));
        assert_eq!(h.undo(&1), Some(0));
        assert_eq!(h.undo(&0), None);
        assert_eq!(h.redo(&0), Some(1));
        assert_eq!(h.redo(&1), Some(2));
    }

    #[test]
    fn typing_burst_is_one_step() {
        let mut h = History::new(String::new());
        for (i, t) in ["a", "ab", "abc"].iter().enumerate() {
            h.mark_changed(i as u64 * 100);
            h.settle(&t.to_string(), i as u64 * 100);
        }
        assert_eq!(h.undo(&"abc".to_string()), Some(String::new()));
        assert!(!h.can_undo());
    }
}
