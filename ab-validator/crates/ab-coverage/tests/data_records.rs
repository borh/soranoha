use std::{collections::BTreeSet, fs, path::Path};

use serde_json::Value;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

/// Every file in `data/` and the schema it must satisfy.
const RECORDS: &[(&str, &str)] = &[
    (
        "aozora-syntax-coverage.toml",
        "records/aozora-syntax-coverage.schema.json",
    ),
    (
        "historical-parser-comparison.toml",
        "records/historical-parser-comparison.schema.json",
    ),
    (
        "aat-oracle-cases.toml",
        "records/aat-oracle-cases.schema.json",
    ),
    (
        "fidelity-requirements.toml",
        "records/fidelity-requirements.schema.json",
    ),
    (
        "aozora-source-inventory-allowlist.toml",
        "records/aozora-source-inventory-allowlist.schema.json",
    ),
    (
        "adapter-fidelity-notes.toml",
        "records/adapter-fidelity-notes.schema.json",
    ),
    (
        "aat-upstream-observations.toml",
        "records/aat-upstream-observations.schema.json",
    ),
    (
        "residue-decisions.toml",
        "records/residue-decisions.schema.json",
    ),
    (
        "source-emendations.toml",
        "records/source-emendations.schema.json",
    ),
    (
        "aat-to-parser-ir-mapping-v1.json",
        "aat-parser-ir-mapping.schema.json",
    ),
    (
        "aat-to-parser-ir-mapping-v2.json",
        "aat-parser-ir-mapping.schema.json",
    ),
];

fn read_record(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap();
    if path
        .extension()
        .is_some_and(|extension| extension == "toml")
    {
        serde_json::to_value(toml::from_str::<toml::Value>(&text).unwrap()).unwrap()
    } else {
        serde_json::from_str(&text).unwrap()
    }
}

#[test]
fn data_directory_holds_only_schema_backed_records() {
    let present = fs::read_dir(Path::new(ROOT).join("data"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<BTreeSet<_>>();
    let declared = RECORDS
        .iter()
        .map(|(record, _)| (*record).to_owned())
        .collect();
    assert_eq!(present, declared);
}

#[test]
fn every_data_record_satisfies_its_schema() {
    let mut failures = Vec::new();
    for (record, schema) in RECORDS {
        let schema: Value = serde_json::from_str(
            &fs::read_to_string(Path::new(ROOT).join("schemas").join(schema)).unwrap(),
        )
        .unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        let document = read_record(&Path::new(ROOT).join("data").join(record));
        failures.extend(
            validator
                .iter_errors(&document)
                .take(5)
                .map(|error| format!("{record} {}: {error}", error.instance_path())),
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
