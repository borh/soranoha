use ab_ortho_detect::{OrthoToken, OrthoTokenizer};
use ab_plaintext::{PlainTextDocument, SourceFormat};

use crate::MorphAnalyzer;
use crate::VibratoAnalyzer;

impl OrthoTokenizer for VibratoAnalyzer {
    /// Tokenizes via Vibrato for the proper-noun guard in `HeuristicV1`.
    ///
    /// # Failure mode
    ///
    /// `tokenize()` panics on invalid input. On Vibrato `analyze` error (e.g.
    /// dictionary load failure) this returns an empty token list, which
    /// `HeuristicV1` reads as `proper_noun_char_ratio = 0`, so the proper-noun
    /// guard treats the sentence as containing no proper nouns. The error is
    /// logged to stderr; the `OrthoTokenizer` signature returns `Vec` and does
    /// not propagate it.
    fn tokenize(&self, text: &str) -> Vec<OrthoToken> {
        let doc = PlainTextDocument {
            text_id: String::new(),
            source_format: SourceFormat::AozoraHonbun,
            text: text.to_owned(),
        };
        match self.analyze(&doc) {
            Ok(analysis) => analysis
                .morphemes
                .iter()
                .map(|m| OrthoToken {
                    surface: m.surface.clone(),
                    pos2: m
                        .features
                        .get("pos2")
                        .and_then(|opt| opt.as_deref())
                        .map(String::from),
                })
                .collect(),
            Err(e) => {
                eprintln!(
                    "warn: ortho-detect proper-noun guard: Vibrato analyze failed \
                     ({e}); treating as zero tokens (guard degenerates to no-proper-noun)"
                );
                Vec::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-time assertion that `VibratoAnalyzer` satisfies the
    /// `OrthoTokenizer: Send + Sync` supertrait bound.
    #[test]
    fn vibrato_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<VibratoAnalyzer>();
    }
}
