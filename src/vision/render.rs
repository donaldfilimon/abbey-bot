//! Pure presentation helpers for image descriptions and OCR.

use super::MAX_DESCRIBED_IMAGES;

/// Fold image descriptions into the message text, one bracketed line per
/// image, at most [`MAX_DESCRIBED_IMAGES`]. This is the string the intent
/// classifier and the persona see.
pub fn fold_descriptions(text: &str, described: &[(String, String)]) -> String {
    let mut out = text.to_string();
    for (filename, description) in described.iter().take(MAX_DESCRIBED_IMAGES) {
        out.push_str("\n[image ");
        out.push_str(filename);
        out.push_str(": ");
        out.push_str(description);
        out.push(']');
    }
    out
}

/// The `/see` reply: the persona speaking, then the description.
pub fn render_see(persona_label: &str, description: &str) -> String {
    let description = description.trim();
    if description.is_empty() {
        return format!("**{persona_label}** — I couldn't make anything out in that image.");
    }
    format!("**{persona_label}** — {description}")
}

/// The `/ocr` reply: the transcribed text in a code block, or a plain note
/// when the image carried none. Backticks inside the text would close the
/// block early, so they are softened.
pub fn render_ocr(text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        return "No text found.".to_string();
    }
    format!("```\n{}\n```", text.replace("```", "ʼʼʼ"))
}
