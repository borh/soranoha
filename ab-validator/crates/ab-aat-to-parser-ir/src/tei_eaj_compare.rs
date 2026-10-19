//! Comparison of ABC-generated TEI with the pinned TEI-EAJ `aozora_tei`
//! checkout: per-file structural counts, body base text with ruby readings and
//! notes removed, work-id derivation from TEI-EAJ file names, counterpart
//! discovery, and the Melos, all-work and workset-export reports.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use regex::Regex;
use roxmltree::{Document, Node};
use serde_json::{Value, json};

const SPAN_FEATURE_TYPES: [&str; 4] = ["ruby", "rb", "rt", "rp"];
const COUNTERPART_TABLE_LIMIT: usize = 100;
const REPORT_FEATURES: [&str; 22] = [
    "p",
    "s",
    "note",
    "div",
    "head",
    "ruby",
    "rb",
    "rt",
    "rp",
    "persName",
    "rs",
    "placeName",
    "roleName",
    "said",
    "listPerson",
    "person",
    "revisionDesc",
    "editorialDecl",
    "sourceDesc",
    "front",
    "body",
    "back",
];
const CORPUS_TABLE_FEATURES: [&str; 8] = [
    "p",
    "ruby",
    "note",
    "persName",
    "placeName",
    "roleName",
    "said",
    "listPerson",
];
pub const MELOS_WORK_ID: &str = "1567";
const FIRST_DIFFERENCE_CONTEXT: usize = 36;

/// Structural profile of one TEI file.
#[derive(Debug, Clone)]
pub struct FileFeatures {
    pub title: Option<String>,
    pub aozora_work_id: Option<String>,
    pub counts: BTreeMap<String, u64>,
    /// Body text with `rt`, `rp` and `note` content removed and all
    /// whitespace dropped.
    pub body_base_text: String,
}

impl FileFeatures {
    pub fn count(&self, feature: &str) -> u64 {
        self.counts.get(feature).copied().unwrap_or(0)
    }

    pub fn body_base_text_length(&self) -> usize {
        self.body_base_text.chars().count()
    }
}

#[derive(Debug, Clone)]
pub struct TeiEajRecord {
    pub path: PathBuf,
    pub relpath: String,
    pub state: Option<String>,
    pub level: Option<String>,
    pub work_id: Option<String>,
    pub work_id_method: &'static str,
    pub title: Option<String>,
    pub features: FileFeatures,
}

#[derive(Debug, Clone)]
pub struct Counterpart {
    pub work_id: String,
    pub path: String,
    pub title: Option<String>,
    features: Option<FileFeatures>,
}

#[derive(Debug, Clone)]
pub struct FirstDifference {
    pub index: usize,
    pub abc: String,
    pub tei_eaj: String,
}

#[derive(Debug, Clone)]
pub struct FeatureCount {
    pub abc: u64,
    pub tei_eaj: u64,
}

#[derive(Debug, Clone)]
pub struct Comparison {
    pub relpath: String,
    pub work_id: Option<String>,
    pub work_id_method: &'static str,
    pub state: Option<String>,
    pub level: Option<String>,
    pub title: Option<String>,
    pub base_text_equal: bool,
    pub base_text_relation: &'static str,
    pub base_text_length_delta: i64,
    pub abc_body_base_text_length: usize,
    pub tei_eaj_body_base_text_length: usize,
    pub first_difference: Option<FirstDifference>,
    pub feature_counts: BTreeMap<&'static str, FeatureCount>,
}

#[derive(Debug, Clone)]
pub enum AllWorkRow {
    Compared {
        abc_path: String,
        comparison: Comparison,
    },
    Uncompared {
        record: TeiEajRecord,
        missing_reason: &'static str,
    },
}

impl AllWorkRow {
    pub fn work_id(&self) -> Option<&str> {
        match self {
            AllWorkRow::Compared { comparison, .. } => comparison.work_id.as_deref(),
            AllWorkRow::Uncompared { record, .. } => record.work_id.as_deref(),
        }
    }

    pub fn relpath(&self) -> &str {
        match self {
            AllWorkRow::Compared { comparison, .. } => &comparison.relpath,
            AllWorkRow::Uncompared { record, .. } => &record.relpath,
        }
    }

    pub fn missing_reason(&self) -> Option<&'static str> {
        match self {
            AllWorkRow::Compared { .. } => None,
            AllWorkRow::Uncompared { missing_reason, .. } => Some(missing_reason),
        }
    }

    pub fn comparison(&self) -> Option<&Comparison> {
        match self {
            AllWorkRow::Compared { comparison, .. } => Some(comparison),
            AllWorkRow::Uncompared { .. } => None,
        }
    }

    fn work_id_method(&self) -> &'static str {
        match self {
            AllWorkRow::Compared { comparison, .. } => comparison.work_id_method,
            AllWorkRow::Uncompared { record, .. } => record.work_id_method,
        }
    }

    fn state(&self) -> Option<&str> {
        match self {
            AllWorkRow::Compared { comparison, .. } => comparison.state.as_deref(),
            AllWorkRow::Uncompared { record, .. } => record.state.as_deref(),
        }
    }

    fn level(&self) -> Option<&str> {
        match self {
            AllWorkRow::Compared { comparison, .. } => comparison.level.as_deref(),
            AllWorkRow::Uncompared { record, .. } => record.level.as_deref(),
        }
    }

    fn title(&self) -> Option<&str> {
        match self {
            AllWorkRow::Compared { comparison, .. } => comparison.title.as_deref(),
            AllWorkRow::Uncompared { record, .. } => record.title.as_deref(),
        }
    }

    fn abc_path(&self) -> Option<&str> {
        match self {
            AllWorkRow::Compared { abc_path, .. } => Some(abc_path),
            AllWorkRow::Uncompared { .. } => None,
        }
    }

    fn tei_eaj_body_base_text_length(&self) -> usize {
        match self {
            AllWorkRow::Compared { comparison, .. } => comparison.tei_eaj_body_base_text_length,
            AllWorkRow::Uncompared { record, .. } => record.features.body_base_text_length(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GroupCount {
    pub state: Option<String>,
    pub level: Option<String>,
    pub count: u64,
}

#[derive(Debug, Clone)]
pub struct FeaturePrevalence {
    pub feature: &'static str,
    pub files_with_feature: u64,
    pub max_count: u64,
}

#[derive(Debug, Clone)]
pub struct AllWorkReport {
    pub abc_tei_specs: Vec<String>,
    pub abc_tei_dirs: Vec<String>,
    /// Mapping of work ids to ABC TEI paths, ordered lexicographically by work id.
    pub abc_counterparts: BTreeMap<String, String>,
    pub tei_eaj_root: String,
    pub tei_eaj_source_rev: Option<String>,
    pub records: Vec<TeiEajRecord>,
    pub rows: Vec<AllWorkRow>,
}

impl AllWorkReport {
    pub fn tei_eaj_file_count(&self) -> usize {
        self.records.len()
    }

    pub fn tei_eaj_work_id_count(&self) -> usize {
        self.records
            .iter()
            .filter_map(|record| record.work_id.as_deref())
            .collect::<BTreeSet<_>>()
            .len()
    }

    pub fn compared_rows(&self) -> impl Iterator<Item = &Comparison> {
        self.rows.iter().filter_map(AllWorkRow::comparison)
    }

    pub fn compared_file_count(&self) -> usize {
        self.compared_rows().count()
    }

    pub fn missing_counterpart_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| row.missing_reason() == Some("missing_abc_counterpart"))
            .count()
    }

    pub fn no_work_id_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| row.missing_reason() == Some("no_tei_eaj_work_id"))
            .count()
    }

    pub fn uncompared_file_count(&self) -> usize {
        self.missing_counterpart_count() + self.no_work_id_count()
    }

    pub fn base_text_equal_count(&self) -> usize {
        self.compared_rows()
            .filter(|row| row.base_text_equal)
            .count()
    }

    pub fn base_text_mismatch_count(&self) -> usize {
        self.compared_rows()
            .filter(|row| !row.base_text_equal)
            .count()
    }

    pub fn base_text_relation_counts(&self) -> BTreeMap<&'static str, u64> {
        let mut counts = BTreeMap::new();
        for row in self.compared_rows() {
            *counts.entry(row.base_text_relation).or_insert(0) += 1;
        }
        counts
    }
}

