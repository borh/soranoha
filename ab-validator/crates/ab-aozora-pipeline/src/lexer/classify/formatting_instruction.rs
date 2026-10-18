use std::num::NonZeroU32;

use ab_aozora_syntax::{
    AbsoluteSize, BoutenPosition, DecorationPosition, EnclosureKind, ForwardAttr,
    QualitativeFontSize, Span,
    ast::{
        FormattingEffect, FormattingInstruction, FormattingOperation, FormattingPlacement,
        FormattingPredecessor, FormattingRuleStyle, FormattingSelection, FormattingSelector,
        HeadingTypography, KuntenExclusion, KuntenPolicy, KuntenScope, LineStartAlignment,
    },
};

use super::directive::bouten_kind_from_suffix;

struct SourceBody<'a> {
    text: &'a str,
    start: u32,
}

impl<'a> SourceBody<'a> {
    fn span(&self, text: &str) -> Span {
        let offset = text.as_ptr().addr() - self.text.as_ptr().addr();
        let start = self.start + u32::try_from(offset).expect("source offset fits u32");
        Span::new(
            start,
            start + u32::try_from(text.len()).expect("source length fits u32"),
        )
    }

    fn quoted(&self, text: &'a str) -> Option<(Span, &'a str)> {
        let contents = text.strip_prefix('「')?;
        let mut depth = 1_u32;
        let mut last_close = None;
        for (offset, character) in contents.char_indices() {
            match character {
                '「' => depth += 1,
                '」' => {
                    last_close = Some(offset);
                    depth -= 1;
                    if depth == 0 {
                        let operand = &contents[..offset];
                        let rest = &contents[offset + character.len_utf8()..];
                        return (!operand.is_empty() && !operand.contains(['\r', '\n']))
                            .then(|| (self.span(operand), rest));
                    }
                }
                '\r' | '\n' => return None,
                _ => {}
            }
        }
        let offset = last_close?;
        let operand = &contents[..offset];
        (!operand.is_empty()).then(|| (self.span(operand), &contents[offset + '」'.len_utf8()..]))
    }

    fn selector(&self, text: &'a str) -> Option<(FormattingSelector, &'a str)> {
        if let Some(contents) = text.strip_prefix('（')
            && let Some(offset) = contents.find('）')
        {
            let end = '（'.len_utf8() + offset + '）'.len_utf8();
            let operand = &text[..end];
            let inner = &contents[..offset];
            if !inner.is_empty() && !inner.contains(['（', '）', '\r', '\n']) {
                return Some((
                    FormattingSelector::Parenthesized(self.span(operand)),
                    &text[end..],
                ));
            }
        }
        if let Some(rest) = text.strip_prefix("アラビア数字") {
            return Some((FormattingSelector::Digits, rest));
        }
        if let Some(rest) = text.strip_prefix("数字") {
            return Some((FormattingSelector::Digits, rest));
        }
        if text.chars().next().is_some_and(char::is_alphabetic)
            && !text.starts_with("すべての")
            && let Some((preceding, rest)) = text.split_once("に続くアラビア数字")
        {
            let condition = if preceding == "アルファベット" {
                FormattingPredecessor::Alphabetic
            } else {
                let operands = preceding
                    .split('、')
                    .map(|operand| {
                        let operand = operand.trim();
                        let mut chars = operand.chars();
                        (chars.next().is_some_and(char::is_alphabetic) && chars.next().is_none())
                            .then(|| self.span(operand))
                    })
                    .collect::<Option<Vec<_>>>()?;
                FormattingPredecessor::NamedCharacters(operands)
            };
            return Some((FormattingSelector::ArabicDigitsAfter(condition), rest));
        }
        if let Some(rest) = text.strip_prefix("すべての") {
            let (operand, tail) = self.quoted(rest)?;
            return Some((FormattingSelector::AllQuoted(operand), tail));
        }
        if let Some((ordinal, rest)) = ordinal(text) {
            if let Some(rest) = rest.strip_prefix("文字目の") {
                let (operand, tail) = self.quoted(rest)?;
                return Some((
                    FormattingSelector::CharacterPosition { ordinal, operand },
                    tail,
                ));
            }
            let (operand, tail) = self.quoted(rest.strip_prefix("つめの")?)?;
            return Some((FormattingSelector::Occurrence { ordinal, operand }, tail));
        }
        let (first, mut tail) = self.quoted(text)?;
        let mut operands = vec![first];
        loop {
            let next = tail.strip_prefix('、').unwrap_or(tail);
            let Some((operand, rest)) = self.quoted(next) else {
                break;
            };
            operands.push(operand);
            tail = rest;
        }
        let selector = if operands.len() == 1 {
            FormattingSelector::Quoted(first)
        } else {
            FormattingSelector::EachQuoted(operands)
        };
        Some((selector, tail))
    }

    fn effect(&self, text: &'a str) -> Option<FormattingEffect> {
        if let Some((base, "の上付き")) = self.quoted(text) {
            return Some(FormattingEffect::UpperRelativeTo {
                operand: base,
                target: None,
            });
        }
        if let Some((degrees, "度回転")) = ordinal(text) {
            return Some(FormattingEffect::Rotation { degrees });
        }
        if let Some(lines) = text
            .strip_suffix("行の左右中央")
            .and_then(japanese_u8)
            .and_then(|value| NonZeroU32::new(u32::from(value)))
        {
            return Some(FormattingEffect::HorizontalCenter { lines });
        }
        if let Some(lines) = text
            .strip_suffix("行組み")
            .and_then(japanese_u8)
            .and_then(|value| NonZeroU32::new(u32::from(value)))
        {
            return Some(FormattingEffect::LineSetting {
                lines,
                parentheses_lines: None,
            });
        }
        let attribute = match text {
            "縦中横" | "横一列" => ForwardAttr::CombineUpright,
            "上付き小文字" | "上付き小書き" => ForwardAttr::SuperScript,
            "下付き小文字" | "下付き小書き" => ForwardAttr::SubScript,
            "行右小書き" => ForwardAttr::SmallScript(BoutenPosition::Right),
            "行左小書き" => ForwardAttr::SmallScript(BoutenPosition::Left),
            "太字" => ForwardAttr::Bold,
            "ゴシック体" => ForwardAttr::Gothic,
            "地付き" => ForwardAttr::AlignEnd { offset: 0 },
            "斜体" | "イタリック体" => ForwardAttr::Italic,
            "特大文字" => ForwardAttr::FontSizeAbsolute(AbsoluteSize::ExtraLarge),
            "大文字" => ForwardAttr::FontSizeAbsolute(AbsoluteSize::Large),
            "中文字" => ForwardAttr::FontSizeAbsolute(AbsoluteSize::Medium),
            "小文字" | "小書き" => ForwardAttr::FontSizeAbsolute(AbsoluteSize::Small),
            "小さい文字" | "本文より小さいサイズの文字" | "ポイントを下げる" => {
                ForwardAttr::QualitativeFontSize(QualitativeFontSize::Smaller)
            }
            "小さめの文字" => {
                ForwardAttr::QualitativeFontSize(QualitativeFontSize::SlightlySmaller)
            }
            "分数" => ForwardAttr::Fraction,
            "罫囲み" => ForwardAttr::Framed(EnclosureKind::Rule),
            "横組み" => ForwardAttr::Horizontal,
            "縦組み" => ForwardAttr::Vertical,
            "下揃え" => {
                return Some(FormattingEffect::Layout {
                    indent: 0,
                    horizontal: false,
                    align_end: true,
                });
            }
            _ => ForwardAttr::Bouten {
                kind: bouten_kind_from_suffix(text)?,
                position: DecorationPosition::Right,
            },
        };
        Some(FormattingEffect::Attribute(attribute))
    }
}

fn ordinal(text: &str) -> Option<(NonZeroU32, &str)> {
    let mut number = 0u32;
    let mut end = 0;
    for (offset, ch) in text.char_indices() {
        let digit = match ch {
            '0'..='9' => u32::from(ch) - u32::from('0'),
            '０'..='９' => u32::from(ch) - u32::from('０'),
            _ => break,
        };
        number = number.checked_mul(10)?.checked_add(digit)?;
        end = offset + ch.len_utf8();
    }
    Some((NonZeroU32::new(number)?, &text[end..]))
}

