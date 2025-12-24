use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub agastya: Arc<AgastyaState>,
}

impl AppState {
    pub async fn new() -> Self {
        let agastya = AgastyaState::init().await;

        Self {
            agastya: Arc::new(agastya),
        }
    }
}

// --- dominio ------------------------------------------------

pub struct AgastyaState;

impl AgastyaState {
    pub async fn init() -> Self {
        // in futuro: carico config, modelli, regole, ecc.
        Self
    }
}
