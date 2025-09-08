use rand::distr::{Alphanumeric, SampleString};

use crate::SessionIdentifier;

/// Status of the active session
#[derive(Debug, PartialEq, Eq)]
enum ActiveSessionStatus {
    /// This is a new session that hasn't been stored yet
    New,
    /// This is an existing session that is unmodified
    Existing,
    /// This is an existing session that has been updated
    Updated,
}

/// Represents a current, active session
struct ActiveSession<T> {
    /// Session ID (20-character alphanumeric string)
    id: String,
    /// Session data
    data: T,
    /// Time-to-live in seconds
    ttl: u32,
    /// Status of the active session
    status: ActiveSessionStatus,
}
impl<T> ActiveSession<T> {
    /// Create a new active session with a generated ID, to be saved in storage
    fn new(new_data: T, ttl: u32) -> Self {
        Self {
            id: Alphanumeric.sample_string(&mut rand::rng(), 20),
            data: new_data,
            ttl,
            status: ActiveSessionStatus::New,
        }
    }
    /// Active session that already exists in storage
    fn existing(id: &str, data: T, ttl: u32) -> ActiveSession<T> {
        Self {
            id: id.to_owned(),
            data,
            ttl,
            status: ActiveSessionStatus::Existing,
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
impl<T> Default for SessionInner<T> {
    fn default() -> Self {
        Self::new_empty()
    }
}

impl<T> SessionInner<T> {
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
        self.current.as_ref().map(|s| &s.data)
    }

    pub(crate) fn get_current_ttl(&self) -> Option<u32> {
        self.current.as_ref().map(|s| s.ttl)
    }

    pub(crate) fn is_new(&self) -> bool {
        self.current
            .as_ref()
            .map(|s| s.status == ActiveSessionStatus::New)
            .unwrap_or(false)
    }

    pub(crate) fn set_data(&mut self, new_data: T, default_ttl: u32) {
        match &mut self.current {
            Some(current) => {
                current.data = new_data;
                self.mark_updated();
            }
            None => self.current = Some(ActiveSession::new(new_data, default_ttl)),
        }
    }

    pub(crate) fn set_ttl(&mut self, new_ttl: u32) {
        if let Some(current) = &mut self.current {
            current.ttl = new_ttl;
            self.mark_updated();
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
        match self.current.take() {
            Some(current) => {
                let mut updated_data = Some(current.data);
                let response = callback(&mut updated_data);
                match updated_data {
                    Some(data) => {
                        self.current = Some(ActiveSession { data, ..current });
                        self.mark_updated();
                        (response, false)
                    }
                    None => {
                        self.delete();
                        (response, true)
                    }
                }
            }
            None => {
                let mut new_data: Option<T> = None;
                let response = callback(&mut new_data);
                if let Some(data) = new_data {
                    self.current = Some(ActiveSession::new(data, default_ttl));
                    (response, false)
                } else {
                    self.delete();
                    (response, true)
                }
            }
        }
    }

    /// If this is an existing session, mark it as updated to ensure it will be saved.
    pub(crate) fn mark_updated(&mut self) {
        if let Some(current) = self.current.as_mut() {
            if current.status == ActiveSessionStatus::Existing {
                current.status = ActiveSessionStatus::Updated;
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
        let updated_session = self
            .current
            .take()
            .filter(|c| {
                c.status == ActiveSessionStatus::New || c.status == ActiveSessionStatus::Updated
            })
            .map(|c| (c.id, c.data, c.ttl));
        (updated_session, self.deleted.take())
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
