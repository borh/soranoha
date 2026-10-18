//! Supplied editorial statements retain their meaning and source position.

use ab_aat::aat_json_from_bytes;
use ab_aozora_facade::Document;
use serde_json::Value;

/// Statement body, expected note kind, and whether the note is uncertain.
const STATEMENTS: &[(&str, &str, bool)] = &[
    ("「註」略", "omission", false),
    ("未完", "incompleteness", false),
    ("現代語訳「月は明るい。」", "translation", false),
    ("「Ｏ」は覆面の英字です。", "explanation", false),
    (
        "この作品は表題と副題のみで、本文はありません。",
        "explanation",
        false,
    ),
    (
        "「雲隠れ」の帖は冒頭の晶子詞のみで本文はありません。",
        "explanation",
        false,
    ),
    ("劇場名", "explanation", false),
    ("ホテル名", "explanation", false),
    ("お手伝いさん", "explanation", false),
    ("夫人", "explanation", false),
    ("スカーフ", "explanation", false),
    ("長男", "explanation", false),
    ("三男", "explanation", false),
    ("次男", "explanation", false),
    ("小説家", "explanation", false),
    ("長女", "explanation", false),
    ("父", "explanation", false),
    ("母", "explanation", false),
    ("甥", "explanation", false),
    ("次女", "explanation", false),
    ("以下、新聞の切抜き", "explanation", false),
    ("「七夕」「真つすぐな街」は自由律俳句", "explanation", false),
    ("罫線の部分は、「｛」「｝」で括る", "transcription", false),
    (
        "欄外に「玉葉集巻十八、雑五、従三位爲子」の校注あり",
        "base-edition",
        false,
    ),
    ("上部欄外に「じうもんじカ」", "base-edition", false),
    (
        "「＊」は注釈記号。欄外に「釜の製造元」の注",
        "base-edition",
        false,
    ),
    (
        "底本の本文にこの注釈場所を示す注釈番号なし",
        "base-edition",
        false,
    ),
    ("省略", "omission", false),
    ("Ａ、Ｂ、Ｃの図省略", "omission", false),
    ("図は省略", "omission", false),
    ("紙片の図、図省略", "omission", false),
    ("図省略", "omission", false),
    ("「年表」省略", "omission", false),
    ("「Ｂ圖」省略", "omission", false),
    ("王家の紙幣の図、図省略", "omission", false),
    ("図が入るが省略。底本43ページ", "omission", false),
    ("図が入るが省略。底本44ページ", "omission", false),
    (
        "この後、改ページに続いて「VI.　文例」の章があるが、著作権の状態が不明なため、省略する。",
        "omission",
        false,
    ),
    (
        "目次のページ数および「解題（大内兵衛）」「追記」は省略しました",
        "omission",
        false,
    ),
    ("「人」が脱落か？", "omission", true),
    ("「や」が欠如か", "omission", true),
    ("句点が抜けていると考えられる", "omission", true),
    // A figure the source places but does not supply. Distinct from an
    // omission: these state that a figure belongs here, not that one was
    // left out.
    ("図１入る", "figure-insertion", false),
    ("図６入る", "figure-insertion", false),
    (
        "カット「焼夷弾の図」入る。16-上段",
        "figure-insertion",
        false,
    ),
    (
        "昭和新山の出来た経過を示す図入る",
        "figure-insertion",
        false,
    ),
    ("図１、工場内の見取り図", "figure-insertion", false),
    (
        "図２、「のり」に丸囲みの手書き文字。「り」は小さく頭の部分が「の」の下隙間に入る",
        "figure-insertion",
        false,
    ),
    ("図形　□（四角）に内接する◆", "figure-insertion", false),
    (
        "子供が描いた地図入る：星形の都市",
        "figure-insertion",
        false,
    ),
    (
        "図「ソヴェト選挙系統」入る、P516",
        "figure-insertion",
        false,
    ),
    ("ここに挿し絵入る", "figure-insertion", false),
    ("ここに花園の挿絵あり", "figure-insertion", false),
    ("楽譜入る", "figure-insertion", false),
    ("図１～図３は、右から続く一葉", "explanation", false),
    (
        "「ねませ和子よの譜」の表題付きの楽譜入る（略）",
        "omission",
        false,
    ),
];

