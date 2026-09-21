//! In-memory history store (UI thread). Newest first.

use crate::model::hash::Hash;
use crate::model::item::{ClipItem, ItemId, Sensitivity};
use crate::settings::Dedupe;
use std::collections::VecDeque;
use std::time::Instant;

pub enum PushResult {
    Inserted(ItemId),
    /// An existing item matched by hash; it was moved to the front / touched.
    Bumped(ItemId),
}

pub struct HistoryStore {
    items: VecDeque<ClipItem>,
    next_id: ItemId,
    pub active: Option<ItemId>,
    pub pinned_first: bool,
    pub dedupe: Dedupe,
}

impl HistoryStore {
    pub fn new(pinned_first: bool, dedupe: Dedupe) -> Self {
        HistoryStore { items: VecDeque::new(), next_id: 1, active: None, pinned_first, dedupe }
    }

    pub fn alloc_id(&mut self) -> ItemId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Bulk load from DB (rows already sorted newest-first).
    pub fn load(&mut self, items: Vec<ClipItem>) {
        for it in items {
            self.next_id = self.next_id.max(it.id + 1);
            self.items.push_back(it);
        }
    }

    pub fn find_by_hash(&self, hash: &Hash) -> Option<ItemId> {
        self.items.iter().find(|i| &i.hash == hash).map(|i| i.id)
    }

    /// Insert a new item or bump an existing duplicate to the front.
    pub fn push(&mut self, item: ClipItem) -> PushResult {
        match self.dedupe {
            Dedupe::Consecutive => {
                if let Some(front) = self.items.front_mut() {
                    if front.hash == item.hash {
                        front.created_at_ms = item.created_at_ms;
                        return PushResult::Bumped(front.id);
                    }
                }
            }
            Dedupe::Anywhere => {
                if let Some(pos) = self.items.iter().position(|i| i.hash == item.hash) {
                    let mut existing = self.items.remove(pos).unwrap();
                    existing.created_at_ms = item.created_at_ms;
                    if existing.sensitivity == Sensitivity::None {
                        existing.sensitivity = item.sensitivity;
                    }
                    let id = existing.id;
                    self.items.push_front(existing);
                    return PushResult::Bumped(id);
                }
            }
        }
        let id = item.id;
        self.next_id = self.next_id.max(id + 1);
        self.items.push_front(item);
        PushResult::Inserted(id)
    }

    pub fn get(&self, id: ItemId) -> Option<&ClipItem> {
        self.items.iter().find(|i| i.id == id)
    }

    pub fn get_mut(&mut self, id: ItemId) -> Option<&mut ClipItem> {
        self.items.iter_mut().find(|i| i.id == id)
    }

    pub fn remove(&mut self, id: ItemId) -> Option<ClipItem> {
        let pos = self.items.iter().position(|i| i.id == id)?;
        if self.active == Some(id) {
            self.active = None;
        }
        self.items.remove(pos)
    }

    pub fn clear(&mut self, keep_pinned: bool) -> Vec<ClipItem> {
        let mut removed = Vec::new();
        let mut kept = VecDeque::new();
        for it in self.items.drain(..) {
            if keep_pinned && it.pinned {
                kept.push_back(it);
            } else {
                removed.push(it);
            }
        }
        self.items = kept;
        if let Some(a) = self.active {
            if self.get(a).is_none() {
                self.active = None;
            }
        }
        removed
    }

    /// Items in display order: pinned first (if configured), then newest first.
    pub fn visible(&self) -> Vec<&ClipItem> {
        let mut v: Vec<&ClipItem> = self.items.iter().collect();
        if self.pinned_first {
            v.sort_by_key(|i| !i.pinned); // stable: keeps recency within groups
        }
        v
    }

    /// Item at hotkey slot `n` (1..=9, 0 = tenth).
    pub fn slot(&self, n: u32) -> Option<&ClipItem> {
        let idx = if n == 0 { 9 } else { (n - 1) as usize };
        self.visible().get(idx).copied()
    }

    pub fn set_active_by_hash(&mut self, hash: &Hash) {
        self.active = self.find_by_hash(hash);
    }