pub(super) fn parse(body: &str, start: u32) -> Option<FormattingInstruction> {
    let source = SourceBody { text: body, start };
    if let Some(instruction) = special_instruction(&source, body) {
        return Some(instruction);
    }
    let mut rest = body;
    let mut previous_selector = None;
    let mut operations = Vec::new();
    loop {
        let clause = rest.trim_start();
        let (selector, tail) = if let Some((mut selector, tail)) = source.selector(clause) {
            let mut tail = tail
                .strip_prefix('は')
                .or_else(|| tail.strip_prefix('に'))
                .or_else(|| tail.strip_prefix('の'))?;
            if matches!(selector, FormattingSelector::ArabicDigitsAfter(_)) {
                tail = tail.strip_prefix("すべて")?;
            } else if matches!(selector, FormattingSelector::Digits) {
                tail = tail.strip_prefix("すべて").unwrap_or(tail);
            } else if let Some(effect) = tail.strip_prefix("すべて") {
                selector = match selector {
                    FormattingSelector::Quoted(operand) => FormattingSelector::AllQuoted(operand),
                    _ => return None,
                };
                tail = effect;
            }
            if let Some(after) = tail.strip_prefix("それぞれ") {
                selector = match selector {
                    FormattingSelector::Quoted(operand) => {
                        FormattingSelector::EachQuoted(vec![operand])
                    }
                    FormattingSelector::EachQuoted(operands) => {
                        FormattingSelector::EachQuoted(operands)
                    }
                    _ => return None,
                };
                tail = after;
            }
            (selector, tail)
        } else {
            (
                previous_selector
                    .clone()
                    .unwrap_or(FormattingSelector::PrecedingText),
                clause,
            )
        };
        let (effect, following) = split_clause(tail);
        let extent = clause.len() - tail.len() + effect.len();
        operations.push(FormattingOperation {
            source_span: source.span(&clause[..extent]),
            selector: selector.clone(),
            effect: source.effect(effect.trim_end())?,
            selection: None,
        });
        previous_selector = Some(selector);
        match following {
            None => break,
            Some("") => return None,
            Some(following) => rest = following,
        }
    }
    (operations.len() >= 2
        || operations
            .first()
            .is_some_and(|operation| supported_single(operation, source.text)))
    .then_some(FormattingInstruction { operations })
}

/// Split off the first clause at `、`, or at a `。` that a new quoted selector follows.
fn split_clause(tail: &str) -> (&str, Option<&str>) {
    let comma = tail.find('、').map(|offset| (offset, '、'.len_utf8()));
    let stop = tail.find("。「").map(|offset| (offset, '。'.len_utf8()));
    match comma.into_iter().chain(stop).min() {
        Some((offset, length)) => (&tail[..offset], Some(&tail[offset + length..])),
        None => (tail, None),
    }
}

fn special_instruction(source: &SourceBody<'_>, body: &str) -> Option<FormattingInstruction> {
    following_lines_typeface_and_rule(source, body)
        .or_else(|| paired_line_setting(source, body))
        .or_else(|| listed_occurrences(source, body))
        .or_else(|| listed_word_ending_ligature(source, body))
        .or_else(|| following_paragraph_frame(source, body))
        .or_else(|| following_parenthesized_line_setting(source, body))
        .or_else(|| cross_line_head_alignment(source, body))
        .or_else(|| split_line_end_instruction(source, body))
        .or_else(|| quoted_line_geometry_instruction(source, body))
        .or_else(|| mismatched_double_quote_end_alignment(source, body))
        .or_else(|| named_spacing_line_instruction(source, body))
        .or_else(|| tcy_below_day_range(source, body))
        .or_else(|| kaeriten_policy(source, body))
        .or_else(|| spatial_instruction(source, body))
        .or_else(|| compact_right_instruction(source, body))
        .or_else(|| line_foot_instruction(source, body))
        .or_else(|| subtarget_instruction(source, body))
        .or_else(|| small_contracting_sound_instruction(source, body))
        .or_else(|| gothic_emphasis_instruction(source, body))
        .or_else(|| {
            heading_typography(body).map(|typography| FormattingInstruction {
                operations: vec![FormattingOperation {
                    source_span: source.span(body),
                    selector: FormattingSelector::PrecedingText,
                    effect: FormattingEffect::HeadingTypography(typography),
                    selection: None,
                }],
            })
        })
        .or_else(|| {
            layout_policy(source, body).map(|(selector, indent, horizontal, align_end)| {
                FormattingInstruction {
                    operations: vec![FormattingOperation {
                        source_span: source.span(body),
                        selector,
                        effect: FormattingEffect::Layout {
                            indent,
                            horizontal,
                            align_end,
                        },
                        selection: None,
                    }],
                }
            })
        })
}

/// `「X」ゴシック体に<emphasis>`: X is set in gothic and carries the emphasis
/// marks, as the particle-free `「X」ゴシック体` sets it in gothic alone.
fn gothic_emphasis_instruction(
    source: &SourceBody<'_>,
    body: &str,
) -> Option<FormattingInstruction> {
    let (operand, rest) = source.quoted(body)?;
    let emphasis = rest.strip_prefix("ゴシック体に")?;
    let kind = bouten_kind_from_suffix(emphasis)?;
    let gothic_end = body.len() - rest.len() + "ゴシック体".len();
    let selector = FormattingSelector::Quoted(operand);
    Some(FormattingInstruction {
        operations: vec![
            FormattingOperation {
                source_span: source.span(&body[..gothic_end]),
                selector: selector.clone(),
                effect: FormattingEffect::Attribute(ForwardAttr::Gothic),
                selection: None,
            },
            FormattingOperation {
                source_span: source.span(&body[gothic_end..]),
                selector,
                effect: FormattingEffect::Attribute(ForwardAttr::Bouten {
                    kind,
                    position: DecorationPosition::Right,
                }),
                selection: None,
            },
        ],
    })
}

fn following_lines_typeface_and_rule(
    source: &SourceBody<'_>,
    body: &str,
) -> Option<FormattingInstruction> {
    let rest = body.strip_prefix('次')?;
    let (count, rest) = rest.split_once("行は、")?;
    let lines = NonZeroU32::new(u32::from(japanese_u8(count)?))?;
    if rest != "文字はゴシック体、罫線は全て波線" {
        return None;
    }
    let first_clause_end = body.find("、罫線")?;
    let second_clause_start = first_clause_end + '、'.len_utf8();
    let selector = FormattingSelector::FollowingLines { lines };
    Some(FormattingInstruction {
        operations: vec![
            FormattingOperation {
                source_span: source.span(&body[..first_clause_end]),
                selector: selector.clone(),
                effect: FormattingEffect::Attribute(ForwardAttr::Gothic),
                selection: None,
            },
            FormattingOperation {
                source_span: source.span(&body[second_clause_start..]),
                selector,
                effect: FormattingEffect::RuleStyle(FormattingRuleStyle::Wavy),
                selection: None,
            },
        ],
    })
}

/// A rule frame around the paragraph on the next source line.
fn following_paragraph_frame(source: &SourceBody<'_>, body: &str) -> Option<FormattingInstruction> {
    if body != "次の段落には、天地左右にオモテケイ囲み" {
        quoted_range_ends(body)?;
    }
    Some(FormattingInstruction {
        operations: vec![FormattingOperation {
            source_span: source.span(body),
            selector: FormattingSelector::FollowingLines {
                lines: NonZeroU32::new(1).expect("one is nonzero"),
            },
            effect: FormattingEffect::Attribute(ForwardAttr::Framed(EnclosureKind::Rule)),
            selection: None,
        }],
    })
}

/// The first and last words of `以下、「A…」から「…B」までは罫線囲み`, without
/// the ellipses that elide the rest of the paragraph.
fn quoted_range_ends(clause: &str) -> Option<(&str, &str)> {
    let rest = clause.strip_prefix("以下、「")?;
    let (first, rest) = rest.split_once("」から「")?;
    let last = rest.strip_suffix("」までは罫線囲み")?;
    let first = first.strip_suffix('…')?;
    let last = last.strip_prefix('…')?;
    (!first.is_empty()
        && !last.is_empty()
        && !first.contains(['「', '」'])
        && !last.contains(['「', '」']))
    .then_some((first, last))
}

