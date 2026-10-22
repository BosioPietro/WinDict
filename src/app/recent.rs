//! A short memory of answers, for the online lookups and the translations:
//! asked again, they come back at once instead of over the network.

/// Up to `capacity` values by key. Making room forgets the one used least
/// recently.
pub(super) struct Recent<K, V> {
    /// Most recently used last.
    entries: Vec<(K, V)>,
    capacity: usize,
}

impl<K: PartialEq, V: Clone> Recent<K, V> {
    pub(super) fn new(capacity: usize) -> Self {
        Recent {
            entries: Vec::new(),
            capacity,
        }
    }

    /// The value kept for `key`, if any; that counts as using it.
    pub(super) fn get(&mut self, key: &K) -> Option<V> {
        let at = self.entries.iter().position(|(k, _)| k == key)?;
        let entry = self.entries.remove(at);
        let value = entry.1.clone();
        self.entries.push(entry);
        Some(value)
    }

    /// Keeps `value` for `key`, in place of any it had.
    pub(super) fn put(&mut self, key: K, value: V) {
        self.entries.retain(|(k, _)| *k != key);
        self.entries.push((key, value));
        if self.entries.len() > self.capacity {
            self.entries.remove(0);
        }
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembers_and_replaces() {
        let mut recent = Recent::new(2);
        assert_eq!(recent.get(&"a"), None);
        recent.put("a", 1);
        recent.put("a", 2);
        assert_eq!(recent.get(&"a"), Some(2));
        recent.clear();
        assert_eq!(recent.get(&"a"), None);
    }

    #[test]
    fn forgets_the_least_recently_used() {
        let mut recent = Recent::new(2);
        recent.put("a", 1);
        recent.put("b", 2);
        // Using "a" makes "b" the one to go.
        assert_eq!(recent.get(&"a"), Some(1));
        recent.put("c", 3);
        assert_eq!(recent.get(&"b"), None);
        assert_eq!(recent.get(&"a"), Some(1));
        assert_eq!(recent.get(&"c"), Some(3));
        // Putting one again counts as using it, too.
        recent.put("a", 4);
        recent.put("d", 5);
        assert_eq!(recent.get(&"c"), None);
        assert_eq!(recent.get(&"a"), Some(4));
    }
}
