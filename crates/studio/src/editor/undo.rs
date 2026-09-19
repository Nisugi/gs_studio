//! Snapshot undo/redo. Rigs are tiny, so whole-value snapshots are fine.

const LIMIT: usize = 200;

#[derive(Debug, Clone)]
pub struct UndoStack<T> {
    undo: Vec<T>,
    redo: Vec<T>,
    /// Tag of the newest undo entry; a record with the same tag coalesces
    /// into it (one undo step per slider drag, not one per frame).
    last_tag: Option<u64>,
}

impl<T> Default for UndoStack<T> {
    fn default() -> Self {
        UndoStack { undo: Vec::new(), redo: Vec::new(), last_tag: None }
    }
}

impl<T> UndoStack<T> {
    /// Record the value as it was BEFORE an edit. `tag` = Some(id) marks a
    /// continuous edit; consecutive records with the same id keep only the
    /// first snapshot. Any new record clears redo.
    pub fn record(&mut self, before: T, tag: Option<u64>) {
        self.redo.clear();
        if tag.is_some() && tag == self.last_tag {
            return;
        }
        self.undo.push(before);
        if self.undo.len() > LIMIT {
            self.undo.remove(0);
        }
        self.last_tag = tag;
    }

    /// End a continuous edit: the next record starts a new undo step even if
    /// it carries the same tag.
    pub fn seal(&mut self) {
        self.last_tag = None;
    }

    /// Swap `current` for the previous snapshot. False if there is none.
    pub fn undo(&mut self, current: &mut T) -> bool {
        self.last_tag = None;
        match self.undo.pop() {
            Some(prev) => {
                self.redo.push(std::mem::replace(current, prev));
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self, current: &mut T) -> bool {
        self.last_tag = None;
        match self.redo.pop() {
            Some(next) => {
                self.undo.push(std::mem::replace(current, next));
                true
            }
            None => false,
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_then_redo_restores_both_ways() {
        let mut s = UndoStack::default();
        let mut v = 1;
        s.record(v, None);
        v = 2;
        s.record(v, None);
        v = 3;
        assert!(s.undo(&mut v) && v == 2);
        assert!(s.undo(&mut v) && v == 1);
        assert!(!s.undo(&mut v) && v == 1);
        assert!(s.redo(&mut v) && v == 2);
        assert!(s.redo(&mut v) && v == 3);
        assert!(!s.redo(&mut v));
    }

    #[test]
    fn a_new_edit_drops_redo() {
        let mut s = UndoStack::default();
        let mut v = 1;
        s.record(v, None);
        v = 2;
        s.undo(&mut v);
        s.record(v, None);
        v = 9;
        assert!(!s.can_redo());
        assert!(s.undo(&mut v) && v == 1);
    }

    #[test]
    fn tagged_records_coalesce_until_sealed() {
        let mut s = UndoStack::default();
        let mut v = 0;
        for next in 1..=5 {
            s.record(v, Some(7));
            v = next;
        }
        s.seal();
        s.record(v, Some(7));
        v = 6;
        assert!(s.undo(&mut v) && v == 5);
        assert!(s.undo(&mut v) && v == 0);
        assert!(!s.can_undo());
    }

    #[test]
    fn history_is_bounded() {
        let mut s = UndoStack::default();
        for i in 0..LIMIT + 50 {
            s.record(i, None);
        }
        assert_eq!(s.undo.len(), LIMIT);
        assert_eq!(s.undo[0], 50);
    }
}