/// `二つ目、三つ目の「X」は太字`: one operation per listed occurrence of `X`.
fn listed_occurrences(source: &SourceBody<'_>, body: &str) -> Option<FormattingInstruction> {
    let (ordinals, rest) = body.split_once("つ目の")?;
    let ordinals = ordinals
        .split(['、', 'と'])
        .map(|ordinal| {
            let ordinal = ordinal.strip_suffix("つ目").unwrap_or(ordinal);
            match ordinal {
                "一" => NonZeroU32::new(1),
                "二" => NonZeroU32::new(2),
                "三" => NonZeroU32::new(3),
                "四" => NonZeroU32::new(4),
                "五" => NonZeroU32::new(5),
                _ => None,
            }
        })
        .collect::<Option<Vec<_>>>()?;
    if ordinals.len() < 2 {
        return None;
    }
    let (operand, tail) = source.quoted(rest)?;
    let effect = source.effect(tail.strip_prefix('は')?)?;
    if !matches!(effect, FormattingEffect::Attribute(_)) {
        return None;
    }
    Some(FormattingInstruction {
        operations: ordinals
            .into_iter()
            .map(|ordinal| FormattingOperation {
                source_span: source.span(body),
                selector: FormattingSelector::Occurrence { ordinal, operand },
                effect: effect.clone(),
                selection: None,
            })
            .collect(),
    })
}

/// `「A」「B」および「C」のそれぞれの末尾「ae」は、「a」と「e」の合字`: one
/// operation per listed word, each selecting the ending the components spell.
fn listed_word_ending_ligature(
    source: &SourceBody<'_>,
    body: &str,
) -> Option<FormattingInstruction> {
    let mut words = Vec::new();
    let mut rest = body;
    while let Some((word, tail)) = source.quoted(
        rest.strip_prefix("および")
            .or_else(|| rest.strip_prefix('、'))
            .unwrap_or(rest),
    ) {
        words.push(word);
        rest = tail;
    }
    let (ending, rest) = source.quoted(rest.strip_prefix("のそれぞれの末尾")?)?;
    let rest = rest.strip_prefix('は')?;
    let (first, mut rest) = source.quoted(rest.strip_prefix('、').unwrap_or(rest))?;
    let mut components = vec![first];
    while let Some(tail) = rest.strip_prefix('と') {
        let (component, tail) = source.quoted(tail)?;
        components.push(component);
        rest = tail;
    }
    let ending_text = source_slice(source, ending);
    let spelled = components
        .iter()
        .map(|component| source_slice(source, *component))
        .collect::<String>();
    (rest == "の合字"
        && words.len() >= 2
        && components.len() >= 2
        && spelled == ending_text
        && words
            .iter()
            .all(|word| source_slice(source, *word).ends_with(ending_text)))
    .then(|| FormattingInstruction {
        operations: words
            .into_iter()
            .map(|enclosing| FormattingOperation {
                source_span: source.span(body),
                selector: FormattingSelector::Subtarget {
                    enclosing,
                    operand: ending,
                },
                effect: FormattingEffect::Ligature {
                    components: components.clone(),
                },
                selection: None,
            })
            .collect(),
    })
}

/// `「A」と「B」は２列に並ぶ` and its spellings: the joined runs share one line.
fn paired_line_setting(source: &SourceBody<'_>, body: &str) -> Option<FormattingInstruction> {
    let (first, mut tail) = source.quoted(body)?;
    let mut operands = vec![first];
    while let Some(rest) = tail.strip_prefix('と') {
        let (operand, rest) = source.quoted(rest)?;
        operands.push(operand);
        tail = rest;
    }
    let count = u8::try_from(operands.len()).ok()?;
    if count < 2 {
        return None;
    }
    let lines = if tail == "は横並びになっている" {
        count
    } else if let Some(rest) = tail.strip_prefix('が') {
        let (outer, rest) = rest.split_once("行内で")?;
        if japanese_u8(outer)? != 1 {
            return None;
        }
        japanese_u8(rest.strip_suffix("行に分けられている")?)?
    } else {
        let rest = tail.strip_prefix('は')?;
        japanese_u8(
            rest.strip_suffix("列に並ぶ")
                .or_else(|| rest.strip_suffix('行'))?,
        )?
    };
    if lines != count {
        return None;
    }
    Some(FormattingInstruction {
        operations: vec![FormattingOperation {
            source_span: source.span(body),
            selector: FormattingSelector::EachQuoted(operands),
            effect: FormattingEffect::LineSetting {
                lines: NonZeroU32::new(u32::from(lines))?,
                parentheses_lines: None,
            },
            selection: None,
        }],
    })
}

/// `地からN字上げ、M行にわたる丸括弧で挟んだL行組み`, stated before the text it sets.
fn following_parenthesized_line_setting(
    source: &SourceBody<'_>,
    body: &str,
) -> Option<FormattingInstruction> {
    let (alignment, setting) = body.split_once('、')?;
    let offset = japanese_u8(alignment.strip_prefix("地から")?.strip_suffix("字上げ")?)?;
    let (parentheses, lines) = setting.split_once("行にわたる丸括弧で挟んだ")?;
    let parentheses_lines = NonZeroU32::new(u32::from(japanese_u8(parentheses)?))?;
    let lines = NonZeroU32::new(u32::from(japanese_u8(lines.strip_suffix("行組み")?)?))?;
    Some(FormattingInstruction {
        operations: vec![
            FormattingOperation {
                source_span: source.span(alignment),
                selector: FormattingSelector::FollowingText,
                effect: FormattingEffect::Attribute(ForwardAttr::AlignEnd { offset }),
                selection: None,
            },
            FormattingOperation {
                source_span: source.span(setting),
                selector: FormattingSelector::FollowingText,
                effect: FormattingEffect::LineSetting {
                    lines,
                    parentheses_lines: Some(parentheses_lines),
                },
                selection: None,
            },
        ],
    })
}

fn cross_line_head_alignment(source: &SourceBody<'_>, body: &str) -> Option<FormattingInstruction> {
    let (selector, relation) = match body {
        "この歌、二行前の歌に頭揃え。" => (
            FormattingSelector::PrecedingText,
            LineStartAlignment::RelativeLine {
                lines_before: NonZeroU32::new(2).expect("two is nonzero"),
                reference_target: None,
            },
        ),
        "で各歌の頭は全て揃っている。" => {
            let members = &body['で'.len_utf8()..'で'.len_utf8() + "各歌".len()];
            (
                FormattingSelector::Named(source.span(members)),
                LineStartAlignment::Collective {
                    members: source.span(members),
                },
            )
        }
        _ => return None,
    };
    Some(FormattingInstruction {
        operations: vec![FormattingOperation {
            source_span: source.span(body),
            selector,
            effect: FormattingEffect::LineStartAlignment(relation),
            selection: None,
        }],
    })
}

fn split_line_end_instruction(
    source: &SourceBody<'_>,
    body: &str,
) -> Option<FormattingInstruction> {
    let rest = body.strip_prefix("この行ここまで相対的に字が小さい、ここからは下揃え、下から")?;
    let offset = japanese_u8(rest.strip_suffix("字上げ")?)?;
    if offset == 0 {
        return None;
    }
    let comma = body.find('、')?;
    Some(FormattingInstruction {
        operations: vec![
            FormattingOperation {
                source_span: source.span(&body[..comma]),
                selector: FormattingSelector::PrecedingText,
                effect: FormattingEffect::Attribute(ForwardAttr::QualitativeFontSize(
                    QualitativeFontSize::Smaller,
                )),
                selection: None,
            },
            FormattingOperation {
                source_span: source.span(&body[comma + '、'.len_utf8()..]),
                selector: FormattingSelector::FollowingText,
                effect: FormattingEffect::Attribute(ForwardAttr::AlignEnd { offset }),
                selection: None,
            },
        ],
    })
}

fn quoted_line_geometry_instruction(
    source: &SourceBody<'_>,
    body: &str,
) -> Option<FormattingInstruction> {
    let (operand, tail) = source.quoted(body)?;
    let tail = tail.strip_prefix("は天より")?;
    let (indent, tail) = ordinal(tail)?;
    let tail = tail.strip_prefix("字下げ、地より")?;
    let (offset, tail) = ordinal(tail)?;
    if tail != "字上げ" || indent.get() > u32::from(u8::MAX) || offset.get() > u32::from(u8::MAX)
    {
        return None;
    }
    let selector = FormattingSelector::Quoted(operand);
    Some(FormattingInstruction {
        operations: vec![
            FormattingOperation {
                source_span: source.span(body),
                selector: selector.clone(),
                effect: FormattingEffect::Layout {
                    indent: u8::try_from(indent.get()).ok()?,
                    horizontal: false,
                    align_end: false,
                },
                selection: None,
            },
            FormattingOperation {
                source_span: source.span(body),
                selector,
                effect: FormattingEffect::Attribute(ForwardAttr::AlignEnd {
                    offset: u8::try_from(offset.get()).ok()?,
                }),
                selection: None,
            },
        ],
    })
}