    /// Drop the oldest unpinned items beyond `max`. Returns removed items.
    pub fn trim(&mut self, max: usize) -> Vec<ClipItem> {
        let mut removed = Vec::new();
        while self.items.len() > max {
            let pos = self.items.iter().rposition(|i| !i.pinned);
            match pos {
                Some(p) => {
                    if let Some(it) = self.items.remove(p) {
                        if self.active == Some(it.id) {
                            self.active = None;
                        }
                        removed.push(it);
                    }
                }
                None => break,
            }
        }
        removed
    }

    /// Remove sensitive items whose TTL passed. Returns them.
    pub fn expire_sensitive(&mut self, now: Instant) -> Vec<ClipItem> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < self.items.len() {
            if self.items[i].expires_at.is_some_and(|t| t <= now) {
                if let Some(it) = self.items.remove(i) {
                    if self.active == Some(it.id) {
                        self.active = None;
                    }
                    out.push(it);
                }
            } else {
                i += 1;
            }
        }
        out
    }

    pub fn next_expiry(&self) -> Option<Instant> {
        self.items.iter().filter_map(|i| i.expires_at).min()
    }

    pub fn next_reveal_end(&self) -> Option<Instant> {
        self.items.iter().filter_map(|i| i.revealed_until).min()
    }

    pub fn iter(&self) -> impl Iterator<Item = &ClipItem> {
        self.items.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::hash::hash_text;
    use crate::model::item::ClipContent;

    fn mk(id: ItemId, text: &str) -> ClipItem {
        ClipItem {
            id,
            content: ClipContent::text(text.into()),
            created_at_ms: id,
            hash: hash_text(text),
            source_exe: None,
            sensitivity: Sensitivity::None,
            pinned: false,
            revealed_until: None,
            expires_at: None,
            layout_gen: 0,
        }
    }

    #[test]
    fn dedupe_anywhere_moves_to_front() {
        let mut h = HistoryStore::new(false, Dedupe::Anywhere);
        h.push(mk(1, "a"));
        h.push(mk(2, "b"));
        match h.push(mk(3, "a")) {
            PushResult::Bumped(1) => {}
            _ => panic!(),
        }
        assert_eq!(h.visible()[0].id, 1);
        assert_eq!(h.len(), 2);
    }

    #[test]
    fn dedupe_consecutive_only_front() {
        let mut h = HistoryStore::new(false, Dedupe::Consecutive);
        h.push(mk(1, "a"));
        h.push(mk(2, "b"));
        assert!(matches!(h.push(mk(3, "a")), PushResult::Inserted(3)));
        assert!(matches!(h.push(mk(4, "a")), PushResult::Bumped(3)));
    }

    #[test]
    fn pinned_first_and_slots() {
        let mut h = HistoryStore::new(true, Dedupe::Anywhere);
        for i in 1..=12 {
            h.push(mk(i, &format!("t{i}")));
        }
        h.get_mut(3).unwrap().pinned = true;
        let v = h.visible();
        assert_eq!(v[0].id, 3);
        assert_eq!(v[1].id, 12);
        assert_eq!(h.slot(1).unwrap().id, 3);
        assert_eq!(h.slot(2).unwrap().id, 12);
        assert_eq!(h.slot(0).unwrap().id, 4); // tenth visible
    }

    #[test]
    fn trim_keeps_pinned() {
        let mut h = HistoryStore::new(true, Dedupe::Anywhere);
        for i in 1..=5 {
            h.push(mk(i, &format!("t{i}")));
        }
        h.get_mut(1).unwrap().pinned = true;
        let removed = h.trim(2);
        assert_eq!(removed.len(), 3);
        assert!(h.get(1).is_some());
        assert!(h.get(5).is_some());
    }

    #[test]
    fn expiry() {
        let mut h = HistoryStore::new(false, Dedupe::Anywhere);
        let mut it = mk(1, "secret");
        it.expires_at = Some(Instant::now());
        h.push(it);
        h.push(mk(2, "ok"));
        let gone = h.expire_sensitive(Instant::now() + std::time::Duration::from_millis(1));
        assert_eq!(gone.len(), 1);
        assert_eq!(h.len(), 1);
    }
}
