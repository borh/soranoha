//! The two JIS X 0213 character mapping tables derive from
//! `data/jisx0213-2004-std.txt`, which supplies both the source mappings and
//! their licensing terms.
//!
//! Single-codepoint cells appear in `data/jisx0213-2004.tsv`, whereas
//! characters requiring Unicode combining sequences appear in
//! `data/jisx0213-combo.tsv`. Discrepancies between committed files and their
//! derivations are reported by file name. Setting `AB_WRITE_JISX0213_TABLES=1`
//! updates both files directly.
use std::{
    env, fs,
    path::{Path, PathBuf},
};

const SOURCE: &str = "jisx0213-2004-std.txt";
const SINGLE: &str = "jisx0213-2004.tsv";
const COMBO: &str = "jisx0213-combo.tsv";
const WRITE_ENV: &str = "AB_WRITE_JISX0213_TABLES";

const SINGLE_HEADER: &str = "\
# JIS X 0213:2004 men-ku-ten mapping table — single-codepoint cells only.
#
# Generated from jisx0213-2004-std.txt by tests/jisx0213_tables.rs.
# Do not edit: regenerate instead.
#
# Terms: inherited from the source table, whose header grants free use,
# modification and distribution (Project X0213, earthian@tama.or.jp, I'O).
#
# Filtered to gaiji territory: plane 1 cells marked [2000] or [2004] in the source,
# representing characters introduced by JIS X 0213 beyond JIS X 0208 (第3水準), plus all
# plane 2 cells (第4水準). Cells mapped to combining sequences appear in
# jisx0213-combo.tsv.
#
# Format: plane<TAB>row<TAB>cell<TAB>codepoint_hex
";

const COMBO_HEADER: &str = "\
# JIS X 0213:2004 multi-codepoint cells.
#
# Generated from jisx0213-2004-std.txt by tests/jisx0213_tables.rs.
# Do not edit: regenerate instead.
#
# Terms: inherited from the source table, whose header grants free use,
# modification and distribution (Project X0213, earthian@tama.or.jp, I'O).
#
# The 25 plane 1 第3水準 cells whose canonical mapping is a Unicode combining
# sequence: a base character followed by a combining diacritic rather than an
# individual scalar value. Because a combining sequence exceeds a single Unicode
# scalar, these entries are recorded in a separate string map. Queries across both
# mappings return uniform `&str` results through the lookup interface.
#
# Format: plane<TAB>row<TAB>cell<TAB>comma-separated codepoint hex
# (e.g. `1\\t4\\t87\\t304B,309A` = か + 半濁点 = か゚)
";

/// The combining cells fall into five orthographic groups unlabelled in the
/// source; each heading is keyed to the initial cell of that group.
const COMBO_GROUPS: [(&str, &str); 5] = [
    (
        "1\t4\t87",
        "半濁点付きひらがな (か゚ き゚ く゚ け゚ こ゚ — Ainu / Tōhoku notation)",
    ),
    (
        "1\t5\t87",
        "半濁点付きカタカナ (カ゚ キ゚ ク゚ ケ゚ コ゚ セ゚ ツ゚ ト゚ — Ainu)",
    ),
    (
        "1\t6\t88",
        "Katakana phonetic extensions: ㇷ゚ (small katakana pu)",
    ),
    ("1\t11\t36", "拡張ラテン文字 + 結合濁点 (アクセント記号)"),
    ("1\t11\t69", "声調記号の組合せ (IPA tone)"),
];

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("data")
}

/// `3-RRCC` is plane 1 and `4-RRCC` plane 2, both in GL coordinates: row and
/// cell are the hex bytes less 0x20, so 3-2121 is plane 1 row 1 cell 1.
fn parse_cell(token: &str) -> Option<(u8, u8, u8)> {
    let bytes = token.as_bytes();
    if bytes.len() != 6 || bytes[1] != b'-' || !matches!(bytes[0], b'3' | b'4') {
        return None;
    }
    let hex = &token[2..];
    if !hex
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte))
    {
        return None;
    }
    let plane = if bytes[0] == b'3' { 1 } else { 2 };
    let row = u8::from_str_radix(&hex[..2], 16).ok()? - 0x20;
    let cell = u8::from_str_radix(&hex[2..], 16).ok()? - 0x20;
    Some((plane, row, cell))
}

/// Compatibility cells specify halfwidth forms in their mapping columns and
/// fullwidth equivalents in explanatory notes. Because Aozora Bunko texts are
/// typeset fullwidth, the fullwidth note takes priority whenever present. This
/// represents the sole interpretive derivation applied to the source table.
fn fullwidth_note(note: &str) -> Option<&str> {
    let start = note.find("Fullwidth: U+")? + "Fullwidth: U+".len();
    let rest = &note[start..];
    let end = rest
        .find(|ch: char| !(ch.is_ascii_digit() || ('A'..='F').contains(&ch)))
        .unwrap_or(rest.len());
    (end > 0).then_some(&rest[..end])
}