fn mismatched_double_quote_end_alignment(
    source: &SourceBody<'_>,
    body: &str,
) -> Option<FormattingInstruction> {
    let contents = body.strip_prefix('『')?;
    let operand = contents.strip_suffix("』は地付き")?;
    if operand.is_empty() || operand.contains(['『', '』', '\r', '\n']) {
        return None;
    }
    Some(FormattingInstruction {
        operations: vec![FormattingOperation {
            source_span: source.span(body),
            selector: FormattingSelector::Quoted(source.span(operand)),
            effect: FormattingEffect::Attribute(ForwardAttr::AlignEnd { offset: 0 }),
            selection: None,
        }],
    })
}

fn named_spacing_line_instruction(
    source: &SourceBody<'_>,
    body: &str,
) -> Option<FormattingInstruction> {
    let rest = body.strip_prefix("この行は下揃え")?;
    let end = rest.find("字上げ、")?;
    let offset = japanese_u8(&rest[..end])?;
    if offset == 0 {
        return None;
    }
    let spacing_clause = &rest[end + "字上げ、".len()..];
    let named = spacing_clause.strip_suffix("は一字毎に空きあり")?;
    if named.is_empty() {
        return None;
    }
    let comma = body.len() - spacing_clause.len() - '、'.len_utf8();
    Some(FormattingInstruction {
        operations: vec![
            FormattingOperation {
                source_span: source.span(&body[..comma]),
                selector: FormattingSelector::PrecedingText,
                effect: FormattingEffect::Attribute(ForwardAttr::AlignEnd { offset }),
                selection: None,
            },
            FormattingOperation {
                source_span: source.span(spacing_clause),
                selector: FormattingSelector::Named(source.span(named)),
                effect: FormattingEffect::LetterSpacing {
                    amount: NonZeroU32::new(1).expect("one is nonzero"),
                },
                selection: None,
            },
        ],
    })
}

fn tcy_below_day_range(source: &SourceBody<'_>, body: &str) -> Option<FormattingInstruction> {
    let (target, tail) = source.quoted(body)?;
    let range = tail.strip_prefix('は')?.strip_suffix("日の下に縦中横")?;
    let (start, end) = range.split_once('〜').or_else(|| range.split_once('～'))?;
    let start = start.strip_suffix('日')?;
    let start_day = NonZeroU32::new(u32::from(japanese_u8(start)?))?;
    let end_day = NonZeroU32::new(u32::from(japanese_u8(end)?))?;
    if start_day > end_day {
        return None;
    }
    let selector = FormattingSelector::Quoted(target);
    let placement = FormattingPlacement::BelowDayRange {
        start_operand: source.span(start),
        end_operand: source.span(end),
        start_day,
        end_day,
        start_target: None,
        end_target: None,
    };
    Some(FormattingInstruction {
        operations: vec![
            FormattingOperation {
                source_span: source.span(body),
                selector: selector.clone(),
                effect: FormattingEffect::Attribute(ForwardAttr::CombineUpright),
                selection: None,
            },
            FormattingOperation {
                source_span: source.span(body),
                selector,
                effect: FormattingEffect::RelativePlacement(placement),
                selection: None,
            },
        ],
    })
}

fn kaeriten_policy(source: &SourceBody<'_>, body: &str) -> Option<FormattingInstruction> {
    kaeriten_policy_with_scope(source, body, None)
}

pub(super) fn kaeriten_policy_clause(
    body: &str,
    start: u32,
    scope: KuntenScope,
) -> Option<FormattingInstruction> {
    let source = SourceBody { text: body, start };
    kaeriten_policy_with_scope(&source, body, Some(scope))
}

fn kaeriten_policy_with_scope(
    source: &SourceBody<'_>,
    body: &str,
    supplied_scope: Option<KuntenScope>,
) -> Option<FormattingInstruction> {
    const MARKS: &str = "一二三四五六七八九十レ上中下甲乙丙丁天地人";
    let insertion = body.starts_with("返り点の");
    if !insertion && !body.contains("は返り点") {
        return None;
    }
    let quoted = quoted_spans(source, body);
    let quoted_mark = quoted.iter().rev().copied().find(|span| {
        let value = source_slice(source, *span);
        !value.is_empty() && value.chars().all(|character| MARKS.contains(character))
    });
    let marks = quoted_mark.or_else(|| {
        let end = body.find("は返り点")?;
        let prefix = &body[..end];
        let start = prefix
            .char_indices()
            .rev()
            .find(|(_, character)| !MARKS.contains(*character))
            .map_or(0, |(offset, character)| offset + character.len_utf8());
        (start < end).then(|| source.span(&body[start..end]))
    })?;
    let scope = supplied_scope.unwrap_or_else(|| {
        if insertion {
            KuntenScope::InsertionSite
        } else if body.starts_with("以下") {
            KuntenScope::FollowingText
        } else {
            KuntenScope::QuotedRegion
        }
    });
    if supplied_scope.is_none()
        && !insertion
        && !body.starts_with("以下")
        && !body.contains("内の")
        && !body.contains("のぞ")
    {
        return None;
    }
    let exclusions = if body.contains("のぞ") {
        let before_marks = quoted
            .iter()
            .copied()
            .take_while(|span| *span != marks)
            .collect::<Vec<_>>();
        let (&mark, enclosing) = before_marks.split_last()?;
        enclosing
            .iter()
            .copied()
            .filter(|span| span.start != span.end)
            .map(|enclosing| KuntenExclusion { enclosing, mark })
            .collect()
    } else {
        Vec::new()
    };
    Some(FormattingInstruction {
        operations: vec![FormattingOperation {
            source_span: source.span(body),
            selector: FormattingSelector::Unresolved,
            effect: FormattingEffect::KuntenPolicy(KuntenPolicy {
                marks,
                scope,
                exclusions,
                repeat_following: body.contains("以下同じ"),
                vertical_rule_right: body.contains("右横に縦棒"),
            }),
            selection: None,
        }],
    })
}

fn source_slice<'a>(source: &SourceBody<'a>, span: Span) -> &'a str {
    &source.text[(span.start - source.start) as usize..(span.end - source.start) as usize]
}

fn quoted_spans(source: &SourceBody<'_>, body: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut open = None;
    for (offset, character) in body.char_indices() {
        match (character, open) {
            ('「', None) => open = Some(offset + character.len_utf8()),
            ('」', Some(start)) => {
                spans.push(source.span(&body[start..offset]));
                open = None;
            }
            _ => {}
        }
    }
    spans
}

fn compact_right_instruction(source: &SourceBody<'_>, body: &str) -> Option<FormattingInstruction> {
    let (operand, tail) = source.quoted(body)?;
    (tail == "はポイント小さく右寄せ").then(|| FormattingInstruction {
        operations: vec![
            FormattingOperation {
                source_span: source.span(body),
                selector: FormattingSelector::Quoted(operand),
                effect: FormattingEffect::Attribute(ForwardAttr::QualitativeFontSize(
                    QualitativeFontSize::Smaller,
                )),
                selection: None,
            },
            FormattingOperation {
                source_span: source.span(body),
                selector: FormattingSelector::Quoted(operand),
                effect: FormattingEffect::Attribute(ForwardAttr::AlignEnd { offset: 0 }),
                selection: None,
            },
        ],
    })
}

fn line_foot_instruction(source: &SourceBody<'_>, body: &str) -> Option<FormattingInstruction> {
    let rest = body.strip_prefix("この行はポイントを下げ、")?;
    let (selector, rest) = if rest.starts_with('「') {
        let (operand, tail) = source.quoted(rest)?;
        (
            FormattingSelector::Quoted(operand),
            tail.strip_prefix('は')?,
        )
    } else {
        (FormattingSelector::Unresolved, rest.strip_prefix('は')?)
    };
    let rest = rest.strip_prefix("地より")?;
    let (offset, tail) = ordinal(rest)?;
    if tail != "字上げ" || offset.get() > u32::from(u8::MAX) {
        return None;
    }
    let offset = u8::try_from(offset.get()).ok()?;
    Some(FormattingInstruction {
        operations: vec![
            FormattingOperation {
                source_span: source.span(body),
                selector: FormattingSelector::PrecedingText,
                effect: FormattingEffect::Attribute(ForwardAttr::QualitativeFontSize(
                    QualitativeFontSize::Smaller,
                )),
                selection: None,
            },
            FormattingOperation {
                source_span: source.span(body),
                selector,
                effect: FormattingEffect::Attribute(ForwardAttr::AlignEnd { offset }),
                selection: None,
            },
        ],
    })
}

