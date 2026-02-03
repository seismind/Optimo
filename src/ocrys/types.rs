pub struct OCRDocument {
    pub source: String,
    pub pages: Vec<OCRPage>,
}

pub struct OCRPage {
    pub page_number: usize,
    pub lines: Vec<OCRLine>,
}

pub struct OCRLine {
    pub text: String,
    pub confidence: Option<f32>,
}