#[derive(Debug, Clone)]
pub struct MelosReport {
    pub abc_path: String,
    pub tei_eaj_root: String,
    pub tei_eaj_source_rev: Option<String>,
    pub abc_features: FileFeatures,
    pub tei_eaj_file_count: usize,
    pub records: Vec<TeiEajRecord>,
    pub melos_comparisons: Vec<Comparison>,
    pub complete_melos_file_count: usize,
    pub draft_melos_file_count: usize,
}

pub fn canonical_work_id(value: &str) -> Option<String> {
    let text = value.trim();
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let normalized = text.trim_start_matches('0');
    Some(if normalized.is_empty() {
        "0".to_owned()
    } else {
        normalized.to_owned()
    })
}

pub fn normalize_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn remove_ws(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_whitespace()).collect()
}

fn skip_base_text_element(node: Node<'_, '_>) -> bool {
    let name = node.tag_name().name();
    if matches!(name, "rt" | "rp" | "note") {
        return true;
    }
    name == "span"
        && (matches!(node.attribute("type"), Some("rt" | "rp"))
            || node.attribute("rend") == Some("notes"))
}

fn element_text(node: Node<'_, '_>, skip: fn(Node<'_, '_>) -> bool, out: &mut String) {
    if node.is_text() {
        out.push_str(node.text().unwrap_or_default());
        return;
    }
    if node.is_element() && skip(node) {
        return;
    }
    for child in node.children() {
        element_text(child, skip, out);
    }
}

fn full_text(node: Node<'_, '_>) -> String {
    let mut out = String::new();
    element_text(node, |_| false, &mut out);
    out
}

fn base_text(node: Node<'_, '_>) -> String {
    let mut out = String::new();
    element_text(node, skip_base_text_element, &mut out);
    out
}

fn first_element<'a, 'input>(
    document: &'a Document<'input>,
    local: &str,
) -> Option<Node<'a, 'input>> {
    document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == local)
}

pub fn analyze_xml(xml: &str) -> Result<FileFeatures> {
    let document = Document::parse(xml)?;
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    for element in document.descendants().filter(Node::is_element) {
        *counts
            .entry(element.tag_name().name().to_owned())
            .or_insert(0) += 1;
        if element.tag_name().name() == "span"
            && let Some(span_type) = element.attribute("type")
            && SPAN_FEATURE_TYPES.contains(&span_type)
        {
            *counts.entry(span_type.to_owned()).or_insert(0) += 1;
        }
    }
    let title = first_element(&document, "title").map(|node| normalize_ws(&full_text(node)));
    let aozora_work_id = document
        .descendants()
        .find(|node| {
            node.is_element()
                && node.tag_name().name() == "idno"
                && node.attribute("type") == Some("aozora-work-id")
        })
        .and_then(|node| canonical_work_id(&full_text(node)));
    let body_base_text = first_element(&document, "body")
        .map(|body| remove_ws(&base_text(body)))
        .unwrap_or_default();
    Ok(FileFeatures {
        title,
        aozora_work_id,
        counts,
        body_base_text,
    })
}

pub fn analyze_file(path: &Path) -> Result<FileFeatures> {
    let xml = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read TEI {}", path.display()))?;
    analyze_xml(&xml).with_context(|| format!("failed to parse TEI {}", path.display()))
}

fn state_from_relpath(relpath: &str) -> Option<String> {
    let parts: Vec<&str> = relpath.split('/').collect();
    if parts.len() >= 2 && parts[0] == "data" {
        return Some(parts[1].to_owned());
    }
    parts.first().map(|part| (*part).to_owned())
}

fn level_from_relpath(relpath: &str) -> Option<String> {
    let level = Regex::new(r"tei_lib_lv([2-5])").expect("static regex");
    level
        .captures(relpath)
        .map(|captures| format!("Level {}", &captures[1]))
}

fn unique_title_work_id(
    title: Option<&str>,
    title_to_work_ids: Option<&BTreeMap<String, BTreeSet<String>>>,
) -> Option<String> {
    let title = title.filter(|title| !title.is_empty())?;
    let work_ids = title_to_work_ids?.get(&normalize_ws(title))?;
    if work_ids.len() == 1 {
        work_ids.iter().next().cloned()
    } else {
        None
    }
}

fn file_stem(relpath: &str) -> &str {
    let name = relpath.rsplit('/').next().unwrap_or(relpath);
    match name.rfind('.') {
        Some(0) | None => name,
        Some(index) => &name[..index],
    }
}

/// `(work_id, method)` derived from a TEI-EAJ file name. When a file is
/// identified by a file id rather than a work id, the counterpart title index
/// resolves the work id provided that exactly one counterpart shares that title.
pub fn tei_eaj_work_identity(
    relpath: &str,
    title: Option<&str>,
    title_to_work_ids: Option<&BTreeMap<String, BTreeSet<String>>>,
) -> (Option<String>, &'static str) {
    let stem = file_stem(relpath);
    let patterns = [
        (r"^([0-9]+)_tei$", "filename_work_id"),
        (r"^([0-9]+)_header.*$", "filename_header_work_id"),
        (r"^([0-9]+)-[0-9]+_tei$", "filename_work_id_variant"),
        (r"^([0-9]+)_[0-9]+$", "filename_work_id_file_id"),
        (r"^([0-9]{3,})$", "filename_numeric_work_id"),
    ];
    for (pattern, method) in patterns {
        let regex = Regex::new(pattern).expect("static regex");
        if let Some(captures) = regex.captures(stem) {
            let candidate = canonical_work_id(&captures[1]);
            if method == "filename_work_id"
                && let Some(title_work_id) = unique_title_work_id(title, title_to_work_ids)
                && Some(&title_work_id) != candidate.as_ref()
            {
                return (Some(title_work_id), "title_unique_index_correction");
            }
            return (candidate, method);
        }
    }
    (None, "no_work_id")
}

fn xml_files_under(root: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    if !root.is_dir() {
        return Ok(paths);
    }
    for entry in walkdir::WalkDir::new(root) {
        let entry = entry.with_context(|| format!("failed to scan {}", root.display()))?;
        if entry.file_type().is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "xml")
        {
            paths.push(entry.into_path());
        }
    }
    paths.sort();
    Ok(paths)
}

fn relative_posix(path: &Path, root: &Path) -> Result<String> {
    let relative = path
        .strip_prefix(root)
        .with_context(|| format!("{} is outside {}", path.display(), root.display()))?;
    Ok(relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/"))
}

pub fn discover_tei_eaj_files(
    source_root: &Path,
    title_to_work_ids: Option<&BTreeMap<String, BTreeSet<String>>>,
) -> Result<Vec<TeiEajRecord>> {
    let mut records = Vec::new();
    for path in xml_files_under(&source_root.join("data"))? {
        let relpath = relative_posix(&path, source_root)?;
        let features = analyze_file(&path)?;
        let (work_id, work_id_method) =
            tei_eaj_work_identity(&relpath, features.title.as_deref(), title_to_work_ids);
        records.push(TeiEajRecord {
            state: state_from_relpath(&relpath),
            level: level_from_relpath(&relpath),
            work_id,
            work_id_method,
            title: features.title.clone(),
            relpath,
            path,
            features,
        });
    }
    Ok(records)
}

fn is_melos_record(record: &TeiEajRecord) -> bool {
    let basename = record.relpath.rsplit('/').next().unwrap_or(&record.relpath);
    basename.starts_with(MELOS_WORK_ID)
        || record
            .title
            .as_deref()
            .is_some_and(|title| title.contains("走れメロス"))
}

fn first_text_difference(left: &str, right: &str) -> Option<FirstDifference> {
    if left == right {
        return None;
    }
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    let limit = left.len().min(right.len());
    let index = (0..limit).find(|&i| left[i] != right[i]).unwrap_or(limit);
    let start = index.saturating_sub(FIRST_DIFFERENCE_CONTEXT);
    let end = index + FIRST_DIFFERENCE_CONTEXT;
    let window = |text: &[char]| {
        text[start.min(text.len())..end.min(text.len())]
            .iter()
            .collect::<String>()
    };
    Some(FirstDifference {
        index,
        abc: window(&left),
        tei_eaj: window(&right),
    })
}

fn base_text_relation(abc_text: &str, tei_eaj_text: &str) -> &'static str {
    if abc_text == tei_eaj_text {
        "equal"
    } else if !tei_eaj_text.is_empty() && abc_text.contains(tei_eaj_text) {
        "tei_eaj_subset_of_abc"
    } else if !abc_text.is_empty() && tei_eaj_text.contains(abc_text) {
        "abc_subset_of_tei_eaj"
    } else {
        "mismatch"
    }
}