fn small_contracting_sound_instruction(
    source: &SourceBody<'_>,
    body: &str,
) -> Option<FormattingInstruction> {
    let rest = body.strip_prefix("ここのみ拗音が小さい字")?;
    let (operand, tail) = source.quoted(rest)?;
    (tail == "になっている").then(|| FormattingInstruction {
        operations: vec![FormattingOperation {
            source_span: source.span(body),
            selector: FormattingSelector::Quoted(operand),
            effect: FormattingEffect::Attribute(ForwardAttr::FontSizeAbsolute(AbsoluteSize::Small)),
            selection: None,
        }],
    })
}

fn subtarget_instruction(source: &SourceBody<'_>, body: &str) -> Option<FormattingInstruction> {
    let (enclosing, tail) = source.quoted(body)?;
    let (operand, tail) = tail
        .strip_prefix('の')
        .and_then(|tail| source.quoted(tail))?;
    let effect = tail
        .strip_prefix('は')
        .and_then(|tail| source.effect(tail))?;
    Some(FormattingInstruction {
        operations: vec![FormattingOperation {
            source_span: source.span(body),
            selector: FormattingSelector::Subtarget { enclosing, operand },
            effect,
            selection: None,
        }],
    })
}

fn spatial_instruction(source: &SourceBody<'_>, body: &str) -> Option<FormattingInstruction> {
    let (target, tail) = source.quoted(body)?;
    let tail = tail.strip_prefix('は')?;
    let (placement, tail) = if let Some(tail) = tail.strip_prefix('、') {
        let (preceding, tail) = source.quoted(tail)?;
        let (following, tail) = tail
            .strip_prefix('と')
            .and_then(|tail| source.quoted(tail))?;
        (
            FormattingPlacement::Between {
                preceding,
                following,
            },
            tail.strip_prefix("の間に挟まれるような形でポイントを下げて")?,
        )
    } else {
        let (anchor, tail) = source.quoted(tail)?;
        (
            FormattingPlacement::Below(anchor),
            tail.strip_prefix("の下にポイントを下げて")?,
        )
    };
    let (lines, tail) = line_count(tail)?;
    let parentheses_lines = if tail.is_empty() {
        None
    } else {
        let tail = tail.strip_prefix("、カッコはその")?;
        let number = tail.strip_suffix("行を括る形で")?;
        Some(NonZeroU32::new(u32::from(japanese_u8(number)?))?)
    };
    Some(FormattingInstruction {
        operations: vec![FormattingOperation {
            source_span: source.span(body),
            selector: FormattingSelector::Quoted(target),
            effect: FormattingEffect::ReducedPointLines {
                placement,
                lines,
                parentheses_lines,
            },
            selection: None,
        }],
    })
}

fn line_count(text: &str) -> Option<(NonZeroU32, &str)> {
    let end = text.find("行で")?;
    let value = u32::from(japanese_u8(&text[..end])?);
    Some((NonZeroU32::new(value)?, &text[end + "行で".len()..]))
}

fn supported_single(operation: &FormattingOperation, source: &str) -> bool {
    matches!(operation.selector, FormattingSelector::Digits)
        || matches!(operation.selector, FormattingSelector::Subtarget { .. })
        || matches!(operation.selector, FormattingSelector::AllQuoted(_))
        || matches!(operation.selector, FormattingSelector::Quoted(span)
            if span.start.checked_sub(operation.source_span.start)
                .and_then(|start| source.get(start as usize..))
                .is_some_and(|operand| operand.starts_with('「')))
        || matches!(operation.effect, FormattingEffect::UpperRelativeTo { .. })
        || matches!(operation.effect, FormattingEffect::Rotation { .. })
        || matches!(
            (&operation.selector, &operation.effect),
            (
                FormattingSelector::Quoted(_) | FormattingSelector::EachQuoted(_),
                FormattingEffect::Attribute(ForwardAttr::CombineUpright | ForwardAttr::Fraction)
            )
        )
        || matches!(
            (&operation.selector, &operation.effect),
            (
                FormattingSelector::Parenthesized(_),
                FormattingEffect::Layout {
                    align_end: true,
                    ..
                }
            )
        )
}

fn heading_typography(body: &str) -> Option<HeadingTypography> {
    match body {
        "見出し文字" => Some(HeadingTypography::Heading),
        "小見出し文字" => Some(HeadingTypography::SmallHeading),
        _ => None,
    }
}

fn layout_policy(
    source: &SourceBody<'_>,
    body: &str,
) -> Option<(FormattingSelector, u8, bool, bool)> {
    if matches!(body, "以下地付き" | "以下、地付き" | "以下の文章は地付き") {
        return Some((FormattingSelector::FollowingText, 0, false, true));
    }
    if let Some(value) = body
        .strip_prefix("改行ごとに")
        .and_then(|rest| rest.strip_suffix("字下げ"))
        .and_then(japanese_u8)
    {
        return Some((FormattingSelector::EachLine, value, false, false));
    }
    let (operand, tail) = source.quoted(body)?;
    if let Some(value) = tail
        .strip_prefix("全体、天より")
        .and_then(|rest| rest.strip_suffix("字下げ"))
        .and_then(japanese_u8)
    {
        return Some((
            FormattingSelector::QuotedWhole(operand),
            value,
            false,
            false,
        ));
    }
    None
}

fn japanese_u8(text: &str) -> Option<u8> {
    if let Ok(value) = text.parse::<u8>() {
        return Some(value);
    }
    let digits = text
        .chars()
        .map(|character| match character {
            '０'..='９' => Some(character as u32 - '０' as u32),
            _ => None,
        })
        .collect::<Option<Vec<_>>>();
    if let Some(digits) = digits.filter(|digits| !digits.is_empty()) {
        return digits.into_iter().try_fold(0_u8, |value, digit| {
            value
                .checked_mul(10)?
                .checked_add(u8::try_from(digit).ok()?)
        });
    }
    let digit = |character| match character {
        '一' => Some(1_u8),
        '二' => Some(2),
        '三' => Some(3),
        '四' => Some(4),
        '五' => Some(5),
        '六' => Some(6),
        '七' => Some(7),
        '八' => Some(8),
        '九' => Some(9),
        _ => None,
    };
    match text.chars().collect::<Vec<_>>().as_slice() {
        ['十'] => Some(10),
        ['十', ones] => Some(10 + digit(*ones)?),
        [tens, '十'] => Some(digit(*tens)? * 10),
        [tens, '十', ones] => Some(digit(*tens)? * 10 + digit(*ones)?),
        [single] => digit(*single),
        _ => None,
    }
}

pub(super) fn bind(instruction: &mut FormattingInstruction, source: &str, text: Span) {
    if bind_word_endings(instruction, source, text) {
        return;
    }
    let body = &source[text.start as usize..text.end as usize];
    let line_setting = instruction
        .operations
        .iter()
        .any(|operation| matches!(operation.effect, FormattingEffect::LineSetting { .. }));
    let occurrence_operand = |operation: &FormattingOperation| match operation.selector {
        FormattingSelector::Occurrence { operand, .. } => {
            Some(&source[operand.start as usize..operand.end as usize])
        }
        _ => None,
    };
    let listed_occurrences = instruction.operations.len() >= 2
        && instruction.operations.iter().all(|operation| {
            occurrence_operand(operation).is_some()
                && occurrence_operand(operation) == occurrence_operand(&instruction.operations[0])
        });
    let explicit_scope = (line_setting
        || listed_occurrences
        || instruction
            .operations
            .iter()
            .any(|operation| matches!(operation.selector, FormattingSelector::PrecedingText))
        || instruction
            .operations
            .iter()
            .all(|operation| matches!(operation.selector, FormattingSelector::AllQuoted(_))))
    .then_some(text)
    .or_else(|| {
        instruction
            .operations
            .iter()
            .filter_map(|operation| match &operation.selector {
                FormattingSelector::Quoted(operand)
                | FormattingSelector::Parenthesized(operand) => Some(operand),
                FormattingSelector::Subtarget { enclosing, .. } => Some(enclosing),
                _ => None,
            })
            .filter(|operand| body.ends_with(&source[operand.start as usize..operand.end as usize]))
            .map(|operand| Span::new(text.end - (operand.end - operand.start), text.end))
            .min_by_key(|scope| scope.start)
    });
    let Some((scope, allow_repeated_references)) = explicit_scope
        .map(|scope| (scope, true))
        .or_else(|| inferred_scope(instruction, source, text).map(|scope| (scope, false)))
    else {
        bind_indexed_template(instruction, source, text);
        return;
    };
    for operation in &mut instruction.operations {
        operation.selection = select(
            source,
            scope,
            &operation.selector,
            allow_repeated_references,
        )
        .filter(|targets| !targets.is_empty())
        .filter(|targets| {
            !matches!(operation.effect, FormattingEffect::LineSetting { .. })
                || adjacent_line_setting_targets(source, targets)
        })
        .map(|targets| FormattingSelection { scope, targets });
        if let FormattingEffect::UpperRelativeTo { operand, target } = &mut operation.effect {
            *target = operation.selection.as_ref().and_then(|selection| {
                let [selected] = selection.targets.as_slice() else {
                    return None;
                };
                let start = selected.start.checked_sub(operand.end - operand.start)?;
                if start < text.start {
                    return None;
                }
                let preceding = source.get(start as usize..selected.start as usize)?;
                (preceding == &source[operand.start as usize..operand.end as usize])
                    .then_some(Span::new(start, selected.start))
            });
        }
        if let FormattingEffect::RelativePlacement(FormattingPlacement::BelowDayRange {
            start_day,
            end_day,
            start_target,
            end_target,
            ..
        }) = &mut operation.effect
        {
            (*start_target, *end_target) =
                day_range_targets(source, text.start, *start_day, *end_day)
                    .map_or((None, None), |(start, end)| (Some(start), Some(end)));
        }
        if let FormattingEffect::LineStartAlignment(LineStartAlignment::RelativeLine {
            lines_before,
            reference_target,
        }) = &mut operation.effect
        {
            *reference_target = preceding_line(source, text.start, lines_before.get());
        }
    }
}

