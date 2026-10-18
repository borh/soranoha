//! Adapter: drive the historical modernizer from a
//! `kindai-bungo`-backed [`VibratoAnalyzer`].
//!
//! `ab-ortho-detect` defines [`HistoricalOracle`] (surface + span + `pron` +
//! `pos1` per token) and the [`HistoricalRewriteV1`] detector but depends on no
//! analyzer crate; this module supplies the concrete oracle over Vibrato, as
//! [`crate::ortho_compat`] does for the heuristic detector.
//!
//! **Dictionary requirement.** The oracle must be backed by the historical
//! `kindai-bungo` UniDic, the dictionary that segments 歴史的仮名遣い correctly
//! and yields the modern `pron`. This impl is generic over any
//! `VibratoAnalyzer`; [`historical_rewrite_detector`] is the entry point
//! because it binds the analyzer's own `archive_content_hash` as the
//! detector's `dictionary_hash`, so the identity matches the dictionary used.

use std::sync::Arc;

use ab_ortho_detect::{HistOracleToken, HistoricalOracle, HistoricalRewriteV1};
use ab_plaintext::{PlainTextDocument, SourceFormat};

use crate::{AnalyzerError, MorphAnalyzer, VibratoAnalyzer};

impl HistoricalOracle for VibratoAnalyzer {
    /// Tokenize one sentence and project each morpheme to the fields the
    /// historical modernizer reads. Byte spans are morpheme `byte_span`s
    /// (relative to `text`), which the detector lifts into source coordinates.
    ///
    /// # Failure mode
    /// On Vibrato `analyze` error this returns an empty token stream (logged to
    /// stderr), so the detector emits no annotations for the sentence and the
    /// text passes through un-modernized instead of the run aborting; the
    /// failure contract is the same as `ortho_compat`'s.
    fn tokenize(&self, text: &str) -> Vec<HistOracleToken> {
        let doc = PlainTextDocument {
            text_id: String::new(),
            source_format: SourceFormat::AozoraHonbun,
            text: text.to_owned(),
        };
        match self.analyze(&doc) {
            Ok(analysis) => analysis
                .morphemes
                .iter()
                .map(|m| HistOracleToken {
                    surface: m.surface.clone(),
                    byte_span: m.byte_span.clone(),
                    pron: feature(m, "pron"),
                    pos1: feature(m, "pos1"),
                })
                .collect(),
            Err(e) => {
                eprintln!(
                    "warn: ortho Lane-B oracle: Vibrato analyze failed ({e}); \
                     emitting no historical rewrites for this sentence"
                );
                Vec::new()
            }
        }
    }
}

fn feature(m: &ab_morph_diff::Morpheme, key: &str) -> Option<String> {
    m.features
        .get(key)
        .and_then(|opt| opt.as_deref())
        .map(String::from)
}

/// Build the historical→modern detector over a `kindai-bungo`-backed
/// analyzer, binding the analyzer's archive content hash as the detector's
/// `dictionary_hash`. Pass an analyzer that was loaded from the
/// historical UniDic; the resulting policy identity pins that exact archive.
///
/// # Errors
///
/// Returns [`AnalyzerError::DictionaryLoad`] if the archive cannot be hashed.
pub fn historical_rewrite_detector(
    oracle: Arc<VibratoAnalyzer>,
) -> Result<HistoricalRewriteV1, AnalyzerError> {
    let dictionary_hash = oracle.archive_content_hash()?;
    Ok(HistoricalRewriteV1::new(oracle, dictionary_hash))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-time assertion that `VibratoAnalyzer` satisfies the
    /// `HistoricalOracle: Send + Sync` supertrait bound (so `Arc<VibratoAnalyzer>`
    /// coerces to `Arc<dyn HistoricalOracle>`).
    #[test]
    fn vibrato_is_historical_oracle() {
        fn assert_oracle<T: HistoricalOracle>() {}
        assert_oracle::<VibratoAnalyzer>();
    }
}
