use crate::model::RepeatMode;
use rand::seq::SliceRandom;

/// A play queue over *library indices*.
///
/// `base` is the canonical order the queue was built in; `order` is the order
/// actually played (equal to `base` unless shuffle is on). Toggling shuffle off
/// restores the original order while keeping the current track selected.
#[derive(Default)]
pub struct Queue {
    base: Vec<usize>,
    order: Vec<usize>,
    current: Option<usize>, // position within `order`
    shuffle: bool,
}

impl Queue {
    /// Replace the queue contents. `start_at` is an index into `tracks`.
    pub fn set_tracks(&mut self, tracks: Vec<usize>, start_at: usize) {
        self.base = tracks;
        self.rebuild_order();
        self.current = if self.order.is_empty() {
            None
        } else {
            // Find where the requested start track landed in `order`.
            let lib = self.base.get(start_at).copied();
            lib.and_then(|l| self.order.iter().position(|&x| x == l))
                .or(Some(0))
        };
    }

    /// Append a single library index to the end of the queue.
    pub fn push(&mut self, lib_index: usize) {
        self.base.push(lib_index);
        self.order.push(lib_index);
        if self.current.is_none() {
            self.current = Some(self.order.len() - 1);
        }
    }

    fn rebuild_order(&mut self) {
        self.order = self.base.clone();
        if self.shuffle {
            let mut rng = rand::rng();
            self.order.shuffle(&mut rng);
        }
    }

    /// Current library index being played.
    pub fn current_track(&self) -> Option<usize> {
        self.current.and_then(|c| self.order.get(c).copied())
    }

    /// Position of the current track within the (visible) `order`.
    pub fn current_pos(&self) -> Option<usize> {
        self.current
    }

