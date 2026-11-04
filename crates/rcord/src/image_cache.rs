//! A size-bounded image cache. GPUI's default keeps every decoded image for the life of
//! the window; a chat client scrolls past thousands, so this one evicts the least
//! recently drawn images once their decoded size passes the budget.

use std::collections::HashMap;
use std::sync::Arc;

use futures::FutureExt;
use futures::future::Shared;
use gpui::{
    App, AppContext, Asset, AssetLogger, Entity, ImageAssetLoader, ImageCache, ImageCacheError,
    RenderImage, Resource, Task, Window, hash,
};

/// The main window's cache, for other windows to draw from the same images.
pub struct SharedImages(pub Entity<LruImageCache>);

impl gpui::Global for SharedImages {}

type Load = Shared<Task<Result<Arc<RenderImage>, ImageCacheError>>>;

struct Entry {
    load: Load,
    bytes: usize,
    last_used: u64,
}

pub struct LruImageCache {
    entries: HashMap<u64, Entry>,
    budget: usize,
    used: usize,
    clock: u64,
}

impl LruImageCache {
    pub fn new(budget_bytes: usize, cx: &mut App) -> Entity<Self> {
        let cache = cx.new(|_| Self {
            entries: HashMap::new(),
            budget: budget_bytes,
            used: 0,
            clock: 0,
        });
        cx.observe_release(&cache, |cache, cx| {
            for (_, entry) in cache.entries.drain() {
                if let Some(Ok(image)) = entry.load.now_or_never() {
                    cx.drop_image(image, None);
                }
            }
        })
        .detach();
        cache
    }

    pub fn set_budget(&mut self, bytes: usize) {
        self.budget = bytes;
    }

    fn evict(&mut self, keep: u64, window: &mut Window, cx: &mut App) {
        while self.used > self.budget && self.entries.len() > 1 {
            let Some((&oldest, _)) = self
                .entries
                .iter()
                .filter(|(k, e)| **k != keep && e.bytes > 0)
                .min_by_key(|(_, e)| e.last_used)
            else {
                break;
            };
            if let Some(entry) = self.entries.remove(&oldest) {
                self.used = self.used.saturating_sub(entry.bytes);
                if let Some(Ok(image)) = entry.load.now_or_never() {
                    cx.drop_image(image, Some(window));
                }
            }
        }
    }
}

fn decoded_size(image: &RenderImage) -> usize {
    (0..image.frame_count())
        .map(|i| {
            let s = image.size(i);
            (s.width.0.max(0) as usize) * (s.height.0.max(0) as usize) * 4
        })
        .sum()
}

impl ImageCache for LruImageCache {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let key = hash(resource);
        self.clock += 1;
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.last_used = self.clock;
            let result = entry.load.clone().now_or_never()?;
            if entry.bytes == 0
                && let Ok(image) = &result
            {
                entry.bytes = decoded_size(image).max(1);
                self.used += entry.bytes;
                self.evict(key, window, cx);
            }
            return Some(result);
        }

        let fut = AssetLogger::<ImageAssetLoader>::load(resource.clone(), cx);
        let load = cx.background_executor().spawn(fut).shared();
        self.entries.insert(
            key,
            Entry {
                load: load.clone(),
                bytes: 0,
                last_used: self.clock,
            },
        );
        let view = window.current_view();
        window
            .spawn(cx, async move |cx| {
                let _ = load.await;
                cx.on_next_frame(move |_, cx| cx.notify(view));
            })
            .detach();
        None
    }
}
