use std::sync::Arc;

use crate::database;

mod auth;
mod batch;
mod canonical;
pub mod errors;
mod handlers;
mod introspect;
mod legacy;
mod server;
#[cfg(test)]
mod tests;
pub mod types_canonical;

pub use auth::bearer_auth;
pub use server::RunToolResult;

#[derive(Clone)]
pub struct AppState {
    pub cfg: crate::config::Config,
    pub manager: database::Manager,
}

#[derive(Clone)]
pub struct DbMcpService {
    state: Arc<AppState>,
}

impl DbMcpService {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }
}