#[test]
fn source_statements_preserve_assertion_kind_and_exact_marker() {
    for &(statement, kind, uncertain) in STATEMENTS {
        let marker = format!("［＃{statement}］");
        let source = format!("前{marker}後");
        assert_eq!(Document::new(source.as_str()).parse().to_source(), source);
        let aat: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        let content = aat["blocks"][0]["content"].as_array().unwrap();
        let note = content
            .iter()
            .find(|node| node["kind"] == "editorial_note")
            .unwrap();
        assert_eq!(note["note_kind"], kind);
        let expected_text = if kind == "translation" {
            statement
                .strip_prefix("現代語訳「")
                .and_then(|text| text.strip_suffix('」'))
                .unwrap()
        } else {
            statement
        };
        assert_eq!(note["text"], expected_text);
        assert_eq!(note.get("certainty").is_some(), uncertain);
        if uncertain {
            assert_eq!(note["certainty"], "uncertain");
        }
        assert_eq!(note["span"]["byte_start"], "前".len());
        assert_eq!(note["span"]["byte_end"], "前".len() + marker.len());
        assert_eq!(content.first().unwrap()["value"], "前");
        assert_eq!(content.last().unwrap()["value"], "後");
        let facts = aat["meta"]["interpretation_facts"].as_array().unwrap();
        assert_eq!(
            facts
                .iter()
                .filter(|fact| fact["kind"] == "editorial-note")
                .count(),
            1
        );
    }
}

