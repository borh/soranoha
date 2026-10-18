//! Per-work corrections of transcription defects in the pinned source,
//! read from `data/source-emendations.toml`.
//!
//! An entry applies to a decoded text in which its `transcribed` notation
//! occurs exactly once. The decoder receives no work identity; the source
//! inventory verifies that each entry applies to its declared edition only.

use std::sync::LazyLock;

use anyhow::{Result, anyhow, bail};
use serde::Deserialize;

const TABLE_TOML: &str = include_str!("../../../data/source-emendations.toml");

/// One table entry.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Emendation {
    pub id: String,
    /// Declared edition, verified by the source inventory.
    pub work_id: String,
    pub indexed_path: String,
    pub transcribed: String,
    pub emended: String,
}

/// An entry applied to one decoded text.
///
/// `byte_start..byte_end` is the extent of `emended` in the emended text;
/// lines are 1-based and inclusive.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, Deserialize))]
pub struct AppliedEmendation {
    pub id: String,
    pub transcribed: String,
    pub emended: String,
    pub byte_start: usize,
    pub byte_end: usize,
    pub line_start: usize,
    pub line_end: usize,
}

#[derive(Deserialize)]
struct Table {
    emendations_version: u32,
    #[serde(default)]
    emendation: Vec<Emendation>,
}

static TABLE: LazyLock<Result<Vec<Emendation>, String>> =
    LazyLock::new(|| parse_table(TABLE_TOML).map_err(|error| format!("{error:#}")));

/// The validated embedded table.
///
/// # Errors
///
/// Returns an error when the embedded table does not parse or violates the
/// invariants checked by [`parse_table`].
pub fn emendation_table() -> Result<&'static [Emendation]> {
    TABLE
        .as_deref()
        .map_err(|error| anyhow!("invalid source emendation table: {error}"))
}

/// Parse and validate an emendation table.
///
/// # Errors
///
/// Returns an error for an unsupported version, a duplicate or malformed id,
/// an empty `transcribed`, a `transcribed` or `emended` that contains a line
/// break, or an `emended` that contains its `transcribed` (which includes
/// equality). An applied emendation therefore lies within one source line.
pub fn parse_table(source: &str) -> Result<Vec<Emendation>> {
    let table: Table = toml::from_str(source)?;
    if table.emendations_version != 1 {
        bail!(
            "unsupported emendations_version {}",
            table.emendations_version
        );
    }
    let mut ids = std::collections::BTreeSet::new();
    for entry in &table.emendation {
        if !is_valid_id(&entry.id) {
            bail!("malformed emendation id {:?}", entry.id);
        }
        if !ids.insert(entry.id.as_str()) {
            bail!("duplicate emendation id {:?}", entry.id);
        }
        if entry.transcribed.is_empty() {
            bail!("emendation {} has an empty transcribed text", entry.id);
        }
        if [&entry.transcribed, &entry.emended]
            .iter()
            .any(|text| text.contains(['\n', '\r']))
        {
            bail!("emendation {} contains a line break", entry.id);
        }
        if entry.emended.contains(&entry.transcribed) {
            bail!(
                "emendation {} has an emended text that contains its transcribed text",
                entry.id
            );
        }
    }
    Ok(table.emendation)
}

fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

/// Apply the embedded table to a decoded text.
///
/// Returns `None` when no entry applies.
///
/// # Errors
///
/// Returns an error when the embedded table is invalid or when the matches of
/// two entries overlap; no entry is applied in either case.
pub fn emend(text: &str) -> Result<Option<(String, Vec<AppliedEmendation>)>> {
    emend_with(emendation_table()?, text)
}

/// Apply `entries` to a decoded text; see [`emend`].
///
/// # Errors
///
/// Returns an error when the matches of two entries overlap.
pub fn emend_with(
    entries: &[Emendation],
    text: &str,
) -> Result<Option<(String, Vec<AppliedEmendation>)>> {
    let mut matches = entries
        .iter()
        .filter_map(|entry| sole_occurrence(text, &entry.transcribed).map(|start| (start, entry)))
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return Ok(None);
    }
    matches.sort_by_key(|(start, _)| *start);
    for pair in matches.windows(2) {
        let (start, entry) = pair[0];
        let (next_start, next) = pair[1];
        if next_start < start + entry.transcribed.len() {
            bail!(
                "source emendations {} and {} overlap at byte {next_start}",
                entry.id,
                next.id
            );
        }
    }
    let mut emended = String::with_capacity(text.len());
    let mut applied = Vec::with_capacity(matches.len());
    let mut cursor = 0;
    let mut line = 1;
    for (start, entry) in matches {
        let preceding = &text[cursor..start];
        line += line_terminators(preceding);
        emended.push_str(preceding);
        let byte_start = emended.len();
        emended.push_str(&entry.emended);
        applied.push(AppliedEmendation {
            id: entry.id.clone(),
            transcribed: entry.transcribed.clone(),
            emended: entry.emended.clone(),
            byte_start,
            byte_end: emended.len(),
            line_start: line,
            line_end: line,
        });
        cursor = start + entry.transcribed.len();
    }
    emended.push_str(&text[cursor..]);
    Ok(Some((emended, applied)))
}

