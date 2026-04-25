use anyhow::{Context, Result};
use std::path::PathBuf;

use crate::profile::IngestionProfile;

#[derive(Debug, Clone)]
pub struct AppState {
    #[allow(dead_code)]
    pub root_dir: PathBuf,
    pub data_dir: PathBuf,
    pub ocrys_dir: PathBuf,
    #[allow(dead_code)]
    pub db_path: PathBuf,
    pub ocr_lang: String,
    /// Active ingestion policy — resolved once at startup, never mutated.
    pub ingestion_profile: IngestionProfile,
}

impl AppState {
    /// Create AppState from already-resolved values.
    /// All resolution (CLI/ENV/file/default) happens in `ResolvedConfig::resolve()`
    /// before this is called.
    pub async fn new(ingestion_profile: IngestionProfile, ocr_lang: String) -> Result<Self> {
        let root_dir = std::env::current_dir()
            .context("failed to resolve current working directory")?;

        let data_dir = root_dir.join("data");
        let ocrys_dir = data_dir.join("ocrys");
        let db_path = data_dir.join("optimo.sqlite");

        std::fs::create_dir_all(&ocrys_dir)
            .with_context(|| format!("failed to create OCRYS directory at {:?}", ocrys_dir))?;

        Ok(Self {
            root_dir,
            data_dir,
            ocrys_dir,
            db_path,
            ocr_lang,
            ingestion_profile,
        })
    }

    pub fn ocr_run_dir(&self, run_id: &str) -> PathBuf {
        self.ocrys_dir.join(run_id)
    }
}


