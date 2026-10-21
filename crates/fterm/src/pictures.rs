//! The pictures of one pane (from its output, OSC 1337): kept by the id of their mark, and dropped when the mark is
//! gone from the grid or when they take too much memory.

use std::collections::{BTreeMap, HashSet};

use fterm_term::osc::PictureMark;

/// The most memory for the pictures of one pane. The oldest go first.
pub const MAX_BYTES: usize = 256 * 1024 * 1024;

#[derive(Default)]
pub struct PictureStore {
    /// By id; the ids grow, so the first one is the oldest.
    items: BTreeMap<u32, PictureMark>,
    bytes: usize,
}

impl PictureStore {
    pub fn insert(&mut self, mark: PictureMark) {
        self.bytes += mark.picture.rgba.len();
        if let Some(old) = self.items.insert(mark.id, mark) {
            self.bytes -= old.picture.rgba.len();
        }
        while self.bytes > MAX_BYTES && self.items.len() > 1 {
            if let Some((_, old)) = self.items.pop_first() {
                self.bytes -= old.picture.rgba.len();
            }
        }
    }

    pub fn get(&self, id: u32) -> Option<&PictureMark> {
        self.items.get(&id)
    }

    /// Keeps only the pictures whose marks are still in the grid.
    pub fn retain(&mut self, alive: &HashSet<u32>) {
        let bytes = &mut self.bytes;
        self.items.retain(|id, mark| {
            let keep = alive.contains(id);
            if !keep {
                *bytes -= mark.picture.rgba.len();
            }
            keep
        });
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The rows of the highest picture (to find a picture whose top is scrolled away).
    pub fn max_rows(&self) -> usize {
        self.items.values().map(|m| m.rows).max().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fterm_term::picture::Picture;

    use super::*;

    fn mark(id: u32, bytes: usize, rows: usize) -> PictureMark {
        PictureMark {
            id,
            picture: Picture {
                width: 1,
                height: 1,
                rgba: Arc::new(vec![0; bytes]),
            },
            columns: 1,
            rows,
        }
    }

    #[test]
    fn pictures_are_kept_by_id() {
        let mut store = PictureStore::default();
        assert!(store.is_empty());
        store.insert(mark(1, 100, 3));
        store.insert(mark(2, 50, 7));
        assert_eq!(store.get(1).unwrap().rows, 3);
        assert_eq!(store.max_rows(), 7);
        assert_eq!(store.bytes, 150);
        // The same id again replaces it.
        store.insert(mark(1, 10, 1));
        assert_eq!(store.bytes, 60);
        assert_eq!(store.items.len(), 2);
    }

    #[test]
    fn a_picture_without_its_mark_goes() {
        let mut store = PictureStore::default();
        store.insert(mark(1, 100, 3));
        store.insert(mark(2, 50, 7));
        store.retain(&HashSet::from([2]));
        assert!(store.get(1).is_none());
        assert_eq!(store.bytes, 50);
        assert_eq!(store.max_rows(), 7);
    }

    #[test]
    fn the_oldest_pictures_go_when_there_is_too_much() {
        let mut store = PictureStore::default();
        let big = MAX_BYTES / 3 + 1;
        for id in 1..=3 {
            store.insert(mark(id, big, 1));
        }
        assert!(store.get(1).is_none(), "the oldest went");
        assert!(store.get(2).is_some() && store.get(3).is_some());
        assert!(store.bytes <= MAX_BYTES);
        // One picture bigger than the limit is still kept (there is nothing older to drop).
        let mut store = PictureStore::default();
        store.insert(mark(9, MAX_BYTES + 1, 1));
        assert_eq!(store.items.len(), 1);
    }
}
