//! Strips Gemma channel markers from model output before it can reach a
//! reply. Pure: no I/O, no clock, no locks.
//!
//! Grammar, as the pinned Gemma 4 chat template and the MLX-VLM server emit it:
//!
//! ```text
//! <|channel>  NAME "\n"  REASONING...  <channel|>
//! ```
//!
//! The open literal is `<|channel>`, the close literal is `<channel|>`, the
//! channel name observed is `thought`, and whatever sits between the two
//! literals (name, newline, reasoning text) is model-internal and never
//! user-visible. The observed leak is the empty pre-closed block
//! `<|channel>thought\n<channel|>` repeated in `content`.
//!
//! Sources:
//! - `docs/superpowers/specs/2026-09-04-mlx-vlm-tool-continuation-diagnosis.md`:
//!   `chat_template.jinja` line 362 at snapshot
//!   `73bcf09092aa277861d5a191b989b666f7f32e8f` emits
//!   `<|channel>thought\n<channel|>`, and `mlx_vlm/server/responses_state.py`
//!   (`ThinkingStreamState`) splits on the pair `("<|channel>thought",
//!   "<channel|>")` only once, so later blocks land in `content` with their
//!   marker text. Its "Defence in depth" section is why this module exists.
//! - `tasks/goals.md` (2026-09 continuation probe): 3 of 3 streamed runs emitted
//!   `<|channel>thought\n<channel|>` repeatedly.
//! - `deploy/patch-mlx-vlm-tool-encoding.py` header: the same loop.
//!
//! Rules: every `<|channel>...<channel|>` block is removed with its enclosed
//! text; an unclosed trailing block is dropped to the end of the text; a stray
//! `<channel|>` without an open is removed; any other `<` passes through. Text
//! with no markers comes back byte-identical. This is a single left-to-right
//! pass: a marker that only forms by concatenation after a removal is not
//! re-scanned. It is defence in depth for the reply path, not a security
//! boundary.

/// Opens a channel block.
const OPEN: &str = "<|channel>";
/// Closes a channel block.
const CLOSE: &str = "<channel|>";

/// Whether `tail` is a proper, non-empty prefix of `marker`: text that could
/// still become the marker once more bytes arrive.
fn could_become(tail: &str, marker: &str) -> bool {
    tail.len() < marker.len() && marker.starts_with(tail)
}

/// Incremental stripper for streamed deltas. Holds back any suffix that could
/// be the start of a marker and suppresses text inside a block.
#[derive(Debug, Default)]
pub(super) struct Stripper {
    inside: bool,
    pending: String,
}

impl Stripper {
    /// Consume one delta and return the text that is safe to emit now.
    pub(super) fn push(&mut self, delta: &str) -> String {
        self.pending.push_str(delta);
        let mut out = String::new();
        loop {
            if self.inside {
                if let Some(at) = self.pending.find(CLOSE) {
                    self.pending.drain(..at + CLOSE.len());
                    self.inside = false;
                    continue;
                }
                // Keep only a tail that could complete the close marker. The
                // marker holds a single `<`, so its prefix starts at the last one.
                let keep = self
                    .pending
                    .rfind('<')
                    .filter(|&at| could_become(&self.pending[at..], CLOSE));
                match keep {
                    Some(at) => {
                        self.pending.drain(..at);
                    }
                    None => self.pending.clear(),
                }
                break;
            }
            let Some(at) = self.pending.find('<') else {
                out.push_str(&self.pending);
                self.pending.clear();
                break;
            };
            out.push_str(&self.pending[..at]);
            self.pending.drain(..at);
            if self.pending.starts_with(OPEN) {
                self.pending.drain(..OPEN.len());
                self.inside = true;
            } else if self.pending.starts_with(CLOSE) {
                self.pending.drain(..CLOSE.len());
            } else if could_become(&self.pending, OPEN) || could_become(&self.pending, CLOSE) {
                break;
            } else {
                out.push('<');
                self.pending.drain(..1);
            }
        }
        out
    }

    /// End of text: flush held-back plain text and drop an unclosed block.
    pub(super) fn finish(&mut self) -> String {
        let held = std::mem::take(&mut self.pending);
        if std::mem::take(&mut self.inside) {
            String::new()
        } else {
            held
        }
    }
}

/// Strip every channel block from complete text.
pub(super) fn strip(text: &str) -> String {
    let mut stripper = Stripper::default();
    let mut out = stripper.push(text);
    out.push_str(&stripper.finish());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEAK: &str = "<|channel>thought\n<channel|>";

    #[test]
    fn a_whole_block_is_removed_with_its_text() {
        assert_eq!(
            strip("Hi <|channel>thought\nprivate reasoning<channel|>there."),
            "Hi there."
        );
    }

    #[test]
    fn multiple_blocks_are_all_removed() {
        assert_eq!(
            strip("a<|channel>thought\nx<channel|>b<|channel>thought\ny<channel|>c"),
            "abc"
        );
    }

    #[test]
    fn the_observed_repeated_empty_block_is_removed() {
        let leaked = format!("{LEAK}{LEAK}Answer.{LEAK}");
        assert_eq!(strip(&leaked), "Answer.");
        assert_eq!(strip(&LEAK.repeat(12)), "");
    }

    #[test]
    fn an_unclosed_trailing_block_is_dropped_never_leaked() {
        assert_eq!(strip("Visible.<|channel>thought\nhalf a thou"), "Visible.");
        let mut stripper = Stripper::default();
        assert_eq!(stripper.push("ok <|channel>thought\nmore"), "ok ");
        assert_eq!(stripper.push(" and more <chan"), "");
        assert_eq!(stripper.finish(), "");
    }

    #[test]
    fn a_stray_close_is_removed() {
        assert_eq!(strip("before<channel|>after"), "beforeafter");
    }

    #[test]
    fn marker_free_text_is_byte_identical() {
        for text in [
            "",
            "plain",
            "a < b and c > d",
            "<|other> and <other|> and <chan and <|chan",
            "café ☕ 日本語 🦆 <b>bold</b>",
        ] {
            assert_eq!(strip(text), text, "changed {text:?}");
        }
    }

    #[test]
    fn non_marker_angle_brackets_pass_through() {
        assert_eq!(strip("a < b"), "a < b");
        assert_eq!(strip("<|other>"), "<|other>");
        assert_eq!(strip("<<|channel>x<channel|>"), "<");
        assert_eq!(strip("ends with <|chan"), "ends with <|chan");
        assert_eq!(strip("ends with <"), "ends with <");
    }

    #[test]
    fn every_split_point_streams_to_the_one_shot_result() {
        let sample = format!(
            "Café <{LEAK}je suis <|channel>thought\nprivé 日本<channel|>là. a<b <channel|>{LEAK}fin 🦆<|channel>thought\nunclosed"
        );
        let whole = strip(&sample);
        assert_eq!(whole, "Café <je suis là. a<b fin 🦆");
        for (split, _) in sample.char_indices() {
            let mut stripper = Stripper::default();
            let mut streamed = stripper.push(&sample[..split]);
            streamed.push_str(&stripper.push(&sample[split..]));
            streamed.push_str(&stripper.finish());
            assert_eq!(streamed, whole, "split at byte {split}");
        }
        // One char per delta, the worst case for held-back prefixes.
        let mut stripper = Stripper::default();
        let mut streamed = String::new();
        for ch in sample.chars() {
            streamed.push_str(&stripper.push(ch.encode_utf8(&mut [0; 4])));
        }
        streamed.push_str(&stripper.finish());
        assert_eq!(streamed, whole);
    }
}
