//! Retrospective gaiji declarations replace one adjacent source character.

use ab_aozora_facade::syntax::ast::{GaijiMarkerPlacement, Segment};
use ab_aozora_facade::{Content, Document, Node, NodeRef};

#[test]
fn retrospective_gaiji_declarations_replace_their_adjacent_source_text() {
    for (source, target, glyph) in [
        ("詩集1［＃「1」はローマ数字、1-13-21］後", "1", "Ⅰ"),
        ("詩集４［＃「４」はローマ数字4、1-13-24］後", "４", "Ⅳ"),
        ("詩集？！［＃「？！」は横１文字、1-8-77］後", "？！", "⁈"),
        (
            "詩集!!［＃「!!」は一文字、第3水準1-8-75、111-11］後",
            "!!",
            "‼",
        ),
        (
            "LACLARTE［＃「E」はアキュートアクセント付きE、1-9-32］後",
            "E",
            "É",
        ),
        (
            "タックネップ［＃「プ」は小書き半濁点付き片仮名フ、1-6-88］ネ",
            "プ",
            "ㇷ゚",
        ),
        ("シサム［＃「ム」は小書き片仮名ム、1-6-89］達", "ム", "ㇺ"),
    ] {
        let document = Document::new(source);
        let tree = document.parse();
        let gaiji = tree
            .source_nodes()
            .iter()
            .find_map(|node| match node.node {
                NodeRef::Inline(Node::Gaiji(gaiji)) => Some((node, gaiji)),
                _ => None,
            })
            .expect("roman declaration is a gaiji");

        assert!(gaiji.1.replaced.is_some(), "{source}");
        let marker_start = source.find('［').unwrap();
        let target_start = marker_start - target.len();
        assert_eq!(
            &source[gaiji.0.source_span.start as usize..gaiji.0.source_span.end as usize],
            &source[target_start..source.len() - source.chars().next_back().unwrap().len_utf8()],
        );
        assert_eq!(tree.to_source(), source);
        let html = tree.to_html();
        assert!(html.contains(&format!(">{glyph}</span>")), "{html}");
        assert!(
            !html.contains(&format!("{target}<span class=\"aozora-gaiji\"")),
            "{html}"
        );
    }
}

#[test]
fn retrospective_gaiji_inside_a_ruby_reading_replaces_its_source_character() {
    let source = "和人《シサム［＃「ム」は小書き片仮名ム、1-6-89］》";
    let document = Document::new(source);
    let tree = document.parse();
    let ruby = tree
        .source_nodes()
        .iter()
        .find_map(|node| match node.node {
            NodeRef::Inline(Node::Ruby(ruby)) => Some(ruby),
            _ => None,
        })
        .expect("source contains ruby");
    let [Content::Segments(reading)] = tree.lex_output().store.resolve_content_range(ruby.reading)
    else {
        panic!("reading should retain structured gaiji content");
    };
    let segments = tree.lex_output().store.resolve_seg_range(*reading);
    assert!(
        matches!(segments, [Segment::Text(_), Segment::Gaiji { value, .. }] if value.replaced.is_some()),
        "{segments:?}"
    );
    assert_eq!(tree.to_source(), format!("｜{source}"));
    assert!(
        tree.to_html()
            .contains("<rt>シサ<span class=\"aozora-gaiji\"")
    );
}

#[test]
fn unresolved_combined_punctuation_retains_its_adjacent_source_fallback() {
    let source = "詩集!!!［＃「!!!」は一文字、95-13］後";
    let document = Document::new(source);
    let tree = document.parse();
    let (node, gaiji) = tree
        .source_nodes()
        .iter()
        .find_map(|node| match node.node {
            NodeRef::Inline(Node::Gaiji(gaiji)) => Some((node, gaiji)),
            _ => None,
        })
        .expect("combined punctuation declaration is a gaiji");

    assert!(gaiji.replaced.is_some());
    assert!(gaiji.resolve(&tree.lex_output().store).is_none());
    assert_eq!(
        &source[node.source_span.start as usize..node.source_span.end as usize],
        "!!!［＃「!!!」は一文字、95-13］"
    );
    assert_eq!(tree.to_source(), source);
    assert!(tree.to_html().contains(">!!!</span>"));
}

