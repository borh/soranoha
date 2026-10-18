//! Named edition readings retain their grouping and exact source identities.

use ab_aat::aat_json_from_bytes;
use serde_json::Value;

fn aat(source: &str) -> Value {
    serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap()
}

#[test]
fn shared_and_distinct_named_readings_are_semantic_apparatus() {
    for (source, reading_count, witness_counts) in [
        (
            "ことは［＃「ことは」は、『全集　第十九巻』（出版社2015年）と『全集　第十五卷』（出版社1968年）では「ことを」］",
            1,
            vec![2],
        ),
        (
            "、［＃「、」は、『全集　第十九巻』（出版社2015年）では「。」、『全集　第十五卷』（出版社1968年）では「、」］",
            2,
            vec![1, 1],
        ),
    ] {
        let value = aat(source);
        let apparatus = &value["blocks"][0]["content"][0];
        assert_eq!(apparatus["kind"], "text-apparatus", "{source}");
        let readings = apparatus["readings"].as_array().unwrap();
        assert_eq!(readings.len(), reading_count, "{source}");
        assert_eq!(
            readings
                .iter()
                .map(|reading| reading["witnesses"].as_array().unwrap().len())
                .collect::<Vec<_>>(),
            witness_counts,
            "{source}"
        );
        for reading in readings {
            for witness in reading["witnesses"].as_array().unwrap() {
                assert!(witness["label"].as_str().unwrap().contains("全集"));
                assert!(witness["span"]["byte_start"].as_u64().is_some());
                assert!(witness["span"]["byte_end"].as_u64().is_some());
            }
        }
    }
}
