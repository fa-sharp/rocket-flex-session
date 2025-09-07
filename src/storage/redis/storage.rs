use fred::prelude::{FromValue, KeysInterface, Value};
use rocket::http::CookieJar;

use crate::{
    error::{SessionError, SessionResult},
    storage::SessionStorage,
};

use super::RedisFredStorage;

#[rocket::async_trait]
impl<T> SessionStorage<T> for RedisFredStorage
where
    T: FromValue + TryInto<Value> + Clone + Send + Sync + 'static,
    <T as TryInto<Value>>::Error: std::error::Error + Send + Sync + 'static,
{
    async fn load(
        &self,
        id: &str,
        ttl: Option<u32>,
        _cookie_jar: &CookieJar,
    ) -> SessionResult<(T, u32)> {
        let (value, ttl) = self.fetch_session(id, ttl).await?;
        let data = T::from_value(value)?;
        Ok((data, ttl))
    }

    async fn save(&self, id: &str, data: T, ttl: u32) -> SessionResult<()> {
        let value: Value = data
            .try_into()
            .map_err(|e| SessionError::Serialization(Box::new(e)))?;
        self.save_session(id, value, ttl).await?;
        Ok(())
    }

    async fn delete(&self, id: &str, _cookie_jar: &CookieJar) -> SessionResult<()> {
        let _: () = self.pool.del(self.session_key(id)).await?;
        Ok(())
    }
}