#[test]
fn unmarked_component_descriptions_remain_standalone_glyph_insertions() {
    for source in [
        "眠そうに［＃「浄」の「さんずい」の代わりに「めへん」、37-3］たまま",
        "波の上。［＃「楫」に「ほこづくり」を加える、55-12］枕。",
        "眼を［＃「浄」の「さんずい」の代わりに「めへん」、58-17］っていた",
    ] {
        let document = Document::new(source);
        let tree = document.parse();
        let (node, gaiji) = tree
            .source_nodes()
            .iter()
            .find_map(|node| match node.node {
                NodeRef::Inline(Node::Gaiji(gaiji)) => Some((node, gaiji)),
                _ => None,
            })
            .expect("component description is a gaiji insertion");

        assert!(gaiji.standalone, "{source}");
        assert!(gaiji.replaced.is_none(), "{source}");
        assert_eq!(
            &source[node.source_span.start as usize..node.source_span.end as usize],
            &source[source.find('［').unwrap()..source.rfind('］').unwrap() + '］'.len_utf8()],
        );
        assert_eq!(tree.to_source(), source);
    }
}

#[test]
fn persistent_double_parenthesis_declarations_apply_to_later_pairs() {
    let open = "［＃この二重括弧は一文字、以下同様に使用、第3水準1-2-54、197-16］";
    let close = "［＃この二重括弧は一文字、以下同様に使用、第3水準1-2-55、197-16］";
    let source = format!("（（{open}前文略））{close}\n（（中略））\n（（下略））");
    let document = Document::new(source.clone());
    let tree = document.parse();
    let gaiji = tree
        .source_nodes()
        .iter()
        .filter_map(|node| match node.node {
            NodeRef::Inline(Node::Gaiji(gaiji)) => Some(gaiji),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(gaiji.len(), 6);
    assert_eq!(
        gaiji
            .iter()
            .map(|gaiji| {
                let mut glyph = String::new();
                gaiji
                    .resolve(&tree.lex_output().store)
                    .unwrap()
                    .write_to(&mut glyph)
                    .unwrap();
                glyph
            })
            .collect::<Vec<_>>(),
        ["｟", "｠", "｟", "｠", "｟", "｠"]
    );
    assert!(gaiji.iter().all(|gaiji| gaiji.declaration_span.is_some()));
    assert_eq!(tree.to_source(), source);
    assert_eq!(
        tree.to_html().matches("data-codepoint=\"U+FF5F\"").count(),
        3
    );
    assert_eq!(
        tree.to_html().matches("data-codepoint=\"U+FF60\"").count(),
        3
    );
}

#[test]
fn postposed_retrospective_gaiji_belongs_to_the_ruby_reading() {
    for (source, prefix, glyph) in [
        (
            "太陽《チュプ》［＃「プ」は小書き半濁点付き片仮名フ、1-6-88］",
            "チュ",
            "ㇷ゚",
        ),
        (
            "和人《シサム》［＃「ム」は小書き片仮名ム、1-6-89］達",
            "シサ",
            "ㇺ",
        ),
    ] {
        let document = Document::new(source);
        let tree = document.parse();
        let source_nodes = tree.source_nodes();
        let ruby = source_nodes
            .iter()
            .find_map(|node| match node.node {
                NodeRef::Inline(Node::Ruby(ruby)) => Some((node, ruby)),
                _ => None,
            })
            .expect("source contains ruby");
        assert!(
            !source_nodes
                .iter()
                .any(|node| matches!(node.node, NodeRef::Inline(Node::Gaiji(_))))
        );
        let [Content::Segments(reading)] = tree
            .lex_output()
            .store
            .resolve_content_range(ruby.1.reading)
        else {
            panic!("reading should retain structured gaiji content");
        };
        let segments = tree.lex_output().store.resolve_seg_range(*reading);
        assert!(matches!(
            segments,
            [Segment::Text(_), Segment::Gaiji {
                value,
                placement: GaijiMarkerPlacement::AfterRuby,
                ..
            }] if value.replaced.is_some()
        ));
        assert_eq!(
            ruby.0.source_span.end as usize,
            source.trim_end_matches('達').len()
        );
        assert_eq!(tree.to_source(), format!("｜{source}"));
        let html = tree.to_html();
        assert!(
            html.contains(&format!("<rt>{prefix}<span class=\"aozora-gaiji\"")),
            "{html}"
        );
        assert!(html.contains(&format!(">{glyph}</span>")), "{html}");
    }
}

#[test]
fn detached_declarations_identify_earlier_gaiji_placeholders() {
    for (source, resolved, ruby_base) in [
        (
            "※々［＃「※」は「つつみがまえ（勹）」＋「夕」で、読みは「そうそう」67-6］",
            None,
            false,
        ),
        (
            "琅※《らうかん》［＃「※」は「王へん」に「干」、第3水準1-87-83、P61-下段5首目］",
            Some("玕"),
            true,
        ),
        (
            "※《いとど》［＃「※」は「虫へん」に「車」、第3水準1-91-55、69-下段9首目］",
            Some("蛼"),
            true,
        ),
        (
            "幽かな※芙藍［＃「さんずい＋自」、第3水準1-86-66、XI-1］《さふらん》の凋れ",
            Some("洎"),
            true,
        ),
        (
            "郵便局の上の字は大※［＃「大※」に傍点］［＃「郷－即のへん」、232-1］だけ見えている",
            Some("\u{28675}"),
            false,
        ),
    ] {
        let document = Document::new(source);
        let tree = document.parse();
        let gaiji = tree
            .source_nodes()
            .iter()
            .find_map(|node| match node.node {
                NodeRef::Inline(Node::Gaiji(value)) => Some(value),
                NodeRef::Inline(Node::Ruby(ruby)) => tree
                    .lex_output()
                    .store
                    .resolve_content_range(ruby.base)
                    .iter()
                    .find_map(|content| match content {
                        Content::Segments(range) => tree
                            .lex_output()
                            .store
                            .resolve_seg_range(*range)
                            .iter()
                            .find_map(|segment| match segment {
                                Segment::Gaiji { value, .. } => Some(*value),
                                _ => None,
                            }),
                        _ => None,
                    }),
                _ => None,
            })
            .expect("detached declaration identifies a gaiji");
        assert!(gaiji.declaration_span.is_some(), "{source}");
        assert_eq!(tree.to_source(), source, "{source}");
        let html = tree.to_html();
        if let Some(resolved) = resolved {
            assert!(html.contains(&format!(">{resolved}</span>")), "{html}");
        } else {
            assert!(html.contains("class=\"aozora-gaiji\""), "{html}");
        }
        assert_eq!(html.contains("<ruby>"), ruby_base, "{html}");
        assert!(!html.contains("［＃"), "{html}");
        assert!(!html.contains("王へん"), "{html}");
        assert!(!html.contains("虫へん"), "{html}");
    }
}

#[test]
fn roman_numeral_declarations_require_an_adjacent_matching_target() {
    for source in [
        "詩集1の［＃「1」はローマ数字、1-13-21］後",
        "詩集2［＃「1」はローマ数字、1-13-21］後",
        "詩集1［＃「1」はローマ数字2、1-13-21］後",
        "詩集1［＃「1」はローマ数字、1-13-22］後",
        "詩集??［＃「!!!」は一文字、95-13］後",
        "LACLARTF［＃「E」はアキュートアクセント付きE、1-9-32］後",
        "LACLARTEの［＃「E」はアキュートアクセント付きE、1-9-32］後",
        "LACLARTE［＃「E」はアキュートアクセント付きE、1-9-33］後",
        "タックネップ［＃「プ」は小書き片仮名ム、1-6-88］ネ",
    ] {
        let document = Document::new(source);
        let tree = document.parse();
        assert!(tree.source_nodes().iter().any(|node| {
            matches!(node.node, NodeRef::Inline(Node::Gaiji(gaiji)) if gaiji.replaced.is_none())
        }));
        assert_eq!(tree.to_source(), source);
    }
}
