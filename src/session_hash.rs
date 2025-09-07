use crate::Session;

/// Optional trait for sessions with a hashmap-like data structure.
pub trait SessionHashMap: Send + Sync + Clone + Default {
    /// The type of values stored in the session hashmap.
    type Value: Send + Sync + Clone;

    /// Get a reference to the value associated with the given key.
    fn get(&self, key: &str) -> Option<&Self::Value>;

    /// Inserts or updates a key-value pair into the map.
    fn insert(&mut self, key: String, value: Self::Value);

    /// Removes a key from the map.
    fn remove(&mut self, key: &str);

    // /// Returns the number of keys in the map.
    // fn len(&self) -> usize;

    // /// Returns an iterator over the key-value pairs in the map.
    // fn iter(&self) -> std::slice::Iter<'_, (&str, &Self::Value)>;

    // /// Returns an iterator over the key-value pairs in the map, with mutable references.
    // fn iter_mut(&mut self) -> std::slice::IterMut<'_, (&str, &mut Self::Value)>;
}

/// Implementation block for sessions with hashmap-like data structures
impl<T> Session<'_, T>
where
    T: SessionHashMap,
{
    /// Get the value of a key in the session data via cloning
    pub fn get_key(&self, key: &str) -> Option<T::Value> {
        self.get_inner_lock()
            .get_current_data()
            .and_then(|h| h.get(key).cloned())
    }

    /// Get the value of a key in the session data via a closure
    pub fn tap_key<F, R>(&self, key: &str, f: F) -> R
    where
        F: FnOnce(Option<&T::Value>) -> R,
    {
        f(self
            .get_inner_lock()
            .get_current_data()
            .and_then(|d| d.get(key)))
    }

    /// Set the value of a key in the session data. Will create a new session if there isn't one.
    pub fn set_key(&mut self, key: String, value: T::Value) {
        self.get_inner_lock().tap_data_mut(
            |data| data.get_or_insert_default().insert(key, value),
            self.get_default_ttl(),
        );
        self.update_cookies();
    }

    /// Remove a key from the session data.
    pub fn remove_key(&mut self, key: &str) {
        self.get_inner_lock().tap_data_mut(
            |data| {
                if let Some(data) = data {
                    data.remove(key);
                }
            },
            self.get_default_ttl(),
        );
        self.update_cookies();
    }
}