/// Bind a word-ending ligature. Each word resolves to its nearest preceding
/// occurrence on the marker's line, which must end a word, and the operation
/// selects the ending of that occurrence.
fn bind_word_endings(instruction: &mut FormattingInstruction, source: &str, text: Span) -> bool {
    if !instruction.operations.iter().all(|operation| {
        matches!(
            (&operation.selector, &operation.effect),
            (
                FormattingSelector::Subtarget { .. },
                FormattingEffect::Ligature { .. }
            )
        )
    }) {
        return false;
    }
    let line_start = source[..text.end as usize]
        .rfind('\n')
        .map_or(0, |offset| offset + 1);
    let start = text
        .start
        .max(u32::try_from(line_start).expect("source offset fits u32"));
    let line = &source[start as usize..text.end as usize];
    for operation in &mut instruction.operations {
        let FormattingSelector::Subtarget { enclosing, operand } = operation.selector else {
            continue;
        };
        let word = &source[enclosing.start as usize..enclosing.end as usize];
        operation.selection = line
            .rfind(word)
            .map(|offset| offset + word.len())
            .filter(|&end| {
                !line[end..]
                    .chars()
                    .next()
                    .is_some_and(char::is_alphanumeric)
            })
            .map(|end| {
                let end = start + u32::try_from(end).expect("source offset fits u32");
                FormattingSelection {
                    scope: Span::new(end - (enclosing.end - enclosing.start), end),
                    targets: vec![Span::new(end - (operand.end - operand.start), end)],
                }
            });
    }
    true
}

/// Runs set as lines within one line follow one another, separated at most by
/// a slash or by the parentheses that close one run and open the next.
fn adjacent_line_setting_targets(source: &str, targets: &[Span]) -> bool {
    targets.windows(2).all(|pair| {
        pair[0].end <= pair[1].start
            && matches!(
                &source[pair[0].end as usize..pair[1].start as usize],
                "" | "／" | "）（"
            )
    })
}

/// Bind a line setting stated before its text to the rest of the marker's line.
pub(super) fn bind_following_line_rest(
    instruction: &mut FormattingInstruction,
    source: &str,
    after: u32,
) {
    if !instruction
        .operations
        .iter()
        .any(|operation| matches!(operation.effect, FormattingEffect::LineSetting { .. }))
        || !instruction
            .operations
            .iter()
            .all(|operation| matches!(operation.selector, FormattingSelector::FollowingText))
    {
        return;
    }
    let Some(remainder) = source.get(after as usize..) else {
        return;
    };
    let line = remainder.split('\n').next().unwrap_or("");
    let line = line.strip_suffix('\r').unwrap_or(line);
    if line.is_empty() || line.contains("［＃") {
        return;
    }
    let target = Span::new(
        after,
        after + u32::try_from(line.len()).expect("source length fits u32"),
    );
    for operation in &mut instruction.operations {
        operation.selection = Some(FormattingSelection {
            scope: target,
            targets: vec![target],
        });
    }
}

pub(super) fn bind_following_lines(
    instruction: &mut FormattingInstruction,
    source: &str,
    after: u32,
) {
    let Some(lines) =
        instruction
            .operations
            .iter()
            .find_map(|operation| match operation.selector {
                FormattingSelector::FollowingLines { lines } => Some(lines),
                _ => None,
            })
    else {
        return;
    };
    if instruction.operations.iter().any(|operation| {
        !matches!(operation.selector, FormattingSelector::FollowingLines { lines: candidate } if candidate == lines)
    }) {
        return;
    }
    let Some(remainder) = source.get(after as usize..) else {
        return;
    };
    let Some(first_break) = remainder.find('\n') else {
        return;
    };
    let mut start = after as usize + first_break + 1;
    let mut targets = Vec::with_capacity(lines.get() as usize);
    for _ in 0..lines.get() {
        let remainder = &source[start..];
        let end = remainder
            .find('\n')
            .map_or(source.len(), |offset| start + offset);
        let content_end =
            end - usize::from(source.as_bytes().get(end.wrapping_sub(1)) == Some(&b'\r'));
        if content_end == start {
            return;
        }
        targets.push(Span::new(
            u32::try_from(start).expect("source offset fits u32"),
            u32::try_from(content_end).expect("source offset fits u32"),
        ));
        start = end.saturating_add(usize::from(end < source.len()));
    }
    let scope = Span::new(
        targets[0].start,
        targets.last().expect("targets are nonempty").end,
    );
    let selected = &source[scope.start as usize..scope.end as usize];
    if instruction.operations.iter().any(|operation| {
        quoted_range_ends(
            &source[operation.source_span.start as usize..operation.source_span.end as usize],
        )
        .is_some_and(|(first, last)| {
            !selected.trim_start_matches([' ', '　']).starts_with(first)
                || !selected.ends_with(last)
        })
    }) {
        return;
    }
    for operation in &mut instruction.operations {
        operation.selection = Some(FormattingSelection {
            scope,
            targets: targets.clone(),
        });
    }
}

fn preceding_line(source: &str, before: u32, lines_before: u32) -> Option<Span> {
    let mut start = usize::try_from(before).ok()?;
    start = source
        .get(..start)?
        .rfind('\n')
        .map_or(0, |offset| offset + 1);
    for index in 0..lines_before {
        let previous = source.get(..start)?.strip_suffix('\n')?;
        let end = previous.len();
        start = previous.rfind('\n').map_or(0, |offset| offset + 1);
        if end < start {
            return None;
        }
        if index == lines_before - 1 {
            let end =
                end - usize::from(previous.as_bytes().get(end.wrapping_sub(1)) == Some(&b'\r'));
            return Some(Span::new(
                u32::try_from(start).ok()?,
                u32::try_from(end).ok()?,
            ));
        }
    }
    None
}

fn day_range_targets(
    source: &str,
    before: u32,
    start_day: NonZeroU32,
    end_day: NonZeroU32,
) -> Option<(Span, Span)> {
    let before = source.get(..usize::try_from(before).ok()?)?;
    let mut offset = 0usize;
    let mut labelled = Vec::new();
    for (line_number, line) in before.split_inclusive('\n').enumerate() {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let content = content.strip_suffix('\r').unwrap_or(content);
        let leading = content.len() - content.trim_start_matches([' ', '\t', '　']).len();
        let text = &content[leading..];
        let digit_end = text
            .char_indices()
            .take_while(|(_, character)| {
                character.is_ascii_digit() || ('０'..='９').contains(character)
            })
            .map(|(index, character)| index + character.len_utf8())
            .last()
            .unwrap_or(0);
        if digit_end > 0
            && let Some(day) = japanese_u8(&text[..digit_end])
            && (text[digit_end..].starts_with('日')
                || (text[digit_end..].starts_with("［＃") && text[digit_end..].contains("］日")))
        {
            let start = offset + leading;
            labelled.push((
                line_number,
                NonZeroU32::new(u32::from(day))?,
                Span::new(
                    u32::try_from(start).ok()?,
                    u32::try_from(start + digit_end).ok()?,
                ),
            ));
        }
        offset += line.len();
    }
    let count = usize::try_from(end_day.get() - start_day.get() + 1).ok()?;
    labelled.windows(count).rev().find_map(|window| {
        window
            .iter()
            .enumerate()
            .all(|(index, (line, day, _))| {
                *line == window[0].0 + index
                    && day.get() == start_day.get() + u32::try_from(index).ok().unwrap_or(u32::MAX)
            })
            .then(|| (window.first().unwrap().2, window.last().unwrap().2))
    })
}