#[test]
fn located_figure_statement_preserves_base_page_and_figure_semantics() {
    let source = "前［＃底本１４２頁右上に、持久戦争と決戦戦争の移り変わりを示した図あり］後";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let note = aat["blocks"][0]["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["kind"] == "editorial_note")
        .unwrap();

    assert_eq!(note["note_kind"], "figure-insertion");
    assert_eq!(note["source_locator"], "１４２");
    assert_eq!(note["page_position"], "top-right");
    assert_eq!(note["figure_kind"], "figure");
    assert_eq!(
        note["figure_description"],
        "持久戦争と決戦戦争の移り変わりを示した"
    );
    assert_eq!(
        note["text"],
        &source["前［＃".len()..source.len() - "］後".len()]
    );
}

#[test]
fn statement_like_prose_and_unknown_qualifiers_remain_distinct() {
    for source in [
        "未完",
        "この作品は表題と副題のみで、本文はありません。",
        "父",
        "［＃父？］",
        "［＃父、太字］",
        "［＃父に傍点］",
        "［＃ここから劇場名］",
        "［＃未知の説明］",
        "［＃「Ｏ」は覆面の英字ですか。］",
        "［＃「０」は覆面の英字です。］",
        "［＃人物］",
        "［＃本文はありません］",
        "［＃この作品は表題と副題のみで、本文はありませんか。］",
        "［＃「雲隠れ」の帖は冒頭の晶子詞のみで本文はありません。以下太字］",
        "［＃未完のため省略］",
        "［＃「註」一部略］",
        "［＃図を省略しない］",
        "［＃図を省略する場合］",
        "［＃「省略」とある］",
        "［＃図省略か］",
        "［＃図が入るが省略。底本不明ページ］",
        "［＃現代語訳「」］",
        "［＃現代語訳「月は明るい］",
        "［＃現代語訳「月は明るい」か］",
        "［＃「現代語訳」は見出しらしい］",
        // Reads as a figure note but supplies one, so it stays an illustration.
        "［＃挿絵１（fig226_01.png、横570×縦829）入る］",
        // A description needs a figure number and text.
        "［＃図、花の絵］",
        "［＃図１、］",
        // Identifies nothing: a declaration must state which figure.
        "［＃入る］",
        "［＃図入る］",
        // A locator tail must state a location, and only one may follow.
        "［＃カット「焼夷弾の図」入る。］",
        "［＃カット「焼夷弾の図」入る。16。上段］",
    ] {
        let aat: Value =
            serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
        assert!(
            aat["blocks"][0]["content"]
                .as_array()
                .unwrap()
                .iter()
                .all(|node| node["kind"] != "editorial_note")
        );
    }
}

fn note_after(prefix: &str, body: &str) -> Option<Value> {
    let source = format!("{prefix}［＃{body}］後");
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    aat["blocks"][0]["content"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|node| node["kind"] == "editorial_note")
        .cloned()
}

#[test]
fn a_gloss_after_printed_text_is_an_explanation() {
    for body in [
        "中條精一郎",
        "大瀧潤家、叔母（父の妹）鷹子の夫",
        "マリアナ基地からのＢ29、東京を初偵察",
        "トゥルビン家のありし日",
    ] {
        let note = note_after("息子", body).unwrap_or_else(|| panic!("{body}"));
        assert_eq!(note["note_kind"], "explanation", "{body}");
        assert_eq!(note["text"], body);
        assert_eq!(note["span"]["byte_start"], "息子".len());
    }
    let note = note_after("小さい弟", "中條《ちゅうじょう》英男、中條家三男").unwrap();
    assert_eq!(note["note_kind"], "explanation");
    assert_eq!(note["annotation_content"][0]["kind"], "ruby");
    assert_eq!(note["annotation_content"][1]["value"], "英男、中條家三男");
    for (prefix, body) in [
        ("", "中條精一郎"),
        ("息子　", "中條精一郎"),
        ("――――", "直線は中央に配置"),
        ("＊", "中央寄せ"),
        ("あ", "ゴチック"),
        ("t", "tは上下逆"),
        ("下船頭", "創元社版は「小船頭」"),
        ("あ", "（止（波））"),
        ("あ", "ルビ付き"),
    ] {
        assert!(note_after(prefix, body).is_none(), "{prefix}［＃{body}］");
    }
}

#[test]
fn supplied_translation_preserves_ruby_as_note_content() {
    let marker = "［＃現代語訳「松籟《しょうらい》を聞かせる。」］";
    let source = format!("前{marker}後");
    assert_eq!(Document::new(source.as_str()).parse().to_source(), source);
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let content = aat["blocks"][0]["content"].as_array().unwrap();
    let note = content
        .iter()
        .find(|node| node["kind"] == "editorial_note")
        .unwrap();
    assert_eq!(note["note_kind"], "translation");
    let annotation_content = note["annotation_content"].as_array().unwrap();
    let ruby = annotation_content
        .iter()
        .find(|node| node["kind"] == "ruby")
        .unwrap();
    assert_eq!(ruby["base"], "松籟");
    assert_eq!(ruby["reading"], "しょうらい");
    assert_eq!(ruby["span"]["byte_start"], source.find("松籟").unwrap());
    assert_eq!(annotation_content[1]["value"], "を聞かせる。");
    assert_eq!(content.first().unwrap()["value"], "前");
    assert_eq!(content.last().unwrap()["value"], "後");
    assert!(
        aat["meta"]["interpretation_facts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|fact| fact["kind"] == "editorial-note"
                && fact["source_span"]["start"] == "前".len()
                && fact["source_span"]["end"] == "前".len() + marker.len())
    );
}

#[test]
fn an_explanation_statement_keeps_the_warichu_it_quotes_as_note_content() {
    let (open, close) = ("［＃ここから割り注］", "［＃ここで割り注終わり］");
    let body = format!(
        "著者所蔵の「研幾小録」の欄外には、「召誥　其惟王位在徳元{open}孔傳其惟王居位在徳之首{close}」といふ著者の書き込みがある。"
    );
    let source = format!("元［＃{body}］後");
    let note = note_after("元", &body).unwrap();
    assert_eq!(note["note_kind"], "explanation");
    let warichu = &note["annotation_content"][1];
    assert_eq!(warichu["kind"], "warichu");
    assert_eq!(warichu["content"][0]["value"], "孔傳其惟王居位在徳之首");
    let open_start = source.find(open).unwrap();
    let close_start = source.find(close).unwrap();
    let markers: Vec<_> = warichu["interpretation_marker_spans"]
        .as_array()
        .unwrap()
        .iter()
        .map(|span| (span["byte_start"].clone(), span["byte_end"].clone()))
        .collect();
    assert_eq!(
        markers,
        [
            (open_start.into(), (open_start + open.len()).into()),
            (close_start.into(), (close_start + close.len()).into()),
        ]
    );
}

#[test]
fn annotation_can_target_the_adjacent_ruby_reading() {
    let source =
        "間人連老《ハシビトノムラジオユ》［＃ルビの「ハシビトノムラジオユ」に〈原〉の注記］";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let note = aat["blocks"][0]["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["note_kind"] == "ruby-reading-annotation")
        .unwrap();

    assert_eq!(note["text"], "〈原〉");
    assert_eq!(note["target_text"], "ハシビトノムラジオユ");
    assert_eq!(note["target_source_spans"][0]["byte_start"], 15);
    assert_eq!(note["target_source_spans"][0]["byte_end"], 45);
    assert_eq!(note["span"]["byte_start"], 48);
    assert_eq!(note["span"]["byte_end"], 123);
}

#[test]
fn sic_annotation_can_target_a_unique_ruby_reading_subrange() {
    let source = "東海美人《しうり》［＃ルビの「しう」にママの注記］";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let note = aat["blocks"][0]["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["note_kind"] == "sic")
        .unwrap();

    assert_eq!(note["text"], "ママ");
    assert_eq!(note["target_text"], "しう");
    assert_eq!(note["target_source_spans"][0]["byte_start"], 15);
    assert_eq!(note["target_source_spans"][0]["byte_end"], 21);
    assert_eq!(note["span"]["byte_start"], 27);
    assert_eq!(note["span"]["byte_end"], 75);
}

#[test]
fn ruby_reading_annotation_does_not_choose_an_ambiguous_subrange() {
    let source = "語《しうしう》［＃ルビの「しう」にママの注記］";
    let aat: Value =
        serde_json::from_slice(&aat_json_from_bytes(source.as_bytes()).unwrap()).unwrap();
    let note = aat["blocks"][0]["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["note_kind"] == "sic")
        .unwrap();

    assert_eq!(note["target_text"], "しう");
    assert!(note.get("target_source_spans").is_none());
}
