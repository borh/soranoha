//! A detached source note can supply a diacritic for an earlier Latin word.

use ab_aat::aat_json_from_bytes;
use serde_json::Value;

fn nodes_of_kind<'a>(value: &'a Value, kind: &str, found: &mut Vec<&'a Value>) {
    match value {
        Value::Object(object) => {
            if value["kind"] == kind && value.get("span").is_some() {
                found.push(value);
            }
            for child in object.values() {
                nodes_of_kind(child, kind, found);
            }
        }
        Value::Array(values) => {
            for child in values {
                nodes_of_kind(child, kind, found);
            }
        }
        _ => {}
    }
}

#[test]
fn quoted_dot_note_binds_the_earlier_latin_word() {
    let marker = "［＃「t」は下点付き、182-6］";
    let source = format!("題\n作者\n\n鳥のアラビア語 tair. {marker}咽喉\n\n底本：本\n");
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let mut accents = Vec::new();
    nodes_of_kind(&aat, "supplied-diacritic", &mut accents);
    assert_eq!(accents.len(), 1, "{aat}");
    let accent = accents[0];
    let target = source.find("tair").unwrap();
    let annotation = source.find(marker).unwrap();
    assert_eq!(accent["text"], "ṭair");
    assert_eq!(accent["source"], "tair");
    assert_eq!(accent["span"]["byte_start"], target);
    assert_eq!(accent["span"]["byte_end"], target + 4);
    assert_eq!(
        accent["interpretation_marker_spans"][0]["byte_start"],
        annotation
    );
    assert_eq!(
        accent["interpretation_marker_spans"][0]["byte_end"],
        annotation + marker.len()
    );
    assert!(!aat.to_string().contains("unknown-notation"), "{aat}");
}