fn compare_features(abc_features: &FileFeatures, record: &TeiEajRecord) -> Comparison {
    let abc_text = &abc_features.body_base_text;
    let other_text = &record.features.body_base_text;
    let abc_len = abc_features.body_base_text_length();
    let other_len = record.features.body_base_text_length();
    Comparison {
        relpath: record.relpath.clone(),
        work_id: record.work_id.clone(),
        work_id_method: record.work_id_method,
        state: record.state.clone(),
        level: record.level.clone(),
        title: record.title.clone(),
        base_text_equal: abc_text == other_text,
        base_text_relation: base_text_relation(abc_text, other_text),
        base_text_length_delta: abc_len as i64 - other_len as i64,
        abc_body_base_text_length: abc_len,
        tei_eaj_body_base_text_length: other_len,
        first_difference: first_text_difference(abc_text, other_text),
        feature_counts: REPORT_FEATURES
            .iter()
            .map(|feature| {
                (
                    *feature,
                    FeatureCount {
                        abc: abc_features.count(feature),
                        tei_eaj: record.features.count(feature),
                    },
                )
            })
            .collect(),
    }
}

fn counterpart_from_spec(spec: &str) -> Result<Option<Counterpart>> {
    if let Some((work_id, path)) = spec.split_once('=') {
        let Some(work_id) = canonical_work_id(work_id) else {
            return Ok(None);
        };
        let features = analyze_file(Path::new(path))?;
        return Ok(Some(Counterpart {
            work_id,
            path: path.to_owned(),
            title: features.title.clone(),
            features: Some(features),
        }));
    }
    let features = analyze_file(Path::new(spec))?;
    Ok(features.aozora_work_id.clone().map(|work_id| Counterpart {
        work_id,
        path: spec.to_owned(),
        title: features.title.clone(),
        features: Some(features),
    }))
}

fn inspection_index_counterparts(directory: &Path) -> Result<BTreeMap<String, Counterpart>> {
    let mut counterparts = BTreeMap::new();
    let Some(index_path) = directory
        .parent()
        .map(|parent| parent.join("inspection-index.tsv"))
        .filter(|candidate| candidate.exists())
    else {
        return Ok(counterparts);
    };
    let text = std::fs::read_to_string(&index_path)
        .with_context(|| format!("failed to read {}", index_path.display()))?;
    let mut lines = text.lines();
    let Some(header) = lines.next() else {
        return Ok(counterparts);
    };
    let columns: Vec<&str> = header.split('\t').collect();
    let column = |name: &str| columns.iter().position(|column| *column == name);
    let (Some(work_id_column), Some(title_column)) = (column("work_id"), column("title")) else {
        return Ok(counterparts);
    };
    for line in lines {
        let fields: Vec<&str> = line.split('\t').collect();
        let Some(work_id) = fields
            .get(work_id_column)
            .and_then(|value| canonical_work_id(value))
        else {
            continue;
        };
        let padded: u64 = work_id.parse().unwrap_or(u64::MAX);
        let path = directory.join(format!("{padded:06}.xml"));
        if !path.exists() {
            continue;
        }
        let title = fields.get(title_column).map(|value| normalize_ws(value));
        counterparts.insert(
            work_id.clone(),
            Counterpart {
                work_id,
                path: path.to_string_lossy().into_owned(),
                title,
                features: None,
            },
        );
    }
    Ok(counterparts)
}

pub fn discover_abc_counterparts(
    abc_tei_specs: &[String],
    abc_tei_dirs: &[String],
) -> Result<BTreeMap<String, Counterpart>> {
    let mut counterparts = BTreeMap::new();
    for spec in abc_tei_specs {
        if let Some(counterpart) = counterpart_from_spec(spec)? {
            counterparts.insert(counterpart.work_id.clone(), counterpart);
        }
    }
    for directory in abc_tei_dirs {
        let directory = Path::new(directory);
        if !directory.exists() {
            continue;
        }
        let indexed = inspection_index_counterparts(directory)?;
        if !indexed.is_empty() {
            for (work_id, counterpart) in indexed {
                counterparts.entry(work_id).or_insert(counterpart);
            }
            continue;
        }
        for path in xml_files_under(directory)? {
            let Ok(xml) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(features) = analyze_xml(&xml) else {
                continue;
            };
            let Some(work_id) = features.aozora_work_id.clone() else {
                continue;
            };
            counterparts.entry(work_id.clone()).or_insert(Counterpart {
                work_id,
                path: path.to_string_lossy().into_owned(),
                title: features.title.clone(),
                features: Some(features),
            });
        }
    }
    Ok(counterparts)
}

pub fn counterpart_title_index(
    counterparts: &BTreeMap<String, Counterpart>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut index: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (work_id, counterpart) in counterparts {
        let title = counterpart
            .title
            .as_deref()
            .filter(|title| !title.is_empty())
            .or_else(|| {
                counterpart
                    .features
                    .as_ref()
                    .and_then(|features| features.title.as_deref())
                    .filter(|title| !title.is_empty())
            });
        if let Some(title) = title {
            index
                .entry(normalize_ws(title))
                .or_default()
                .insert(work_id.clone());
        }
    }
    index
}

fn ensure_features(counterpart: &mut Counterpart) -> Result<&FileFeatures> {
    if counterpart.features.is_none() {
        counterpart.features = Some(analyze_file(Path::new(&counterpart.path))?);
    }
    Ok(counterpart.features.as_ref().expect("features present"))
}

pub fn build_all_work_report(
    abc_tei_specs: &[String],
    abc_tei_dirs: &[String],
    source_root: &Path,
    source_rev: Option<&str>,
) -> Result<AllWorkReport> {
    let mut counterparts = discover_abc_counterparts(abc_tei_specs, abc_tei_dirs)?;
    let title_index = counterpart_title_index(&counterparts);
    let records = discover_tei_eaj_files(source_root, Some(&title_index))?;
    let mut rows = Vec::with_capacity(records.len());
    for record in &records {
        let Some(work_id) = record.work_id.as_deref() else {
            rows.push(AllWorkRow::Uncompared {
                record: record.clone(),
                missing_reason: "no_tei_eaj_work_id",
            });
            continue;
        };
        let Some(counterpart) = counterparts.get_mut(work_id) else {
            rows.push(AllWorkRow::Uncompared {
                record: record.clone(),
                missing_reason: "missing_abc_counterpart",
            });
            continue;
        };
        let abc_path = counterpart.path.clone();
        let comparison = compare_features(ensure_features(counterpart)?, record);
        rows.push(AllWorkRow::Compared {
            abc_path,
            comparison,
        });
    }
    Ok(AllWorkReport {
        abc_tei_specs: abc_tei_specs.to_vec(),
        abc_tei_dirs: abc_tei_dirs.to_vec(),
        abc_counterparts: counterparts
            .into_iter()
            .map(|(work_id, counterpart)| (work_id, counterpart.path))
            .collect(),
        tei_eaj_root: source_root.to_string_lossy().into_owned(),
        tei_eaj_source_rev: source_rev.map(str::to_owned),
        records,
        rows,
    })
}

pub fn resolve_melos_abc_path(
    abc_tei: Option<&str>,
    abc_tei_specs: &[String],
    abc_tei_dirs: &[String],
) -> Result<String> {
    if let Some(abc_tei) = abc_tei {
        return Ok(abc_tei.to_owned());
    }
    let counterparts = discover_abc_counterparts(abc_tei_specs, abc_tei_dirs)?;
    match counterparts.get(MELOS_WORK_ID) {
        Some(counterpart) => Ok(counterpart.path.clone()),
        None => bail!(
            "Melos comparison requires an ABC TEI counterpart for work id {MELOS_WORK_ID}; \
             pass --abc-melos, --abc-tei {MELOS_WORK_ID}=PATH, or --abc-tei-dir GENERATED_TEI_ROOT"
        ),
    }
}