    /// The play order as library indices (what the queue view shows).
    pub fn order(&self) -> &[usize] {
        &self.order
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Advance to the next track per the repeat mode. Returns the new library index.
    pub fn next(&mut self, repeat: RepeatMode) -> Option<usize> {
        if self.order.is_empty() {
            return None;
        }
        let cur = self.current?;
        match repeat {
            RepeatMode::One => Some(self.order[cur]),
            _ => {
                if cur + 1 < self.order.len() {
                    self.current = Some(cur + 1);
                } else if repeat == RepeatMode::All {
                    self.current = Some(0);
                } else {
                    return None; // end of queue, stop
                }
                self.current_track()
            }
        }
    }

    /// Move to the previous track. Returns the new library index.
    pub fn prev(&mut self, repeat: RepeatMode) -> Option<usize> {
        if self.order.is_empty() {
            return None;
        }
        let cur = self.current?;
        if cur > 0 {
            self.current = Some(cur - 1);
        } else if repeat == RepeatMode::All {
            self.current = Some(self.order.len() - 1);
        }
        self.current_track()
    }

    /// Jump to an explicit position in the queue view.
    pub fn jump_to(&mut self, pos: usize) -> Option<usize> {
        if pos < self.order.len() {
            self.current = Some(pos);
            self.current_track()
        } else {
            None
        }
    }

    /// Remove a position from the queue, fixing up the current pointer.
    pub fn remove(&mut self, pos: usize) {
        if pos >= self.order.len() {
            return;
        }
        let removed_lib = self.order.remove(pos);
        // keep base in sync (remove first matching occurrence)
        if let Some(bp) = self.base.iter().position(|&x| x == removed_lib) {
            self.base.remove(bp);
        }
        match self.current {
            Some(c) if c == pos => {
                if self.order.is_empty() {
                    self.current = None;
                } else {
                    self.current = Some(c.min(self.order.len() - 1));
                }
            }
            Some(c) if c > pos => self.current = Some(c - 1),
            _ => {}
        }
    }

    /// Drop every occurrence of a library index from the queue and shift any larger
    /// stored indices down by one. Use this when a track is deleted from the library
    /// (whose backing `Vec` indices then shift), so the queue keeps pointing at the
    /// right tracks. The current pointer follows the playing track, or clamps to a
    /// nearby position if the playing track itself was removed.
    pub fn remove_library_index(&mut self, lib_index: usize) {
        // Remember the library index of the track playing right now (by value).
        let current_lib = self.current_track();

        let fixup = |v: &mut Vec<usize>| {
            v.retain(|&l| l != lib_index);
            for l in v.iter_mut() {
                if *l > lib_index {
                    *l -= 1;
                }
            }
        };
        fixup(&mut self.base);
        fixup(&mut self.order);

        self.current = match current_lib {
            // The playing track survived: follow it to its (possibly shifted) value.
            Some(c) if c != lib_index => {
                let adj = if c > lib_index { c - 1 } else { c };
                self.order.iter().position(|&x| x == adj)
            }
            // The playing track was the one removed (or nothing was playing).
            _ => {
                if self.order.is_empty() {
                    None
                } else {
                    self.current.map(|c| c.min(self.order.len() - 1))
                }
            }
        };
    }

    pub fn set_shuffle(&mut self, on: bool) {
        if self.shuffle == on {
            return;
        }
        self.shuffle = on;
        // Preserve the currently playing track across the reshuffle/restore.
        let playing = self.current_track();
        self.rebuild_order();
        self.current = playing.and_then(|l| self.order.iter().position(|&x| x == l));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RepeatMode::*;

    #[test]
    fn next_stops_at_end_when_off() {
        let mut q = Queue::default();
        q.set_tracks(vec![10, 20, 30], 0);
        assert_eq!(q.current_track(), Some(10));
        assert_eq!(q.next(Off), Some(20));
        assert_eq!(q.next(Off), Some(30));
        assert_eq!(q.next(Off), None); // stop at end
    }

    #[test]
    fn next_wraps_when_all() {
        let mut q = Queue::default();
        q.set_tracks(vec![1, 2], 1);
        assert_eq!(q.current_track(), Some(2));
        assert_eq!(q.next(All), Some(1)); // wrap
    }

    #[test]
    fn repeat_one_replays_same() {
        let mut q = Queue::default();
        q.set_tracks(vec![5, 6, 7], 1);
        assert_eq!(q.next(One), Some(6));
        assert_eq!(q.next(One), Some(6));
    }

    #[test]
    fn shuffle_keeps_current_and_restore_preserves_order() {
        let mut q = Queue::default();
        q.set_tracks(vec![0, 1, 2, 3, 4], 2);
        assert_eq!(q.current_track(), Some(2));
        q.set_shuffle(true);
        assert_eq!(q.current_track(), Some(2)); // still pointing at same track
        assert_eq!(q.order().len(), 5);
        q.set_shuffle(false);
        assert_eq!(q.order(), &[0, 1, 2, 3, 4]); // original order restored
        assert_eq!(q.current_track(), Some(2));
    }

    #[test]
    fn remove_fixes_current_pointer() {
        let mut q = Queue::default();
        q.set_tracks(vec![0, 1, 2, 3], 2);
        q.remove(0); // remove before current
        assert_eq!(q.current_track(), Some(2));
        assert_eq!(q.order(), &[1, 2, 3]);
    }

    #[test]
    fn remove_library_index_shifts_and_follows_current() {
        let mut q = Queue::default();
        q.set_tracks(vec![0, 1, 2, 3, 4], 3); // playing library track 3
        assert_eq!(q.current_track(), Some(3));
        q.remove_library_index(1); // delete library track 1
        // track 1 dropped; every index > 1 shifts down one.
        assert_eq!(q.order(), &[0, 1, 2, 3]); // were [0, 2, 3, 4]
        assert_eq!(q.current_track(), Some(2)); // old track 3 is now index 2
    }

    #[test]
    fn remove_library_index_when_current_deleted() {
        let mut q = Queue::default();
        q.set_tracks(vec![5, 6, 7], 1); // playing library track 6
        q.remove_library_index(6); // delete the playing track
        assert_eq!(q.order(), &[5, 6]); // 7 shifted down to 6
        assert_eq!(q.current_track(), Some(6)); // moved to the track now in that slot
    }

    #[test]
    fn remove_library_index_drops_all_duplicates() {
        let mut q = Queue::default();
        q.set_tracks(vec![2, 5, 2, 8], 0);
        q.remove_library_index(2); // delete library track 2 (appears twice)
        assert_eq!(q.order(), &[4, 7]); // 5->4, 8->7, both 2s removed
    }
}
