use anyhow::{Context, Result};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct AppState {
    pub root_dir: PathBuf,
    pub data_dir: PathBuf,
    pub ocrys_dir: PathBuf,
    pub db_path: PathBuf,
    pub ocr_lang: String,
}

impl AppState {
    pub async fn new() -> Result<Self> {
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
            ocr_lang: "ita".to_string(),
        })
    }

    pub fn ocr_run_dir(&self, run_id: &str) -> PathBuf {
        self.ocrys_dir.join(run_id)
    }
}