pub fn build_melos_report(
    abc_tei: Option<&str>,
    source_root: &Path,
    source_rev: Option<&str>,
    abc_tei_specs: &[String],
    abc_tei_dirs: &[String],
) -> Result<MelosReport> {
    let abc_path = resolve_melos_abc_path(abc_tei, abc_tei_specs, abc_tei_dirs)?;
    let abc_features = analyze_file(Path::new(&abc_path))?;
    let records = discover_tei_eaj_files(source_root, None)?;
    Ok(melos_report_from_records(
        abc_path,
        abc_features,
        source_root,
        source_rev,
        records,
    ))
}

pub fn melos_report_from_records(
    abc_path: String,
    abc_features: FileFeatures,
    source_root: &Path,
    source_rev: Option<&str>,
    records: Vec<TeiEajRecord>,
) -> MelosReport {
    let melos: Vec<&TeiEajRecord> = records
        .iter()
        .filter(|record| is_melos_record(record))
        .collect();
    let state_count = |state: &str| {
        melos
            .iter()
            .filter(|record| record.state.as_deref() == Some(state))
            .count()
    };
    MelosReport {
        tei_eaj_root: source_root.to_string_lossy().into_owned(),
        tei_eaj_source_rev: source_rev.map(str::to_owned),
        tei_eaj_file_count: records.len(),
        complete_melos_file_count: state_count("complete"),
        draft_melos_file_count: state_count("draft"),
        melos_comparisons: melos
            .iter()
            .map(|record| compare_features(&abc_features, record))
            .collect(),
        abc_path,
        abc_features,
        records,
    }
}

pub fn group_counts(records: &[TeiEajRecord]) -> Vec<GroupCount> {
    let mut grouped: BTreeMap<(Option<String>, Option<String>), u64> = BTreeMap::new();
    for record in records {
        *grouped
            .entry((record.state.clone(), record.level.clone()))
            .or_insert(0) += 1;
    }
    let mut groups: Vec<GroupCount> = grouped
        .into_iter()
        .map(|((state, level), count)| GroupCount {
            state,
            level,
            count,
        })
        .collect();
    groups.sort_by(|a, b| {
        (
            a.state.as_deref().unwrap_or(""),
            a.level.as_deref().unwrap_or(""),
        )
            .cmp(&(
                b.state.as_deref().unwrap_or(""),
                b.level.as_deref().unwrap_or(""),
            ))
    });
    groups
}

pub fn feature_prevalence(records: &[TeiEajRecord]) -> Vec<FeaturePrevalence> {
    REPORT_FEATURES
        .iter()
        .map(|feature| {
            let counts = records.iter().map(|record| record.features.count(feature));
            FeaturePrevalence {
                feature,
                files_with_feature: counts.clone().filter(|count| *count > 0).count() as u64,
                max_count: counts.max().unwrap_or(0),
            }
        })
        .collect()
}

fn markdown_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut lines = vec![
        format!("| {} |", headers.join(" | ")),
        format!(
            "| {} |",
            headers
                .iter()
                .map(|_| "---")
                .collect::<Vec<_>>()
                .join(" | ")
        ),
    ];
    for row in rows {
        lines.push(format!("| {} |", row.join(" | ")));
    }
    lines.join("\n")
}