fn strip_leading_zeros(hex: &str) -> &str {
    hex.trim_start_matches('0')
}

fn zero_pad(hex: &str) -> String {
    format!("{hex:0>4}")
}

/// Single-codepoint and combining-sequence rows, arranged in source order.
/// Every data line is accounted for: any line that is neither a mapped cell nor
/// a reserved entry produces an error, preventing undetected truncation or
/// format changes in the source data.
fn read_cells(source: &str) -> (Vec<String>, Vec<String>) {
    let mut single = Vec::new();
    let mut combo = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let number = index + 1;
        if line.starts_with("##") || line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        let cell = parse_cell(fields[0]);
        let (Some((plane, row, column)), true) = (cell, fields.len() >= 2) else {
            panic!("{SOURCE}:{number}: not a JIS X 0213 cell line: {line:?}");
        };
        let note = fields[2..].join("\t");
        let mapping = fields[1].trim();
        if mapping.is_empty() {
            // Unassigned cells designated `<reserved>` in the standard.
            assert!(
                note.contains("<reserved>"),
                "{SOURCE}:{number}: empty mapping is not reserved: {line:?}"
            );
            continue;
        }
        let Some(points) = mapping.strip_prefix("U+") else {
            panic!("{SOURCE}:{number}: mapping is not a codepoint: {mapping:?}");
        };
        if plane == 1 && !note.contains("[2000]") && !note.contains("[2004]") {
            continue;
        }
        if let Some(fullwidth) = fullwidth_note(&note) {
            single.push(format!(
                "{plane}\t{row}\t{column}\t{}",
                strip_leading_zeros(fullwidth)
            ));
            continue;
        }
        let points: Vec<&str> = points.split('+').collect();
        if points.len() == 1 {
            single.push(format!(
                "{plane}\t{row}\t{column}\t{}",
                strip_leading_zeros(points[0])
            ));
        } else {
            // Combining sequence values are zero-padded to four hexadecimal digits; single scalar entries are not.
            let padded: Vec<String> = points.iter().map(|point| zero_pad(point)).collect();
            combo.push(format!("{plane}\t{row}\t{column}\t{}", padded.join(",")));
        }
    }
    (single, combo)
}

fn render(header: &str, rows: &[String]) -> String {
    format!("{header}{}\n", rows.join("\n"))
}

/// Renders the combining table with a blank line and a descriptive heading
/// before each orthographic group. An unreferenced group heading produces an
/// error, ensuring that each label accurately prefaces its corresponding rows.
fn render_combo(rows: &[String]) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut unused: Vec<&str> = COMBO_GROUPS.iter().map(|(key, _)| *key).collect();
    for row in rows {
        let key = row.rsplit_once('\t').map_or(row.as_str(), |(key, _)| key);
        if let Some((_, label)) = COMBO_GROUPS.iter().find(|(group, _)| *group == key) {
            unused.retain(|candidate| *candidate != key);
            lines.push(String::new());
            lines.push(format!("# {label}"));
        }
        lines.push(row.clone());
    }
    assert!(
        unused.is_empty(),
        "group headings specify cell coordinates absent from the source data: {unused:?}"
    );
    format!("{COMBO_HEADER}{}\n", lines.join("\n"))
}

#[test]
fn committed_tables_match_the_derivation_from_the_source_table() {
    let data = data_dir();
    let source = fs::read_to_string(data.join(SOURCE)).expect("read source table");
    let (single, combo) = read_cells(&source);
    let outputs = [
        (data.join(SINGLE), render(SINGLE_HEADER, &single)),
        (data.join(COMBO), render_combo(&combo)),
    ];

    if env::var(WRITE_ENV).is_ok_and(|value| value == "1") {
        for (path, text) in &outputs {
            fs::write(path, text).expect("write table");
        }
        return;
    }

    let stale: Vec<String> = outputs
        .iter()
        .filter(|(path, text)| fs::read_to_string(path).expect("read table") != *text)
        .map(|(path, _)| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(
        stale.is_empty(),
        "stale generated tables: {}; regenerate with {WRITE_ENV}=1",
        stale.join(", ")
    );
}

#[test]
fn cell_tokens_and_fullwidth_notes_parse_source_notations() {
    assert_eq!(Some((1, 1, 1)), parse_cell("3-2121"));
    assert_eq!(Some((2, 94, 94)), parse_cell("4-7E7E"));
    assert_eq!(None, parse_cell("5-2121"));
    assert_eq!(None, parse_cell("3-212"));
    assert_eq!(None, parse_cell("3-212g"));
    assert_eq!(Some("FF0C"), fullwidth_note("# COMMA\tFullwidth: U+FF0C"));
    assert_eq!(None, fullwidth_note("# COMMA\tWindows: U+FFE3"));
    assert_eq!("304B,309A", zero_pad("304B") + "," + &zero_pad("309A"));
    assert_eq!("E6", strip_leading_zeros("00E6"));
}
