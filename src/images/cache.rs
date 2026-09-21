//! LRU caches of Direct2D bitmaps (UI thread only).

use crate::model::item::ItemId;
use crate::msg::BgraBuf;
use std::collections::{HashMap, HashSet};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT, D2D_SIZE_U};
use windows::Win32::Graphics::Direct2D::{ID2D1Bitmap1, ID2D1DeviceContext, D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_PROPERTIES1};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;

pub struct Entry {
    pub bitmap: ID2D1Bitmap1,
    pub width: u32,
    pub height: u32,
    bytes: usize,
    last_use: u64,
}

pub struct BitmapCache {
    map: HashMap<ItemId, Entry>,
    total: usize,
    cap: usize,
    tick: u64,
    /// Requests already sent to the worker (avoid duplicate work).
    pub pending: HashSet<ItemId>,
    /// Ids that failed to load; don't retry every frame.
    pub failed: HashSet<ItemId>,
}

impl BitmapCache {
    pub fn new(cap_bytes: usize) -> Self {
        BitmapCache { map: HashMap::new(), total: 0, cap: cap_bytes, tick: 0, pending: HashSet::new(), failed: HashSet::new() }
    }

    pub fn set_cap(&mut self, cap_bytes: usize) {
        self.cap = cap_bytes;
        self.evict();
    }

    pub fn total_bytes(&self) -> usize {
        self.total
    }

    pub fn contains(&self, id: ItemId) -> bool {
        self.map.contains_key(&id)
    }

    pub fn get(&mut self, id: ItemId) -> Option<&Entry> {
        self.tick += 1;
        let t = self.tick;
        if let Some(e) = self.map.get_mut(&id) {
            e.last_use = t;
        }
        self.map.get(&id)
    }

    pub fn insert(&mut self, dc: &ID2D1DeviceContext, id: ItemId, buf: &BgraBuf) -> bool {
        self.pending.remove(&id);
        let Some(bitmap) = create_bitmap(dc, buf) else {
            self.failed.insert(id);
            return false;
        };
        self.remove(id);
        let bytes = buf.pixels.len();
        self.tick += 1;
        self.map.insert(id, Entry { bitmap, width: buf.width, height: buf.height, bytes, last_use: self.tick });
        self.total += bytes;
        self.failed.remove(&id);
        self.evict();
        true
    }

    pub fn remove(&mut self, id: ItemId) {
        if let Some(e) = self.map.remove(&id) {
            self.total -= e.bytes;
        }
        self.pending.remove(&id);
        self.failed.remove(&id);
    }

    /// Drop all GPU resources (device lost); keeps nothing.
    pub fn clear(&mut self) {
        self.map.clear();
        self.total = 0;
        self.pending.clear();
    }

    fn evict(&mut self) {
        while self.total > self.cap && self.map.len() > 1 {
            let Some((&victim, _)) = self.map.iter().min_by_key(|(_, e)| e.last_use) else { break };
            if let Some(e) = self.map.remove(&victim) {
                self.total -= e.bytes;
            }
        }
    }
}

pub fn create_bitmap(dc: &ID2D1DeviceContext, buf: &BgraBuf) -> Option<ID2D1Bitmap1> {
    if buf.width == 0 || buf.height == 0 || buf.pixels.len() < (buf.width * buf.height * 4) as usize {
        return None;
    }
    let props = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
        colorContext: std::mem::ManuallyDrop::new(None),
    };
    unsafe {
        dc.CreateBitmap(
            D2D_SIZE_U { width: buf.width, height: buf.height },
            Some(buf.pixels.as_ptr() as *const _),
            buf.width * 4,
            &props,
        )
        .ok()
    }
}