fn cell<T: ToString>(value: Option<T>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn compact_path(path: Option<&str>) -> String {
    match path {
        Some(path) if !path.is_empty() => format!("`{path}`"),
        _ => String::new(),
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn group_rows(records: &[TeiEajRecord]) -> Vec<Vec<String>> {
    group_counts(records)
        .into_iter()
        .map(|group| {
            vec![
                cell(group.state),
                cell(group.level),
                group.count.to_string(),
            ]
        })
        .collect()
}

fn corpus_table(records: &[TeiEajRecord]) -> String {
    let headers: Vec<&str> = ["Path", "State", "Level", "Title"]
        .into_iter()
        .chain(CORPUS_TABLE_FEATURES)
        .collect();
    let rows: Vec<Vec<String>> = records
        .iter()
        .map(|record| {
            let mut row = vec![
                format!("`{}`", record.relpath),
                cell(record.state.as_deref()),
                cell(record.level.as_deref()),
                cell(record.title.as_deref()),
            ];
            row.extend(
                CORPUS_TABLE_FEATURES
                    .iter()
                    .map(|feature| record.features.count(feature).to_string()),
            );
            row
        })
        .collect();
    markdown_table(&headers, &rows)
}

pub fn render_melos_markdown(report: &MelosReport) -> String {
    let mut lines = vec![
        "# TEI-EAJ Aozora Melos Comparison Report".to_owned(),
        String::new(),
        "## Inputs".to_owned(),
        String::new(),
        format!("- ABC TEI: `{}`", report.abc_path),
        format!("- TEI-EAJ source root: `{}`", report.tei_eaj_root),
    ];
    if let Some(rev) = report
        .tei_eaj_source_rev
        .as_deref()
        .filter(|rev| !rev.is_empty())
    {
        lines.push(format!("- TEI-EAJ source revision: `{rev}`"));
    }
    lines.extend([
        format!("- TEI-EAJ XML files scanned: {}", report.tei_eaj_file_count),
        format!(
            "- TEI-EAJ Melos files found: {}",
            report.melos_comparisons.len()
        ),
        format!(
            "- Complete Melos files: {}",
            report.complete_melos_file_count
        ),
        format!("- Draft Melos files: {}", report.draft_melos_file_count),
        String::new(),
    ]);
    if report.draft_melos_file_count == 0 {
        lines.extend([
            "No draft Melos TEI files were found in the pinned TEI-EAJ checkout.".to_owned(),
            "Draft coverage below is therefore corpus-wide structural context, not".to_owned(),
            "a literal draft Melos text comparison.".to_owned(),
            String::new(),
        ]);
    }
    lines.extend([
        "## Level/State Coverage".to_owned(),
        String::new(),
        markdown_table(&["State", "Level", "Files"], &group_rows(&report.records)),
        String::new(),
        "## Melos File Comparisons".to_owned(),
        String::new(),
    ]);
    let melos_rows: Vec<Vec<String>> = report
        .melos_comparisons
        .iter()
        .map(|comparison| {
            let counts = &comparison.feature_counts;
            vec![
                format!("`{}`", comparison.relpath),
                cell(comparison.state.as_deref()),
                cell(comparison.level.as_deref()),
                yes_no(comparison.base_text_equal).to_owned(),
                comparison.abc_body_base_text_length.to_string(),
                comparison.tei_eaj_body_base_text_length.to_string(),
                counts["p"].abc.to_string(),
                counts["p"].tei_eaj.to_string(),
                counts["note"].abc.to_string(),
                counts["note"].tei_eaj.to_string(),
                counts["persName"].tei_eaj.to_string(),
                counts["said"].tei_eaj.to_string(),
            ]
        })
        .collect();
    lines.extend([
        markdown_table(
            &[
                "TEI-EAJ file",
                "State",
                "Level",
                "Base text equal",
                "ABC chars",
                "TEI-EAJ chars",
                "ABC p",
                "TEI-EAJ p",
                "ABC note",
                "TEI-EAJ note",
                "TEI-EAJ persName",
                "TEI-EAJ said",
            ],
            &melos_rows,
        ),
        String::new(),
        "### First Text Differences".to_owned(),
        String::new(),
    ]);
    for comparison in &report.melos_comparisons {
        lines.push(format!("- `{}`:", comparison.relpath));
        match &comparison.first_difference {
            None => lines
                .push("  base text matches after whitespace and ruby-reading removal.".to_owned()),
            Some(diff) => {
                lines.push(format!(
                    "  first difference at normalized base-text offset {}.",
                    diff.index
                ));
                lines.push(format!("  ABC window: `{}`", diff.abc));
                lines.push(format!("  TEI-EAJ window: `{}`", diff.tei_eaj));
            }
        }
    }
    lines.push(String::new());
    let prevalence_rows: Vec<Vec<String>> = feature_prevalence(&report.records)
        .into_iter()
        .map(|row| {
            vec![
                row.feature.to_owned(),
                row.files_with_feature.to_string(),
                row.max_count.to_string(),
                report.abc_features.count(row.feature).to_string(),
            ]
        })
        .collect();
    lines.extend([
        "## Feature Prevalence Across All TEI-EAJ Files".to_owned(),
        String::new(),
        markdown_table(
            &[
                "Feature",
                "Files With Feature",
                "Max Count In One File",
                "ABC Count",
            ],
            &prevalence_rows,
        ),
        String::new(),
        "## All TEI-EAJ XML Files".to_owned(),
        String::new(),
        corpus_table(&report.records),
        String::new(),
        "## Interpretation".to_owned(),
        String::new(),
        "- The Melos comparison examines every Melos TEI file in the pinned TEI-EAJ checkout.".to_owned(),
        "- Finished and draft TEI-EAJ files are both included in the corpus-wide structural profile.".to_owned(),
        "- Paragraph and source-note/source-attribution structure remain the immediate Level 3 gap for ABC.".to_owned(),
        "- TEI-EAJ Level 4 entity, role, place, and speech markup should remain an enrichment comparison track unless an editorial enrichment layer is defined in ABC.".to_owned(),
    ]);
    lines.join("\n") + "\n"
}

fn joined_paths(paths: &[String]) -> String {
    if paths.is_empty() {
        return "(none)".to_owned();
    }
    paths
        .iter()
        .map(|path| compact_path(Some(path)))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn render_all_work_markdown(report: &AllWorkReport) -> String {
    let mut lines = vec![
        "# TEI-EAJ Aozora All-Work Comparison Report".to_owned(),
        String::new(),
        "## Inputs".to_owned(),
        String::new(),
        format!("- TEI-EAJ source root: `{}`", report.tei_eaj_root),
    ];
    if let Some(rev) = report
        .tei_eaj_source_rev
        .as_deref()
        .filter(|rev| !rev.is_empty())
    {
        lines.push(format!("- TEI-EAJ source revision: `{rev}`"));
    }
    lines.extend([
        format!("- ABC TEI specs: {}", joined_paths(&report.abc_tei_specs)),
        format!(
            "- ABC TEI directories: {}",
            joined_paths(&report.abc_tei_dirs)
        ),
        format!(
            "- ABC counterpart works discovered: {}",
            report.abc_counterparts.len()
        ),
        String::new(),
        "## Summary".to_owned(),
        String::new(),
        format!(
            "- TEI-EAJ XML files scanned: {}",
            report.tei_eaj_file_count()
        ),
        format!(
            "- TEI-EAJ candidate work IDs: {}",
            report.tei_eaj_work_id_count()
        ),
        format!("- Compared TEI-EAJ files: {}", report.compared_file_count()),
        format!("- Base-text matches: {}", report.base_text_equal_count()),
        format!(
            "- Base-text mismatches: {}",
            report.base_text_mismatch_count()
        ),
        format!(
            "- Missing ABC counterparts: {}",
            report.missing_counterpart_count()
        ),
        format!(
            "- TEI-EAJ files without candidate work IDs: {}",
            report.no_work_id_count()
        ),
        format!(
            "- Uncompared TEI-EAJ files: {}",
            report.uncompared_file_count()
        ),
        String::new(),
        "The absence of a counterpart indicates that no ABC-generated TEI carrying the normalized"
            .to_owned(),
        "Aozora work ID was located in the supplied paths. Files without candidate work".to_owned(),
        "IDs require separate source identification before automated comparison is possible."
            .to_owned(),
        String::new(),
        "## Level/State Coverage".to_owned(),
        String::new(),
        markdown_table(&["State", "Level", "Files"], &group_rows(&report.records)),
        String::new(),
        "## ABC Counterparts".to_owned(),
        String::new(),
    ]);
    let counterpart_rows: Vec<Vec<String>> = report
        .abc_counterparts
        .iter()
        .map(|(work_id, path)| vec![work_id.clone(), compact_path(Some(path))])
        .collect();
    if counterpart_rows.len() > COUNTERPART_TABLE_LIMIT {
        lines.push(format!(
            "ABC counterpart table omitted: {} discovered counterparts exceeds display limit {COUNTERPART_TABLE_LIMIT}.",
            counterpart_rows.len()
        ));
    } else if counterpart_rows.is_empty() {
        lines.push("No ABC counterparts were discovered.".to_owned());
    } else {
        lines.push(markdown_table(&["Work ID", "ABC TEI"], &counterpart_rows));
    }
    lines.extend([
        String::new(),
        "## All TEI-EAJ File Comparisons".to_owned(),
        String::new(),
    ]);
    let comparison_rows: Vec<Vec<String>> = report
        .rows
        .iter()
        .map(|row| {
            let comparison = row.comparison();
            let count = |feature: &str, abc: bool| {
                cell(comparison.map(|comparison| {
                    let counts = &comparison.feature_counts[feature];
                    if abc { counts.abc } else { counts.tei_eaj }
                }))
            };
            vec![
                cell(row.work_id()),
                compact_path(Some(row.relpath())),
                cell(row.state()),
                cell(row.level()),
                cell(row.title()),
                compact_path(row.abc_path()),
                cell(comparison.map(|comparison| yes_no(comparison.base_text_equal))),
                cell(comparison.map(|comparison| comparison.abc_body_base_text_length)),
                row.tei_eaj_body_base_text_length().to_string(),
                count("p", true),
                count("p", false),
                count("note", true),
                count("note", false),
                cell(row.missing_reason()),
            ]
        })
        .collect();
    let prevalence_rows: Vec<Vec<String>> = feature_prevalence(&report.records)
        .into_iter()
        .map(|row| {
            vec![
                row.feature.to_owned(),
                row.files_with_feature.to_string(),
                row.max_count.to_string(),
            ]
        })
        .collect();
    lines.extend([
        markdown_table(
            &[
                "Work ID",
                "TEI-EAJ file",
                "State",
                "Level",
                "Title",
                "ABC TEI",
                "Base text equal",
                "ABC chars",
                "TEI-EAJ chars",
                "ABC p",
                "TEI-EAJ p",
                "ABC note",
                "TEI-EAJ note",
                "Missing reason",
            ],
            &comparison_rows,
        ),
        String::new(),
        "## Feature Prevalence Across All TEI-EAJ Files".to_owned(),
        String::new(),
        markdown_table(
            &["Feature", "Files With Feature", "Max Count In One File"],
            &prevalence_rows,
        ),
        String::new(),
        "## Interpretation".to_owned(),
        String::new(),
        "- This report enumerates every XML file in the pinned TEI-EAJ `aozora_tei` checkout, including complete and draft files.".to_owned(),
        "- Rows with an ABC counterpart compare normalized body base text after whitespace and ruby-reading removal.".to_owned(),
        "- Rows without an ABC counterpart define the immediate materialization backlog, not a parser text mismatch.".to_owned(),
        "- Level 3 paragraph and source-note gaps should be resolved before asserting TEI-EAJ Level 3 conformance for ABC-generated prose.".to_owned(),
    ]);
    lines.join("\n") + "\n"
}

fn first_difference_json(diff: Option<&FirstDifference>) -> Value {
    match diff {
        None => Value::Null,
        Some(diff) => json!({
            "index": diff.index,
            "abc": diff.abc,
            "tei_eaj": diff.tei_eaj,
        }),
    }
}

fn workset_export_row(row: &AllWorkRow) -> Value {
    let comparison = row.comparison();
    let count = |feature: &str, abc: bool| {
        comparison.map(|comparison| {
            let counts = &comparison.feature_counts[feature];
            if abc { counts.abc } else { counts.tei_eaj }
        })
    };
    let base_text_relation = match row {
        AllWorkRow::Compared { comparison, .. } => Some(comparison.base_text_relation),
        AllWorkRow::Uncompared {
            missing_reason: "missing_abc_counterpart",
            ..
        } => Some("missing_abc_counterpart"),
        AllWorkRow::Uncompared { .. } => None,
    };
    json!({
        "work_id": row.work_id(),
        "work_id_method": row.work_id_method(),
        "tei_eaj_file": row.relpath(),
        "state": row.state(),
        "level": row.level(),
        "title": row.title(),
        "abc_tei": row.abc_path(),
        "comparison_status": row.missing_reason().unwrap_or("compared"),
        "base_text_equal": comparison.map(|comparison| comparison.base_text_equal),
        "base_text_relation": base_text_relation,
        "base_text_length_delta": comparison.map(|comparison| comparison.base_text_length_delta),
        "abc_body_base_text_length": comparison.map(|comparison| comparison.abc_body_base_text_length),
        "tei_eaj_body_base_text_length": row.tei_eaj_body_base_text_length(),
        "abc_p_count": count("p", true),
        "tei_eaj_p_count": count("p", false),
        "abc_note_count": count("note", true),
        "tei_eaj_note_count": count("note", false),
        "first_difference": first_difference_json(comparison.and_then(|comparison| comparison.first_difference.as_ref())),
    })
}

/// Work ids in numeric order. Canonical work ids omit leading zeros, so
/// numeric sorting proceeds first by string length and then lexicographically.
fn numeric_sorted(work_ids: BTreeSet<&str>) -> Vec<&str> {
    let mut sorted: Vec<&str> = work_ids.into_iter().collect();
    sorted.sort_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
    sorted
}

pub fn workset_export(report: &AllWorkReport) -> Value {
    let candidate_work_ids =
        numeric_sorted(report.rows.iter().filter_map(AllWorkRow::work_id).collect());
    let missing_work_ids = numeric_sorted(
        report
            .rows
            .iter()
            .filter(|row| row.missing_reason() == Some("missing_abc_counterpart"))
            .filter_map(AllWorkRow::work_id)
            .collect(),
    );
    let no_work_id_files: Vec<&str> = report
        .rows
        .iter()
        .filter(|row| row.missing_reason() == Some("no_tei_eaj_work_id"))
        .map(AllWorkRow::relpath)
        .collect();
    json!({
        "schema_version": "tei-eaj-aozora-workset-export-v1",
        "tei_eaj_source": {
            "root": report.tei_eaj_root,
            "revision": report.tei_eaj_source_rev,
        },
        "abc_inputs": {
            "tei_specs": report.abc_tei_specs,
            "tei_dirs": report.abc_tei_dirs,
            "counterparts": report
                .abc_counterparts
                .iter()
                .map(|(work_id, path)| json!({"work_id": work_id, "path": path}))
                .collect::<Vec<_>>(),
        },
        "summary": {
            "tei_eaj_file_count": report.tei_eaj_file_count(),
            "tei_eaj_work_id_count": report.tei_eaj_work_id_count(),
            "abc_counterpart_count": report.abc_counterparts.len(),
            "compared_file_count": report.compared_file_count(),
            "missing_counterpart_count": report.missing_counterpart_count(),
            "no_work_id_count": report.no_work_id_count(),
            "uncompared_file_count": report.uncompared_file_count(),
            "base_text_equal_count": report.base_text_equal_count(),
            "base_text_mismatch_count": report.base_text_mismatch_count(),
            "base_text_relation_counts": report.base_text_relation_counts(),
        },
        "candidate_work_ids": candidate_work_ids,
        "missing_abc_counterpart_work_ids": missing_work_ids,
        "no_work_id_files": no_work_id_files,
        "files": report.rows.iter().map(workset_export_row).collect::<Vec<_>>(),
    })
}

/// Pretty-printed JSON with sorted keys and a trailing newline, matching the
/// serialization format used for the workset export and the probe-attached workset.
pub fn render_sorted_json(value: Value) -> Result<String> {
    Ok(crate::canonical_json::to_canonical_json_pretty(value)? + "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tei(title: &str, body: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<TEI xmlns=\"http://www.tei-c.org/ns/1.0\">\n  <teiHeader><fileDesc><titleStmt><title>{title}</title></titleStmt><sourceDesc><p>source</p></sourceDesc></fileDesc></teiHeader>\n  <text><body>{body}</body></text>\n</TEI>\n"
        )
    }

    fn write(root: &Path, relpath: &str, text: &str) -> PathBuf {
        let path = root.join(relpath);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        path
    }

    fn write_tei(root: &Path, relpath: &str, title: &str, body: &str) -> PathBuf {
        write(root, relpath, &tei(title, body))
    }

    fn with_work_id(title: &str, work_id: &str, body: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<TEI xmlns=\"http://www.tei-c.org/ns/1.0\">\n  <teiHeader><fileDesc><titleStmt><title>{title}</title></titleStmt><publicationStmt><idno type=\"aozora-work-id\">{work_id}</idno></publicationStmt></fileDesc></teiHeader>\n  <text><body>{body}</body></text>\n</TEI>\n"
        )
    }

    fn spec(work_id: &str, path: &Path) -> Vec<String> {
        vec![format!("{work_id}={}", path.display())]
    }

    fn dirs(path: &Path) -> Vec<String> {
        vec![path.to_string_lossy().into_owned()]
    }

    #[test]
    fn body_base_text_drops_ruby_readings_notes_and_whitespace_but_counts_them() {
        let namespaced = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<TEI xmlns=\"http://www.tei-c.org/ns/1.0\" xmlns:eaj=\"http://www.example.org/ns/ejaTEI\">\n<text><body><p>お<eaj:ruby><eaj:rb>世嗣</eaj:rb><eaj:rp>（</eaj:rp><eaj:rt>よつぎ</eaj:rt><eaj:rp>）</eaj:rp></eaj:ruby>を</p></body></text></TEI>";
        let features = analyze_xml(namespaced).unwrap();
        assert_eq!("お世嗣を", features.body_base_text);
        assert_eq!(
            [1, 1, 1, 2],
            [
                features.count("ruby"),
                features.count("rb"),
                features.count("rt"),
                features.count("rp")
            ]
        );

        let spans = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<TEI xmlns=\"http://www.tei-c.org/ns/1.0\">\n<text><body><p>高瀬舟 <span type=\"rp\">（</span><span type=\"rt\">たかせぶね</span><span type=\"rp\">）</span><span rend=\"notes\">［＃校訂注］</span>は小舟である<note>source note</note></p></body></text></TEI>";
        let features = analyze_xml(spans).unwrap();
        assert_eq!("高瀬舟は小舟である", features.body_base_text);
        assert_eq!(9, features.body_base_text_length());
        assert_eq!(1, features.count("rt"));
        assert_eq!(2, features.count("rp"));
        assert_eq!(1, features.count("note"));
        assert_eq!(1, features.count("TEI"));
    }

    #[test]
    fn title_and_work_id_are_read_from_the_header() {
        let features = analyze_xml(&with_work_id("走れ\n  メロス", "001567", "<p>x</p>")).unwrap();
        assert_eq!(Some("走れ メロス"), features.title.as_deref());
        assert_eq!(Some("1567"), features.aozora_work_id.as_deref());
        assert_eq!(None, analyze_xml(&tei("x", "")).unwrap().aozora_work_id);
        assert_eq!(Some("0"), canonical_work_id(" 000 ").as_deref());
        assert_eq!(None, canonical_work_id("15a"));
    }

    #[test]
    fn work_ids_are_derived_from_tei_eaj_file_names() {
        let cases = [
            (
                "data/complete/tei_lib_lv4/1567_tei.xml",
                Some("1567"),
                "filename_work_id",
            ),
            (
                "data/complete/tei_lib_lv4/1567_header_updated.xml",
                Some("1567"),
                "filename_header_work_id",
            ),
            (
                "data/complete/tei_lib_lv4/104_15099.xml",
                Some("104"),
                "filename_work_id_file_id",
            ),
            (
                "data/draft/tei_lib_lv4/4244-3_tei.xml",
                Some("4244"),
                "filename_work_id_variant",
            ),
            (
                "data/draft/tei_lib_lv2/0456.xml",
                Some("456"),
                "filename_numeric_work_id",
            ),
            ("data/draft/tei_lib_lv2/01.xml", None, "no_work_id"),
            ("data/etc/Curriculum vitae.xml", None, "no_work_id"),
        ];
        for (relpath, work_id, method) in cases {
            let (derived, derived_method) = tei_eaj_work_identity(relpath, None, None);
            assert_eq!(
                (work_id, method),
                (derived.as_deref(), derived_method),
                "{relpath}"
            );
        }
    }

    #[test]
    fn discovery_records_state_and_level_in_path_order() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write_tei(
            root,
            "data/complete/tei_lib_lv4/1567_tei.xml",
            "走れメロス",
            "<p>one</p>",
        );
        write_tei(
            root,
            "data/draft/tei_lib_lv3/86_tei.xml",
            "羅生門",
            "<p>two</p>",
        );
        write_tei(root, "data/etc/misc.xml", "Other", "<p>three</p>");
        write_tei(root, "src/ignored.xml", "Ignored", "<p>four</p>");

        let records = discover_tei_eaj_files(root, None).unwrap();
        let summary: Vec<String> = records
            .iter()
            .map(|record| {
                format!(
                    "{} {} {} {}",
                    record.relpath,
                    record.state.as_deref().unwrap_or("-"),
                    record.level.as_deref().unwrap_or("-"),
                    record.title.as_deref().unwrap_or("-"),
                )
            })
            .collect();
        assert_eq!(
            vec![
                "data/complete/tei_lib_lv4/1567_tei.xml complete Level 4 走れメロス",
                "data/draft/tei_lib_lv3/86_tei.xml draft Level 3 羅生門",
                "data/etc/misc.xml etc - Other",
            ],
            summary
        );
    }

    #[test]
    fn melos_report_compares_every_melos_variant_and_flags_missing_drafts() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let abc = write_tei(
            root,
            "abc-melos.xml",
            "走れメロス",
            "<p>メロス<ruby><rb>邪智暴虐</rb><rt>じゃちぼうぎゃく</rt></ruby></p><note>source note</note>",
        );
        write_tei(
            root,
            "data/complete/tei_lib_lv4/1567_tei.xml",
            "走れメロス",
            "<p>メロス邪智暴虐</p>",
        );
        write_tei(
            root,
            "data/complete/tei_lib_lv4/1567_header_updated.xml",
            "走れメロス",
            "<p>メロス邪智暴虐</p>",
        );
        write_tei(
            root,
            "data/draft/tei_lib_lv3/86_tei.xml",
            "羅生門",
            "<p>羅生門</p>",
        );

        let report = build_melos_report(
            Some(&abc.to_string_lossy()),
            root,
            Some("probe-rev"),
            &[],
            &[],
        )
        .unwrap();
        let markdown = render_melos_markdown(&report);

        assert_eq!(3, report.tei_eaj_file_count);
        assert_eq!(2, report.melos_comparisons.len());
        assert_eq!(2, report.complete_melos_file_count);
        assert_eq!(0, report.draft_melos_file_count);
        assert!(markdown.contains("No draft Melos TEI files were found"));
        assert!(markdown.contains("- TEI-EAJ source revision: `probe-rev`"));
        assert_eq!(
            vec![
                "data/complete/tei_lib_lv4/1567_header_updated.xml",
                "data/complete/tei_lib_lv4/1567_tei.xml"
            ],
            report
                .melos_comparisons
                .iter()
                .map(|comparison| comparison.relpath.as_str())
                .collect::<Vec<_>>()
        );
        assert!(
            report
                .melos_comparisons
                .iter()
                .all(|comparison| comparison.base_text_equal)
        );
        assert!(markdown.contains("base text matches after whitespace and ruby-reading removal."));
    }

    #[test]
    fn melos_abc_path_resolves_from_a_counterpart_directory() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let abc_dir = root.join("generated").join("artifacts");
        let abc = write(
            &abc_dir,
            "works/melos/tei/tei.xml",
            &with_work_id("走れメロス", "001567", "<p>メロス</p>"),
        );
        write_tei(
            root,
            "data/complete/tei_lib_lv4/1567_tei.xml",
            "走れメロス",
            "<p>メロス</p>",
        );

        let report =
            build_melos_report(None, root, Some("probe-rev"), &[], &dirs(&abc_dir)).unwrap();
        assert_eq!(abc.to_string_lossy(), report.abc_path);
        assert_eq!(1, report.melos_comparisons.len());

        let error = build_melos_report(None, root, None, &[], &[]).unwrap_err();
        assert!(error.to_string().contains("work id 1567"));
    }

    #[test]
    fn all_work_report_compares_each_matching_variant_and_classifies_the_rest() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let abc = write_tei(
            root,
            "abc/001567.xml",
            "走れメロス",
            "<p>メロス邪智暴虐</p>",
        );
        write_tei(
            root,
            "data/complete/tei_lib_lv4/1567_tei.xml",
            "走れメロス",
            "<p>メロス邪智暴虐</p>",
        );
        write_tei(
            root,
            "data/draft/tei_lib_lv4/1567_header_updated.xml",
            "走れメロス",
            "<p>メロス別本文</p>",
        );
        write_tei(
            root,
            "data/complete/tei_lib_lv3/86_tei.xml",
            "二人小町",
            "<p>二人小町</p>",
        );
        write_tei(
            root,
            "data/draft/tei_lib_lv2/01.xml",
            "源氏物語 第1冊",
            "<p>桐壺</p>",
        );

        let report =
            build_all_work_report(&spec("1567", &abc), &[], root, Some("probe-rev")).unwrap();
        let markdown = render_all_work_markdown(&report);

        assert_eq!(4, report.tei_eaj_file_count());
        assert_eq!(1, report.abc_counterparts.len());
        assert_eq!(2, report.compared_file_count());
        assert_eq!(1, report.missing_counterpart_count());
        assert_eq!(1, report.no_work_id_count());
        assert_eq!(2, report.uncompared_file_count());
        assert_eq!(1, report.base_text_equal_count());
        assert_eq!(1, report.base_text_mismatch_count());
        let relations: Vec<(Option<&str>, &str, Option<&str>)> = report
            .rows
            .iter()
            .map(|row| {
                (
                    row.work_id(),
                    row.relpath(),
                    row.comparison()
                        .map(|comparison| comparison.base_text_relation),
                )
            })
            .collect();
        assert_eq!(
            vec![
                (Some("86"), "data/complete/tei_lib_lv3/86_tei.xml", None),
                (
                    Some("1567"),
                    "data/complete/tei_lib_lv4/1567_tei.xml",
                    Some("equal")
                ),
                (None, "data/draft/tei_lib_lv2/01.xml", None),
                (
                    Some("1567"),
                    "data/draft/tei_lib_lv4/1567_header_updated.xml",
                    Some("mismatch")
                ),
            ],
            relations
        );
        assert!(markdown.contains("- Compared TEI-EAJ files: 2"));
        assert!(markdown.contains("- Missing ABC counterparts: 1"));
        assert!(markdown.contains("- TEI-EAJ files without candidate work IDs: 1"));
        assert!(markdown.contains(
            "| 1567 | `data/complete/tei_lib_lv4/1567_tei.xml` | complete | Level 4 | 走れメロス |"
        ));
        assert!(markdown.contains("|  | `data/draft/tei_lib_lv2/01.xml` | draft | Level 2 | 源氏物語 第1冊 |  |  |  | 2 |  |  |  |  | no_tei_eaj_work_id |"));
    }

    #[test]
    fn counterpart_directories_yield_files_with_work_ids_and_skip_the_rest() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(
            root,
            "abc/melos.xml",
            &with_work_id("走れメロス", "001567", "<p>メロス</p>"),
        );
        write_tei(root, "abc/no-work-id.xml", "No ID", "<p>ignored</p>");
        write(root, "abc/broken.xml", "<TEI><body>");
        write_tei(
            root,
            "data/complete/tei_lib_lv4/1567_tei.xml",
            "走れメロス",
            "<p>メロス</p>",
        );

        let report = build_all_work_report(&[], &dirs(&root.join("abc")), root, None).unwrap();
        assert_eq!(1, report.abc_counterparts.len());
        assert_eq!(Some("1567"), report.rows[0].work_id());
        assert!(report.rows[0].comparison().unwrap().base_text_equal);
    }

    #[test]
    fn inspection_index_counterparts_correct_file_id_named_tei_eaj_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let abc_dir = root.join("publication").join("tei-by-work-id");
        write(
            root,
            "publication/inspection-index.tsv",
            "work_id\tsource_slug\ttitle\treading\tstatus\ttei_path\n\
             000104\tslug\t長崎小品\tながさきしょうひん\tpassed\tignored\n\
             002575\tslug\t鈴木三重吉宛書簡―明治三十九年\tすずき\tpassed\tignored\n\
             009999\tslug\tabsent\treading\tpassed\tignored\n",
        );
        write_tei(
            &abc_dir,
            "000104.xml",
            "長崎小品",
            "<p>薄暗き硝子戸棚の中。</p>",
        );
        write_tei(
            &abc_dir,
            "002575.xml",
            "鈴木三重吉宛書簡―明治三十九年",
            "<p>明治三十九年一月一日午前零時。</p>",
        );
        write_tei(
            root,
            "data/complete/tei_lib_lv3/15099_tei.xml",
            "長崎小品",
            "<p>薄暗き<span type='rp'>（</span><span type='rt'>うすぐらき</span><span type='rp'>）</span>硝子戸棚の中。</p>",
        );
        write_tei(
            root,
            "data/draft/tei_lib_lv4/2571_tei.xml",
            "鈴木三重吉宛書簡―明治三十九年",
            "<p>明治三十九年一月一日午前零時。</p>",
        );

        let report = build_all_work_report(&[], &dirs(&abc_dir), root, Some("probe-rev")).unwrap();
        let export = workset_export(&report);

        assert_eq!(2, report.abc_counterparts.len());
        let rows: BTreeMap<&str, &Value> = export["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| (row["tei_eaj_file"].as_str().unwrap(), row))
            .collect();
        let corrected = rows["data/complete/tei_lib_lv3/15099_tei.xml"];
        assert_eq!("104", corrected["work_id"]);
        assert_eq!("title_unique_index_correction", corrected["work_id_method"]);
        assert_eq!(
            "2575",
            rows["data/draft/tei_lib_lv4/2571_tei.xml"]["work_id"]
        );
        assert_eq!(2, export["summary"]["compared_file_count"]);
        assert_eq!(2, export["summary"]["base_text_equal_count"]);
    }

    #[test]
    fn base_text_relations_classify_subsets_and_report_the_first_difference() {
        assert_eq!("equal", base_text_relation("ab", "ab"));
        assert_eq!("tei_eaj_subset_of_abc", base_text_relation("abc", "b"));
        assert_eq!("abc_subset_of_tei_eaj", base_text_relation("b", "abc"));
        assert_eq!("mismatch", base_text_relation("", "abc"));
        assert_eq!("mismatch", base_text_relation("ax", "bx"));

        let diff = first_text_difference("メロス邪智暴虐（古伝説）", "メロス邪智暴虐").unwrap();
        assert_eq!(7, diff.index);
        assert_eq!("メロス邪智暴虐（古伝説）", diff.abc);
        assert_eq!("メロス邪智暴虐", diff.tei_eaj);
        let long_left: String = "あ".repeat(100) + "い";
        let long_right: String = "あ".repeat(100) + "う";
        let diff = first_text_difference(&long_left, &long_right).unwrap();
        assert_eq!(100, diff.index);
        assert_eq!(37, diff.abc.chars().count());
        assert!(first_text_difference("x", "x").is_none());

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let abc = write_tei(
            root,
            "abc/001567.xml",
            "走れメロス",
            "<p>メロス邪智暴虐（古伝説）</p>",
        );
        write_tei(
            root,
            "data/complete/tei_lib_lv4/1567_tei.xml",
            "走れメロス",
            "<p>メロス邪智暴虐</p>",
        );
        let report = build_all_work_report(&spec("1567", &abc), &[], root, None).unwrap();
        let comparison = report.rows[0].comparison().unwrap();
        assert!(!comparison.base_text_equal);
        assert_eq!("tei_eaj_subset_of_abc", comparison.base_text_relation);
        assert_eq!(5, comparison.base_text_length_delta);
        assert_eq!(
            BTreeMap::from([("tei_eaj_subset_of_abc", 1)]),
            report.base_text_relation_counts()
        );
    }

    #[test]
    fn markdown_omits_the_counterpart_table_above_the_display_limit() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let abc_dir = root.join("publication").join("tei-by-work-id");
        let mut index = String::from("work_id\tsource_slug\ttitle\treading\tstatus\ttei_path\n");
        for work_id in 1..=150 {
            let title = format!("作品{work_id}");
            index.push_str(&format!(
                "{work_id:06}\tslug\t{title}\treading\tpassed\tignored\n"
            ));
            write_tei(
                &abc_dir,
                &format!("{work_id:06}.xml"),
                &title,
                &format!("<p>{title}</p>"),
            );
        }
        write(root, "publication/inspection-index.tsv", &index);
        write_tei(
            root,
            "data/complete/tei_lib_lv3/1_tei.xml",
            "作品1",
            "<p>作品1</p>",
        );

        let report = build_all_work_report(&[], &dirs(&abc_dir), root, None).unwrap();
        let markdown = render_all_work_markdown(&report);
        assert!(markdown.contains("ABC counterpart works discovered: 150"));
        assert!(markdown.contains(
            "ABC counterpart table omitted: 150 discovered counterparts exceeds display limit 100."
        ));
        assert!(!markdown.contains("000150.xml"));
    }

    #[test]
    fn workset_export_orders_work_ids_numerically_and_states_each_file_status() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let abc = write_tei(
            root,
            "abc/001567.xml",
            "走れメロス",
            "<p>メロス邪智暴虐</p>",
        );
        write_tei(
            root,
            "data/complete/tei_lib_lv4/1567_tei.xml",
            "走れメロス",
            "<p>メロス邪智暴虐</p>",
        );
        write_tei(
            root,
            "data/complete/tei_lib_lv3/86_tei.xml",
            "二人小町",
            "<p>二人小町</p>",
        );
        write_tei(
            root,
            "data/draft/tei_lib_lv2/01.xml",
            "源氏物語 第1冊",
            "<p>桐壺</p>",
        );

        let report =
            build_all_work_report(&spec("1567", &abc), &[], root, Some("probe-rev")).unwrap();
        let export = workset_export(&report);
        let rendered: Value =
            serde_json::from_str(&render_sorted_json(export.clone()).unwrap()).unwrap();

        assert_eq!("tei-eaj-aozora-workset-export-v1", export["schema_version"]);
        assert_eq!(export, rendered);
        assert_eq!(
            json!({
                "tei_eaj_file_count": 3,
                "tei_eaj_work_id_count": 2,
                "abc_counterpart_count": 1,
                "compared_file_count": 1,
                "missing_counterpart_count": 1,
                "no_work_id_count": 1,
                "uncompared_file_count": 2,
                "base_text_equal_count": 1,
                "base_text_mismatch_count": 0,
                "base_text_relation_counts": {"equal": 1},
            }),
            export["summary"]
        );
        assert_eq!(json!(["86", "1567"]), export["candidate_work_ids"]);
        assert_eq!(json!(["86"]), export["missing_abc_counterpart_work_ids"]);
        assert_eq!(
            json!(["data/draft/tei_lib_lv2/01.xml"]),
            export["no_work_id_files"]
        );
        assert_eq!(
            json!({"root": root.to_string_lossy(), "revision": "probe-rev"}),
            export["tei_eaj_source"]
        );
        let files = export["files"].as_array().unwrap();
        assert_eq!(
            vec!["missing_abc_counterpart", "compared", "no_tei_eaj_work_id"],
            files
                .iter()
                .map(|row| row["comparison_status"].as_str().unwrap())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            vec![
                json!("missing_abc_counterpart"),
                json!("equal"),
                Value::Null
            ],
            files
                .iter()
                .map(|row| row["base_text_relation"].clone())
                .collect::<Vec<_>>()
        );
        // Paragraph tallies include both the header sourceDesc paragraph and the body paragraph.
        assert_eq!(
            (2, 2),
            (
                files[1]["abc_p_count"].as_u64().unwrap(),
                files[1]["tei_eaj_p_count"].as_u64().unwrap()
            )
        );
        assert_eq!(Value::Null, files[0]["abc_p_count"]);
    }
}