fn adjacent_quoted_scope(
    instruction: &FormattingInstruction,
    source: &str,
    text: Span,
) -> Option<Span> {
    instruction.operations.iter().find_map(|operation| {
        let FormattingSelector::EachQuoted(operands) = &operation.selector else {
            return None;
        };
        if operands.len() < 2 {
            return None;
        }
        let targets = ordered_quoted_targets(source, text, operands, false)?;
        let first = *targets.first()?;
        let mut end = first.end;
        for target in &targets[1..] {
            if target.start < end
                || !source[end as usize..target.start as usize]
                    .chars()
                    .all(adjacent_target_separator)
            {
                return None;
            }
            end = target.end;
        }
        (source[end as usize..text.end as usize]
            .chars()
            .all(adjacent_target_separator))
        .then_some(Span::new(first.start, text.end))
    })
}

fn inferred_scope(instruction: &FormattingInstruction, source: &str, text: Span) -> Option<Span> {
    adjacent_quoted_scope(instruction, source, text)
        .or_else(|| position_word_scope(instruction, source, text))
        .or_else(|| equation_scope(instruction, source, text))
        .or_else(|| listed_fraction_scope(instruction, source, text))
}

/// The marker's line before the marker, for a 分数 note that lists several
/// quoted operands each of which occurs exactly once there.
fn listed_fraction_scope(
    instruction: &FormattingInstruction,
    source: &str,
    text: Span,
) -> Option<Span> {
    let [operation] = instruction.operations.as_slice() else {
        return None;
    };
    let (
        FormattingSelector::EachQuoted(operands),
        FormattingEffect::Attribute(ForwardAttr::Fraction),
    ) = (&operation.selector, &operation.effect)
    else {
        return None;
    };
    let line_start = source[..text.end as usize]
        .rfind('\n')
        .map_or(0, |offset| offset + 1);
    let scope = Span::new(text.start.max(u32::try_from(line_start).ok()?), text.end);
    let line = &source[scope.start as usize..scope.end as usize];
    (operands.len() >= 2
        && operands.iter().all(|operand| {
            line.matches(&source[operand.start as usize..operand.end as usize])
                .count()
                == 1
        }))
    .then_some(scope)
}

fn adjacent_target_separator(character: char) -> bool {
    character.is_whitespace() || matches!(character, '│' | '｜')
}

fn position_word_scope(
    instruction: &FormattingInstruction,
    source: &str,
    text: Span,
) -> Option<Span> {
    let maximum =
        instruction
            .operations
            .iter()
            .try_fold(0, |maximum, operation| match operation.selector {
                FormattingSelector::CharacterPosition { ordinal, .. } => {
                    Some(maximum.max(ordinal.get()))
                }
                _ => None,
            })?;
    let body = &source[text.start as usize..text.end as usize];
    let word = body.rsplit(char::is_whitespace).next()?;
    if word.is_empty()
        || !word.chars().all(char::is_alphanumeric)
        || word.chars().count() < usize::try_from(maximum).ok()?
    {
        return None;
    }
    let length = u32::try_from(word.len()).expect("source length fits u32");
    let start = text.end - length;
    if start == text.start
        && start != 0
        && !source[..start as usize]
            .chars()
            .next_back()
            .is_some_and(|ch| ch.is_whitespace() || matches!(ch, '｜' | '《'))
    {
        return None;
    }
    Some(Span::new(start, text.end))
}

fn bind_indexed_template(instruction: &mut FormattingInstruction, source: &str, text: Span) {
    let Some(operand) = instruction.operations.iter().find_map(|operation| {
        if let FormattingSelector::EachQuoted(operands) = &operation.selector
            && let [operand] = operands.as_slice()
        {
            Some(*operand)
        } else {
            None
        }
    }) else {
        return;
    };
    let template = &source[operand.start as usize..operand.end as usize];
    let Some(prefix) = template.strip_suffix(['n', 'ｎ']) else {
        return;
    };
    if prefix.chars().count() != 1 || !prefix.chars().all(latin_letter) {
        return;
    }
    if !instruction
        .operations
        .iter()
        .all(|operation| match &operation.selector {
            FormattingSelector::EachQuoted(operands) => matches!(operands.as_slice(), [span]
            if &source[span.start as usize..span.end as usize] == template),
            FormattingSelector::Digits => true,
            _ => false,
        })
    {
        return;
    }
    let body = &source[text.start as usize..text.end as usize];
    let series = body
        .rsplit(|ch| !latin_letter(ch) && !decimal_digit(ch))
        .next()
        .unwrap_or("");
    let start = text.end - u32::try_from(series.len()).expect("source length fits u32");
    let preceding_run = &body[..body.len() - series.len()];
    let before_gap = preceding_run.trim_end_matches([' ', '\t', '　']);
    if before_gap.len() != preceding_run.len()
        && before_gap.chars().next_back().is_some_and(decimal_digit)
    {
        return;
    }
    if start != 0 {
        let preceding = source[..start as usize].chars().next_back().unwrap();
        let boundary = preceding.is_whitespace()
            || (start == text.start && matches!(preceding, '｜' | '《'))
            || (start > text.start
                && matches!(
                    ab_aozora_syntax::ruby_base_class(preceding),
                    Some(
                        ab_aozora_syntax::RubyBaseClass::Kanji
                            | ab_aozora_syntax::RubyBaseClass::Hiragana
                            | ab_aozora_syntax::RubyBaseClass::Katakana
                    )
                ));
        if !boundary {
            return;
        }
    }
    let Some(instances) = indexed_instances(series, prefix, start) else {
        return;
    };
    let prefix_length = u32::try_from(prefix.len()).expect("character length fits u32");
    let indices = instances
        .iter()
        .map(|span| Span::new(span.start + prefix_length, span.end))
        .collect::<Vec<_>>();
    for operation in &mut instruction.operations {
        let targets = if matches!(operation.selector, FormattingSelector::Digits) {
            indices.clone()
        } else {
            instances.clone()
        };
        operation.selection = Some(FormattingSelection {
            scope: Span::new(start, text.end),
            targets,
        });
    }
}

fn indexed_instances(mut series: &str, prefix: &str, mut start: u32) -> Option<Vec<Span>> {
    let mut targets = Vec::new();
    while !series.is_empty() {
        let rest = series.strip_prefix(prefix)?;
        let digits = rest
            .chars()
            .take_while(|ch| decimal_digit(*ch))
            .map(char::len_utf8)
            .sum::<usize>();
        if digits == 0 {
            return None;
        }
        let length = prefix.len() + digits;
        let end = start + u32::try_from(length).expect("source length fits u32");
        targets.push(Span::new(start, end));
        start = end;
        series = &series[length..];
    }
    (targets.len() >= 2).then_some(targets)
}

fn latin_letter(ch: char) -> bool {
    matches!(ch, 'A'..='Z' | 'a'..='z' | 'Ａ'..='Ｚ' | 'ａ'..='ｚ')
}

fn decimal_digit(ch: char) -> bool {
    matches!(ch, '0'..='9' | '０'..='９')
}

