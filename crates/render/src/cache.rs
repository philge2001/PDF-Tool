//! Einfacher LRU-Cache mit Byte-Budget (generisch, damit die App GPU-Texturen darin halten kann).

use std::collections::HashMap;
use std::hash::Hash;

struct Entry<V> {
    value: V,
    bytes: usize,
    last_used: u64,
}

/// LRU = "least recently used": Ist das Budget voll, fliegt der am längsten
/// nicht benutzte Eintrag raus.
///
/// `K: Eq + Hash + Clone` sind *Trait-Bounds*: Wir verlangen, dass der Schlüsseltyp
/// vergleichbar, hashbar und klonbar ist – sonst ließe er sich nicht in der HashMap nutzen.
pub struct ByteLru<K: Eq + Hash + Clone, V> {
    map: HashMap<K, Entry<V>>,
    budget: usize,
    used: usize,
    clock: u64,
}

impl<K: Eq + Hash + Clone, V> ByteLru<K, V> {
    pub fn new(budget_bytes: usize) -> Self {
        Self {
            map: HashMap::new(),
            budget: budget_bytes,
            used: 0,
            clock: 0,
        }
    }

    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    /// Holt einen Eintrag und markiert ihn als gerade benutzt.
    pub fn get(&mut self, key: &K) -> Option<&V> {
        let now = self.tick();
        // `map(|e| ...)` auf Option: wandelt Some(entry) in Some(&value) um.
        self.map.get_mut(key).map(|e| {
            e.last_used = now;
            &e.value
        })
    }

    /// Wie `get`, aber ohne die Nutzung zu vermerken.
    pub fn peek(&self, key: &K) -> Option<&V> {
        self.map.get(key).map(|e| &e.value)
    }

    pub fn contains(&self, key: &K) -> bool {
        self.map.contains_key(key)
    }

    /// Fügt ein und verdrängt bei Bedarf alte Einträge. Gibt die verdrängten Schlüssel zurück.
    pub fn insert(&mut self, key: K, value: V, bytes: usize) -> Vec<K> {
        let now = self.tick();
        if let Some(old) = self.map.insert(
            key,
            Entry {
                value,
                bytes,
                last_used: now,
            },
        ) {
            self.used -= old.bytes;
        }
        self.used += bytes;
        let mut evicted = Vec::new();
        // Mindestens den neuesten Eintrag behalten, selbst wenn er allein das Budget sprengt.
        while self.used > self.budget && self.map.len() > 1 {
            // O(n)-Suche reicht: Es liegen typischerweise nur einige hundert Einträge im Cache.
            let Some(oldest) = self
                .map
                .iter()
                .filter(|(_, e)| e.last_used != now)
                .min_by_key(|(_, e)| e.last_used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some(e) = self.map.remove(&oldest) {
                self.used -= e.bytes;
            }
            evicted.push(oldest);
        }
        evicted
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        self.map.remove(key).map(|e| {
            self.used -= e.bytes;
            e.value
        })
    }

    /// Entfernt alle Einträge, für die `keep` false liefert.
    pub fn retain(&mut self, mut keep: impl FnMut(&K) -> bool) {
        let used = &mut self.used;
        self.map.retain(|k, e| {
            let k_keep = keep(k);
            if !k_keep {
                *used -= e.bytes;
            }
            k_keep
        });
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.used = 0;
    }

    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.map.keys()
    }

    pub fn used_bytes(&self) -> usize {
        self.used
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_least_recently_used() {
        let mut c = ByteLru::new(100);
        c.insert("a", 1, 40);
        c.insert("b", 2, 40);
        assert_eq!(c.get(&"a"), Some(&1)); // a ist jetzt jünger als b
        let evicted = c.insert("c", 3, 40);
        assert_eq!(evicted, vec!["b"]);
        assert!(c.contains(&"a") && c.contains(&"c"));
        assert_eq!(c.used_bytes(), 80);
    }

    #[test]
    fn replacing_updates_size() {
        let mut c = ByteLru::new(100);
        c.insert(1, (), 30);
        c.insert(1, (), 50);
        assert_eq!(c.used_bytes(), 50);
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn oversized_entry_is_kept() {
        let mut c = ByteLru::new(10);
        c.insert(1, (), 5);
        let evicted = c.insert(2, (), 50);
        assert_eq!(evicted, vec![1]);
        assert!(c.contains(&2));
    }

    #[test]
    fn retain_tracks_bytes() {
        let mut c = ByteLru::new(100);
        c.insert(1, (), 10);
        c.insert(2, (), 20);
        c.retain(|k| *k == 2);
        assert_eq!(c.used_bytes(), 20);
    }
}
