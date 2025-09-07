use rand::distr::{Alphanumeric, SampleString};

use crate::SessionIdentifier;

/// Represents a current, active session
struct ActiveSession<T> {
    /// Session ID (20-character alphanumeric string)
    id: String,
    /// Original session data
    data: T,
    /// Updated session data
    pending_data: Option<T>,
    /// Time-to-live in seconds
    ttl: u32,
    /// Whether this is a new session that hasn't been stored yet
    new: bool,
}
impl<T: Clone> ActiveSession<T> {
    /// Create a new active session with a generated ID, to be saved in storage
    fn new(new_data: T, ttl: u32) -> Self {
        Self {
            id: Alphanumeric.sample_string(&mut rand::rng(), 20),
            data: new_data.clone(),
            pending_data: Some(new_data),
            ttl,
            new: true,
        }
    }
    /// Active session that already exists in storage
    fn existing(id: &str, data: T, ttl: u32) -> ActiveSession<T> {
        Self {
            id: id.to_owned(),
            data,
            pending_data: None,
            ttl,
            new: false,
        }
    }
}

/** Mutable session state, passed from the Session request guard */
pub(crate) struct SessionInner<T> {
    /// The current, active session
    current: Option<ActiveSession<T>>,
    /// The ID of the original session if deleted during the request
    deleted: Option<String>,
}
impl<T: Clone> Default for SessionInner<T> {
    fn default() -> Self {
        Self::new_empty()
    }
}

impl<T> SessionInner<T>
where
    T: Clone,
{
    pub(crate) fn new_empty() -> Self {
        Self {
            current: None,
            deleted: None,
        }
    }

    pub(crate) fn new_existing(id: &str, data: T, ttl: u32) -> Self {
        Self {
            current: Some(ActiveSession::existing(id, data, ttl)),
            deleted: None,
        }
    }

    pub(crate) fn get_id(&self) -> Option<&str> {
        self.current.as_ref().map(|s| s.id.as_str())
    }

    pub(crate) fn get_current_data(&self) -> Option<&T> {
        self.current
            .as_ref()
            .map(|s| s.pending_data.as_ref().unwrap_or(&s.data))
    }

    pub(crate) fn get_current_ttl(&self) -> Option<u32> {
        self.current.as_ref().map(|s| s.ttl)
    }

    pub(crate) fn is_new(&self) -> bool {
        self.current.as_ref().map(|s| s.new).unwrap_or(false)
    }

    pub(crate) fn set_data(&mut self, new_data: T, default_ttl: u32) {
        match &mut self.current {
            Some(current) => current.pending_data = Some(new_data),
            None => self.current = Some(ActiveSession::new(new_data, default_ttl)),
        }
    }

    pub(crate) fn set_ttl(&mut self, new_ttl: u32) {
        if let Some(current) = &mut self.current {
            current.ttl = new_ttl;
            if current.pending_data.is_none() {
                current.pending_data = Some(current.data.clone());
            }
        }
    }

    pub(crate) fn tap_data_mut<UpdateFn, R>(
        &mut self,
        callback: UpdateFn,
        default_ttl: u32,
    ) -> (R, bool)
    where
        UpdateFn: FnOnce(&mut Option<T>) -> R,
    {
        match &mut self.current {
            Some(current) => {
                if current.pending_data.is_none() {
                    current.pending_data = Some(current.data.clone());
                }
                let response = callback(&mut current.pending_data);
                let is_deleted = current.pending_data.is_none();
                if is_deleted {
                    self.delete();
                };
                (response, is_deleted)
            }
            None => {
                let mut pending_data = None;
                let response = callback(&mut pending_data);
                if let Some(new_data) = pending_data {
                    self.current = Some(ActiveSession::new(new_data, default_ttl));
                    (response, false)
                } else {
                    self.delete();
                    (response, true)
                }
            }
        }
    }

    /// Mark the current session ID as deleted, and clear all data. Can safely be called
    /// multiple times in a request if needed - the original session will still be deleted.
    pub(crate) fn delete(&mut self) {
        if let Some(current) = self.current.take() {
            self.deleted.get_or_insert(current.id);
        }
    }

    pub(crate) fn get_deleted_id(&self) -> Option<&str> {
        self.deleted.as_deref()
    }

    /// Take all data needed to update session storage. Returns a tuple of Options
    /// representing an updated session along with the id of a deleted session.
    /// This should only be called once at the end of the request.
    pub(crate) fn take_for_storage(&mut self) -> (Option<(String, T, u32)>, Option<String>) {
        (
            self.current
                .take()
                .and_then(|c| c.pending_data.map(|data| (c.id, data, c.ttl))),
            self.deleted.take(),
        )
    }
}

impl<T> SessionInner<T>
where
    T: SessionIdentifier + Clone,
{
    pub(crate) fn get_current_identifier(&self) -> Option<&T::Id> {
        self.get_current_data().and_then(|data| data.identifier())
    }
}
