use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OCRDocument {
    pub source: String,
    pub pages: Vec<OCRPage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OCRPage {
    pub page_number: usize,
    pub lines: Vec<OCRLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OCRLine {
    pub text: String,
    pub confidence: Option<f32>,
}