/// Count of `\n`, `\r\n` and bare `\r` terminators, the line rule of the
/// AAT coordinate system.
fn line_terminators(text: &str) -> usize {
    text.matches('\n').count() + text.matches('\r').count() - text.matches("\r\n").count()
}

/// Byte offset of the only occurrence of `needle`, counting overlapping
/// occurrences.
fn sole_occurrence(text: &str, needle: &str) -> Option<usize> {
    let first = text.find(needle)?;
    let after_first = first + text[first..].chars().next().map_or(1, char::len_utf8);
    text[after_first..].find(needle).is_none().then_some(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, transcribed: &str, emended: &str) -> Emendation {
        Emendation {
            id: id.to_owned(),
            work_id: "000000_0".to_owned(),
            indexed_path: "cards/000000/files/0.zip::a.txt".to_owned(),
            transcribed: transcribed.to_owned(),
            emended: emended.to_owned(),
        }
    }

    fn table(entries: &str) -> Result<Vec<Emendation>> {
        parse_table(&format!("emendations_version = 1\n{entries}"))
    }

    #[test]
    fn embedded_table_is_valid() {
        let entries = emendation_table().unwrap();
        assert!(!entries.is_empty(), "embedded table has entries");
    }

    #[test]
    fn applies_an_entry_that_occurs_once_and_records_its_extent() {
        let entries = [entry("a", "向｜《むか》", "｜向《むか》")];
        let (text, applied) = emend_with(&entries, "前\n一番向｜《むか》う\n後")
            .unwrap()
            .unwrap();
        assert_eq!(text, "前\n一番｜向《むか》う\n後");
        let start = "前\n一番".len();
        assert_eq!(
            applied,
            [AppliedEmendation {
                id: "a".to_owned(),
                transcribed: "向｜《むか》".to_owned(),
                emended: "｜向《むか》".to_owned(),
                byte_start: start,
                byte_end: start + "｜向《むか》".len(),
                line_start: 2,
                line_end: 2,
            }]
        );
        assert_eq!(&text[start..start + "｜向《むか》".len()], "｜向《むか》");
    }

    #[test]
    fn extents_account_for_earlier_length_changes() {
        let entries = [entry("late", "cd", "C"), entry("early", "a", "AAA")];
        let (text, applied) = emend_with(&entries, "a\nb\ncd").unwrap().unwrap();
        assert_eq!(text, "AAA\nb\nC");
        let extents = applied
            .iter()
            .map(|item| {
                (
                    item.id.as_str(),
                    item.byte_start,
                    item.byte_end,
                    item.line_start,
                    item.line_end,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(extents, [("early", 0, 3, 1, 1), ("late", 6, 7, 3, 3)]);
    }

    #[test]
    fn line_numbers_count_lf_crlf_and_bare_cr_terminators() {
        let entries = [entry("a", "x", "y")];
        let (_, applied) = emend_with(&entries, "a\r\nb\rc\nx").unwrap().unwrap();
        assert_eq!((applied[0].line_start, applied[0].line_end), (4, 4));
    }

    #[test]
    fn does_not_apply_an_absent_or_repeated_notation() {
        let entries = [entry("a", "ab", "AB")];
        for text in ["xyz", "ab ab"] {
            assert_eq!(emend_with(&entries, text).unwrap(), None, "{text}");
        }
        let overlapping = [entry("a", "aa", "b")];
        assert_eq!(emend_with(&overlapping, "aaa").unwrap(), None);
    }

    #[test]
    fn emended_text_is_a_fixed_point() {
        let entries = [entry("a", "向｜《むか》", "｜向《むか》")];
        let (once, _) = emend_with(&entries, "向｜《むか》").unwrap().unwrap();
        assert_eq!(emend_with(&entries, &once).unwrap(), None);
    }

    #[test]
    fn overlapping_matches_apply_neither_entry() {
        let entries = [entry("a", "abc", "x"), entry("b", "cd", "y")];
        let error = emend_with(&entries, "abcd").unwrap_err();
        assert!(error.to_string().contains("overlap"), "{error}");
    }

    #[test]
    fn rejects_invalid_tables() {
        let valid = "[[emendation]]\nid = \"a-1\"\nwork_id = \"w\"\nindexed_path = \"p\"\ntranscribed = \"x\"\nemended = \"y\"\n";
        assert_eq!(table(valid).unwrap().len(), 1);
        for (case, entries) in [
            ("duplicate", format!("{valid}{valid}")),
            ("pattern", valid.replace("a-1", "A_1")),
            ("trailing hyphen", valid.replace("a-1", "a-")),
            (
                "empty",
                valid.replace("transcribed = \"x\"", "transcribed = \"\""),
            ),
            ("equal", valid.replace("emended = \"y\"", "emended = \"x\"")),
            (
                "containing",
                valid.replace("emended = \"y\"", "emended = \"axb\""),
            ),
            (
                "lf transcribed",
                valid.replace("transcribed = \"x\"", "transcribed = \"x\\n\""),
            ),
            (
                "cr emended",
                valid.replace("emended = \"y\"", "emended = \"y\\r\""),
            ),
        ] {
            assert!(table(&entries).is_err(), "{case}");
        }
        assert!(parse_table("emendations_version = 2\n").is_err(), "version");
    }
}
