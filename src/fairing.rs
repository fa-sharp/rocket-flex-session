use std::{
    marker::{Send, Sync},
    sync::Arc,
};

use rocket::{fairing::Fairing, Build, Orbit, Request, Response, Rocket};

use crate::{guard::LocalCachedSession, RocketFlexSession};

#[rocket::async_trait]
impl<T> Fairing for RocketFlexSession<T>
where
    T: Send + Sync + Clone + 'static,
{
    fn info(&self) -> rocket::fairing::Info {
        use rocket::fairing::Kind;
        rocket::fairing::Info {
            name: "Rocket Flex Session",
            kind: Kind::Ignite | Kind::Response | Kind::Shutdown | Kind::Singleton,
        }
    }

    async fn on_ignite(&self, rocket: Rocket<Build>) -> Result<Rocket<Build>, Rocket<Build>> {
        rocket::debug!("Setting up session resources...");
        if let Err(e) = self.storage.setup().await {
            rocket::warn!("Error during session storage setup: {}", e);
        }

        Ok(rocket.manage::<RocketFlexSession<T>>(RocketFlexSession {
            options: self.options.clone(),
            storage: self.storage.clone(),
        }))
    }

    async fn on_response<'r>(&self, req: &'r Request<'_>, _res: &mut Response<'r>) {
        // Get session data from request local cache, or generate a default empty one
        let (session_inner, _): &LocalCachedSession<T> = req.local_cache(|| (Arc::default(), None));

        // Take inner session data
        let (updated, deleted) = session_inner.lock().unwrap().take_for_storage();

        // Handle deleted session
        if let Some(deleted_id) = deleted {
            let delete_result = self.storage.delete(&deleted_id, req.cookies()).await;
            if let Err(e) = delete_result {
                rocket::error!("Error while deleting session '{}': {}", deleted_id, e);
            }
        }

        // Handle updated session
        if let Some((id, pending_data, ttl)) = updated {
            let save_result = self.storage.save(&id, pending_data, ttl).await;
            if let Err(e) = save_result {
                rocket::error!("Error while saving session '{}': {}", &id, e);
            }
        }
    }

    async fn on_shutdown(&self, _rocket: &Rocket<Orbit>) {
        rocket::debug!("Shutting down session resources...");
        if let Err(e) = self.storage.shutdown().await {
            rocket::warn!("Error during session storage shutdown: {}", e);
        }
    }
}
