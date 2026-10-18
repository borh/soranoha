//! Physical line declarations retain target, placement, and source locator.

use serde_json::{Value, json};

fn find_bosen(value: &Value) -> Option<&Value> {
    match value {
        Value::Object(object) => {
            if value["kind"] == "style" && value["style_type"] == "bosen" {
                return Some(value);
            }
            object.values().find_map(find_bosen)
        }
        Value::Array(values) => values.iter().find_map(find_bosen),
        _ => None,
    }
}

#[test]
fn physical_lines_keep_exact_targets_placement_and_supplied_locator() {
    for (source, target, position, locator) in [
        (
            "Khayyam（Umar Khaiyam［＃「Kh」に下線］）",
            "Kh",
            "below",
            None,
        ),
        (
            "hove［＃「v」は下線（_）付き、181-表組2行目］",
            "v",
            "below",
            Some("181-表組2行目"),
        ),
        (
            "P = 0.07693694［＃「P = 0.07693694」は上線（￣）付き］",
            "P = 0.07693694",
            "above",
            None,
        ),
    ] {
        let aat: Value =
            serde_json::from_slice(&ab_aat::aat_json_from_bytes(source.as_bytes()).unwrap())
                .unwrap();
        let style = find_bosen(&aat["blocks"]).expect("physical line becomes bosen");
        let marker_start = source.find("［＃").unwrap();
        let marker_end =
            marker_start + source[marker_start..].find('］').unwrap() + '］'.len_utf8();
        let target_start = source[..marker_start].rfind(target).unwrap();

        assert_eq!(style["decoration"]["kind"], "傍線");
        assert_eq!(style["decoration"]["position"], position);
        assert_eq!(
            style["decoration"].get("source_locator"),
            locator.map(Value::from).as_ref()
        );
        assert_eq!(style["span"]["byte_start"], target_start);
        assert_eq!(style["span"]["byte_end"], target_start + target.len());
        assert_eq!(style["content"][0]["value"], target);
        assert_eq!(
            style["interpretation_marker_spans"],
            json!([{"byte_start": marker_start, "byte_end": marker_end,
                    "line_start": 1, "line_end": 1}])
        );
        assert!(aat["meta"]["parse_complete"].as_bool().unwrap());
    }
}