fn equation_scope(instruction: &FormattingInstruction, source: &str, text: Span) -> Option<Span> {
    if !instruction.operations.iter().any(|op| {
        matches!(
            op.selector,
            FormattingSelector::ArabicDigitsAfter(_) | FormattingSelector::AllQuoted(_)
        )
    }) || !instruction.operations.iter().all(|op| {
        matches!(
            op.selector,
            FormattingSelector::Quoted(_)
                | FormattingSelector::EachQuoted(_)
                | FormattingSelector::AllQuoted(_)
                | FormattingSelector::ArabicDigitsAfter(_)
        )
    }) {
        return None;
    }
    let body = &source[text.start as usize..text.end as usize];
    let boundary = body.char_indices().rev().find(|(_, ch)| {
        matches!(ch, '\r' | '\n')
            || matches!(
                ab_aozora_syntax::ruby_base_class(*ch),
                Some(
                    ab_aozora_syntax::RubyBaseClass::Kanji
                        | ab_aozora_syntax::RubyBaseClass::Hiragana
                        | ab_aozora_syntax::RubyBaseClass::Katakana
                )
            )
    });
    if boundary.is_none()
        && text.start != 0
        && !source[..text.start as usize]
            .chars()
            .next_back()
            .is_some_and(|ch| ch.is_whitespace() || matches!(ch, '｜' | '《'))
    {
        return None;
    }
    let candidate = &body[boundary.map_or(0, |(offset, ch)| offset + ch.len_utf8())..];
    let candidate = candidate.trim_start_matches(char::is_whitespace);
    if candidate
        .chars()
        .next_back()
        .is_none_or(char::is_whitespace)
        || !complete_equations(candidate)
    {
        return None;
    }
    Some(Span::new(
        text.end - u32::try_from(candidate.len()).expect("source length fits u32"),
        text.end,
    ))
}

fn equation_letter(ch: char) -> bool {
    ch.is_alphabetic()
        && matches!(
            ab_aozora_syntax::ruby_base_class(ch),
            Some(
                ab_aozora_syntax::RubyBaseClass::Latin
                    | ab_aozora_syntax::RubyBaseClass::Greek
                    | ab_aozora_syntax::RubyBaseClass::Cyrillic
            )
        )
}

fn complete_equations(source: &str) -> bool {
    let mut chars = source.chars().peekable();
    let mut closes = Vec::new();
    let mut operand = true;
    let mut equality = false;
    while let Some(ch) = chars.next() {
        match ch {
            ch if ch.is_whitespace() => {}
            '(' | '（' if operand => {
                closes.push(if ch == '(' { ')' } else { '）' });
            }
            ')' | '）' if !operand => {
                if closes.pop() != Some(ch) {
                    return false;
                }
            }
            '=' | '＝' if !operand => {
                equality = true;
                operand = true;
            }
            ';' | '；' if !operand && equality && closes.is_empty() => {
                equality = false;
                operand = true;
            }
            '+' | '＋' | '-' | '−' | '－' => {
                operand = true;
            }
            '*' | '＊' | '×' | '/' | '／' | '÷' if !operand => {
                operand = true;
            }
            '…' if operand => {
                while chars.peek() == Some(&'…') {
                    chars.next();
                }
                operand = false;
            }
            ch if operand && equation_letter(ch) => {
                while chars
                    .peek()
                    .is_some_and(|ch| equation_letter(*ch) || decimal_digit(*ch))
                {
                    chars.next();
                }
                operand = false;
            }
            ch if operand && decimal_digit(ch) => {
                while chars.peek().is_some_and(|ch| decimal_digit(*ch)) {
                    chars.next();
                }
                if chars.peek().is_some_and(|ch| matches!(ch, '.' | '．')) {
                    chars.next();
                    if !chars.peek().is_some_and(|ch| decimal_digit(*ch)) {
                        return false;
                    }
                    while chars.peek().is_some_and(|ch| decimal_digit(*ch)) {
                        chars.next();
                    }
                }
                operand = false;
            }
            _ => return false,
        }
    }
    !operand && equality && closes.is_empty()
}

fn conditional_digits(source: &str, scope: Span, predecessor: &FormattingPredecessor) -> Vec<Span> {
    let body = &source[scope.start as usize..scope.end as usize];
    let mut chars = body.char_indices().peekable();
    let mut previous = None;
    let mut targets = Vec::new();
    while let Some((offset, ch)) = chars.next() {
        let matches = previous.is_some_and(|previous| match predecessor {
            FormattingPredecessor::Alphabetic => matches!(
                ab_aozora_syntax::ruby_base_class(previous),
                Some(ab_aozora_syntax::RubyBaseClass::Latin)
            ),
            FormattingPredecessor::NamedCharacters(operands) => operands
                .iter()
                .any(|span| source[span.start as usize..span.end as usize] == previous.to_string()),
        });
        if decimal_digit(ch) && matches {
            let mut end = offset + ch.len_utf8();
            while chars.peek().is_some_and(|(_, ch)| decimal_digit(*ch)) {
                let (offset, ch) = chars.next().unwrap();
                end = offset + ch.len_utf8();
            }
            targets.push(Span::new(
                scope.start + u32::try_from(offset).expect("source offset fits u32"),
                scope.start + u32::try_from(end).expect("source offset fits u32"),
            ));
        }
        previous = Some(ch);
    }
    targets
}

fn select(
    source: &str,
    scope: Span,
    selector: &FormattingSelector,
    allow_repeated_references: bool,
) -> Option<Vec<Span>> {
    let body = &source[scope.start as usize..scope.end as usize];
    let occurrences = |operand: Span| {
        let spelling = &source[operand.start as usize..operand.end as usize];
        body.char_indices()
            .filter(|(offset, _)| body[*offset..].starts_with(spelling))
            .map(|(offset, _)| {
                let start = scope.start + u32::try_from(offset).expect("source offset fits u32");
                Span::new(start, start + operand.end - operand.start)
            })
            .collect::<Vec<_>>()
    };
    let unique = |operand| {
        let matches = occurrences(operand);
        (matches.len() == 1).then(|| matches[0])
    };
    match selector {
        FormattingSelector::PrecedingText => Some(vec![scope]),
        FormattingSelector::Quoted(operand)
        | FormattingSelector::Parenthesized(operand)
        | FormattingSelector::Named(operand)
        | FormattingSelector::Subtarget { operand, .. } => Some(vec![unique(*operand)?]),
        FormattingSelector::EachQuoted(operands) => {
            ordered_quoted_targets(source, scope, operands, allow_repeated_references)
        }
        FormattingSelector::AllQuoted(operand) => Some(occurrences(*operand)),
        FormattingSelector::Occurrence { ordinal, operand } => Some(vec![
            *occurrences(*operand).get((ordinal.get() - 1) as usize)?,
        ]),
        FormattingSelector::CharacterPosition { ordinal, operand } => {
            let (offset, ch) = body.char_indices().nth((ordinal.get() - 1) as usize)?;
            if source[operand.start as usize..operand.end as usize] != ch.to_string() {
                return None;
            }
            let start = scope.start + u32::try_from(offset).expect("source offset fits u32");
            Some(vec![Span::new(
                start,
                start + u32::try_from(ch.len_utf8()).expect("character length fits u32"),
            )])
        }
        FormattingSelector::ArabicDigitsAfter(predecessor) => {
            Some(conditional_digits(source, scope, predecessor))
        }
        FormattingSelector::Unresolved
        | FormattingSelector::Digits
        | FormattingSelector::EachLine
        | FormattingSelector::FollowingText
        | FormattingSelector::FollowingLines { .. }
        | FormattingSelector::QuotedWhole(_) => None,
    }
}

fn ordered_quoted_targets(
    source: &str,
    scope: Span,
    operands: &[Span],
    allow_repeated_references: bool,
) -> Option<Vec<Span>> {
    let body = source.get(scope.start as usize..scope.end as usize)?;
    let spellings = operands
        .iter()
        .map(|operand| source.get(operand.start as usize..operand.end as usize))
        .collect::<Option<Vec<_>>>()?;
    let repeated = spellings
        .iter()
        .enumerate()
        .any(|(index, spelling)| spellings[..index].contains(spelling));
    if repeated {
        if !allow_repeated_references {
            return None;
        }
        return operands
            .iter()
            .map(|operand| {
                let spelling = source.get(operand.start as usize..operand.end as usize)?;
                let mut matches = body.match_indices(spelling);
                let (offset, _) = matches.next()?;
                if matches.next().is_some() {
                    return None;
                }
                let start = scope.start + u32::try_from(offset).ok()?;
                Some(Span::new(start, start + operand.end - operand.start))
            })
            .collect();
    }
    let mut cursor = body.len();
    let mut targets = Vec::with_capacity(operands.len());
    for operand in operands.iter().rev() {
        let spelling = source.get(operand.start as usize..operand.end as usize)?;
        let offset = body.get(..cursor)?.rfind(spelling)?;
        let start = scope.start + u32::try_from(offset).ok()?;
        targets.push(Span::new(start, start + operand.end - operand.start));
        cursor = offset;
    }
    targets.reverse();
    Some(targets)
}
