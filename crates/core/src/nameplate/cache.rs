use std::collections::BTreeMap;
use std::sync::Arc;

use super::image::{ImageKey, LayerImage};

pub const DEFAULT_CACHE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Default)]
pub struct ImageCache {
    budget: usize,
    bytes: usize,
    tick: u64,
    entries: BTreeMap<ImageKey, (Arc<LayerImage>, u64)>,
    recency: BTreeMap<u64, ImageKey>,
    hits: u64,
    misses: u64,
    evictions: u64,
}

impl ImageCache {
    pub fn new(budget: usize) -> ImageCache {
        ImageCache {
            budget,
            ..ImageCache::default()
        }
    }

    pub fn budget(&self) -> usize {
        self.budget
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn hits(&self) -> u64 {
        self.hits
    }

    pub fn misses(&self) -> u64 {
        self.misses
    }

    pub fn evictions(&self) -> u64 {
        self.evictions
    }

    pub fn get(&mut self, key: &ImageKey) -> Option<Arc<LayerImage>> {
        self.tick += 1;
        let tick = self.tick;
        let Some((image, used)) = self.entries.get_mut(key) else {
            self.misses += 1;
            return None;
        };
        self.recency.remove(used);
        *used = tick;
        self.recency.insert(tick, *key);
        self.hits += 1;
        Some(image.clone())
    }

    pub fn get_or_paint(
        &mut self,
        key: ImageKey,
        paint: impl FnOnce() -> LayerImage,
    ) -> Arc<LayerImage> {
        if let Some(image) = self.get(&key) {
            return image;
        }
        let image = Arc::new(paint());
        let size = image_size(&image);
        if size > self.budget {
            return image;
        }
        self.tick += 1;
        self.entries.insert(key, (image.clone(), self.tick));
        self.recency.insert(self.tick, key);
        self.bytes += size;
        while self.bytes > self.budget {
            let Some((_, oldest)) = self.recency.pop_first() else {
                break;
            };
            if let Some((evicted, _)) = self.entries.remove(&oldest) {
                self.bytes -= image_size(&evicted);
                self.evictions += 1;
            }
        }
        image
    }

    pub fn remove_asset(&mut self, asset: u32) {
        let from = ImageKey {
            asset,
            layer: 0,
            state: 0,
            inputs: 0,
        };
        let to = ImageKey {
            asset,
            layer: u8::MAX,
            state: u16::MAX,
            inputs: u64::MAX,
        };
        let keys: Vec<ImageKey> = self.entries.range(from..=to).map(|(k, _)| *k).collect();
        for key in keys {
            if let Some((image, used)) = self.entries.remove(&key) {
                self.recency.remove(&used);
                self.bytes -= image_size(&image);
            }
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.recency.clear();
        self.bytes = 0;
    }
}

fn image_size(image: &LayerImage) -> usize {
    usize::try_from(image.bytes).unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::super::image::{ColorRef, PaintRect};
    use super::*;

    fn image(rects: usize) -> LayerImage {
        let rect = |i: usize| PaintRect { x0: i as i16 * 2, y0: 0, x1: i as i16 * 2 + 1, y1: 1, color: ColorRef::slot(0, 0) };
        LayerImage::new((0..rects).map(rect).collect(), Vec::new())
    }

    #[test]
    fn the_cache_paints_misses_once_and_evicts_like_a_reference_lru() {
        let key = |asset, state| ImageKey { asset, layer: 0, state, inputs: 7 };
        let budget = 5 * 84 + 30;
        let mut cache = ImageCache::new(budget);
        let mut model: Vec<(ImageKey, usize)> = Vec::new();
        let mut state = 12345_u32;
        for _ in 0..2000 {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let (k, rects) = (key((state >> 16) % 3, ((state >> 8) % 6) as u16), ((state >> 4) % 4) as usize);
            if let Some(i) = model.iter().position(|(m, _)| *m == k) {
                let entry = model.remove(i);
                model.push(entry);
            } else {
                model.push((k, 64 + 20 * rects));
                while model.iter().map(|(_, s)| s).sum::<usize>() > budget {
                    model.remove(0);
                }
            }
            let first = cache.get_or_paint(k, || image(rects));
            assert!(Arc::ptr_eq(&first, &cache.get_or_paint(k, || unreachable!())));
            assert_eq!((cache.len(), cache.bytes()), (model.len(), model.iter().map(|(_, s)| s).sum::<usize>()));
        }
        assert!(model.iter().all(|(k, _)| cache.get(k).is_some()));
        cache.remove_asset(1);
        assert!(model.iter().all(|(k, _)| cache.get(k).is_some() == (k.asset != 1)));
        let mut small = ImageCache::new(84);
        assert_eq!(small.get_or_paint(key(1, 1), || image(6)).rects.len(), 6);
        assert!(small.is_empty(), "oversized images are returned but not kept");
    }
}
