//! Shelves: named, colored collections of items that can be dragged out or pasted as one.

use crate::model::item::{ClipContent, ClipItem, Sensitivity};

pub const PALETTE: [u32; 6] = [0x4cc2ff, 0xff8c69, 0x7ee787, 0xf5c542, 0xc678dd, 0xff6b9d];

#[derive(Clone, Debug)]
pub struct ShelfEntry {
    pub id: i64,
    pub content: ClipContent,
    pub created_at_ms: i64,
}

impl ShelfEntry {
    /// View the entry as a ClipItem for drawing; shelf ids are negated so they never collide with history ids.
    pub fn as_item(&self) -> ClipItem {
        ClipItem {
            id: -self.id,
            content: self.content.clone(),
            created_at_ms: self.created_at_ms,
            hash: [0u8; 32],
            source_exe: None,
            sensitivity: Sensitivity::None,
            pinned: false,
            revealed_until: None,
            expires_at: None,
            layout_gen: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Shelf {
    pub id: i64,
    pub name: String,
    pub color: u32,
    pub entries: Vec<ShelfEntry>,
}

pub struct ShelfStore {
    pub shelves: Vec<Shelf>,
    /// Shelf that receives drops on the bar and is shown first.
    pub active: i64,
    next_shelf: i64,
    next_entry: i64,
}

impl Default for ShelfStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ShelfStore {
    pub fn new() -> ShelfStore {
        ShelfStore { shelves: Vec::new(), active: 0, next_shelf: 1, next_entry: 1 }
    }

    pub fn load(&mut self, shelves: Vec<Shelf>) {
        for s in &shelves {
            self.next_shelf = self.next_shelf.max(s.id + 1);
            for e in &s.entries {
                self.next_entry = self.next_entry.max(e.id + 1);
            }
        }
        self.shelves = shelves;
        if self.get(self.active).is_none() {
            self.active = self.shelves.first().map(|s| s.id).unwrap_or(0);
        }
    }

    /// Create the default shelf when none exists. Returns the new shelf id.
    pub fn ensure_default(&mut self) -> Option<i64> {
        if self.shelves.is_empty() {
            let id = self.add_shelf("Shelf 1".into(), PALETTE[0]);
            return Some(id);
        }
        None
    }

    pub fn next_color(&self) -> u32 {
        PALETTE[self.shelves.len() % PALETTE.len()]
    }

    pub fn get(&self, id: i64) -> Option<&Shelf> {
        self.shelves.iter().find(|s| s.id == id)
    }

    pub fn get_mut(&mut self, id: i64) -> Option<&mut Shelf> {
        self.shelves.iter_mut().find(|s| s.id == id)
    }

    pub fn active_shelf(&self) -> Option<&Shelf> {
        self.get(self.active).or(self.shelves.first())
    }

    pub fn add_shelf(&mut self, name: String, color: u32) -> i64 {
        let id = self.next_shelf;
        self.next_shelf += 1;
        self.shelves.push(Shelf { id, name, color, entries: Vec::new() });
        if self.shelves.len() == 1 {
            self.active = id;
        }
        id
    }

    pub fn remove_shelf(&mut self, id: i64) -> Option<Shelf> {
        let pos = self.shelves.iter().position(|s| s.id == id)?;
        let s = self.shelves.remove(pos);
        if self.active == id {
            self.active = self.shelves.first().map(|s| s.id).unwrap_or(0);
        }
        Some(s)
    }

    pub fn add_entry(&mut self, shelf_id: i64, content: ClipContent, created_at_ms: i64) -> Option<i64> {
        let id = self.next_entry;
        let shelf = self.get_mut(shelf_id)?;
        shelf.entries.push(ShelfEntry { id, content, created_at_ms });
        self.next_entry += 1;
        Some(id)
    }

    pub fn find_entry(&self, entry_id: i64) -> Option<(&Shelf, &ShelfEntry)> {
        for s in &self.shelves {
            if let Some(e) = s.entries.iter().find(|e| e.id == entry_id) {
                return Some((s, e));
            }
        }
        None
    }

    pub fn entry_mut(&mut self, entry_id: i64) -> Option<&mut ShelfEntry> {
        self.shelves.iter_mut().flat_map(|s| s.entries.iter_mut()).find(|e| e.id == entry_id)
    }

    pub fn remove_entry(&mut self, entry_id: i64) -> Option<(i64, ShelfEntry)> {
        for s in &mut self.shelves {
            if let Some(pos) = s.entries.iter().position(|e| e.id == entry_id) {
                return Some((s.id, s.entries.remove(pos)));
            }
        }
        None
    }

    pub fn clear(&mut self, shelf_id: i64) -> Vec<ShelfEntry> {
        self.get_mut(shelf_id).map(|s| std::mem::take(&mut s.entries)).unwrap_or_default()
    }

    pub fn count(&self, shelf_id: i64) -> usize {
        self.get(shelf_id).map(|s| s.entries.len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shelf_lifecycle() {
        let mut st = ShelfStore::new();
        assert!(st.ensure_default().is_some());
        let a = st.active;
        let e1 = st.add_entry(a, ClipContent::text("one".into()), 1).unwrap();
        let e2 = st.add_entry(a, ClipContent::text("two".into()), 2).unwrap();
        assert_eq!(st.count(a), 2);
        assert_eq!(st.find_entry(e2).unwrap().1.id, e2);
        let b = st.add_shelf("B".into(), st.next_color());
        assert_ne!(st.get(b).unwrap().color, st.get(a).unwrap().color);
        assert_eq!(st.remove_entry(e1).unwrap().0, a);
        assert_eq!(st.clear(a).len(), 1);
        st.remove_shelf(a);
        assert_eq!(st.active, b);
        let item = ShelfEntry { id: 7, content: ClipContent::text("x".into()), created_at_ms: 0 }.as_item();
        assert_eq!(item.id, -7);
    }
}
