//! Notes that supply diacritics to Latin or Greek letters of the preceding
//! text: `［＃一つ目の「e」は「´」付き］`, `［＃Kuchenのuにウムラウト（¨）］`,
//! `［＃ηに帯気、ωに曲アクセント］`, `［＃σはファイナルシグマ］`.
//!
//! A note is one or more clauses separated by `。` or `、`. A clause selects
//! letters, optionally within a quoted or bare word, and states the marks
//! they carry. A clause without a word applies within the word of the
//! preceding clause, or, in the first clause, within the letter run that
//! ends at the note.

use core::iter;

use unicode_normalization::UnicodeNormalization;

/// Byte offset in `prefix` where the text that the note addresses begins.
///
/// `prefix` is the source before the note. The result is `None` when `body`
/// is not a supplied-mark note or a letter it selects is absent from the
/// current line. The addressed text runs from the offset to the end of
/// `prefix` and holds no notation delimiter.
#[must_use]
pub fn supplied_mark_target_start(prefix: &str, body: &str) -> Option<usize> {
    let clauses = parse_note(body)?;
    let floor = prefix
        .rfind(['\n', '［', '］', '《', '》', '｜', '※', '〔', '〕'])
        .map_or(0, |index| {
            index + prefix[index..].chars().next().map_or(0, char::len_utf8)
        });
    let text = &prefix[floor..];
    let resolved = resolve(text, &clauses)?;
    let start = floor + resolved.start;
    compose(&prefix[start..], body).map(|_| start)
}

/// `text` with every letter the note `body` selects composed with its marks,
/// or `None` when `body` is not a supplied-mark note or a selection fails.
#[must_use]
pub(super) fn compose(text: &str, body: &str) -> Option<String> {
    let clauses = parse_note(body)?;
    let resolved = resolve(text, &clauses)?;
    let mut out = String::with_capacity(text.len() + 4);
    let mut last = 0;
    for (index, letter, mark) in resolved.letters {
        out.push_str(&text[last..index]);
        out.push(mark.apply(letter)?);
        last = index + letter.len_utf8();
    }
    out.push_str(&text[last..]);
    Some(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mark {
    /// Combining marks in source order, composed with the letter under NFC.
    Combining(&'static [char]),
    FinalSigma,
}

impl Mark {
    fn apply(self, letter: char) -> Option<char> {
        match self {
            Self::FinalSigma => (letter == 'σ').then_some('ς'),
            Self::Combining(marks) => {
                let mut composed = iter::once(letter).chain(marks.iter().copied()).nfc();
                let glyph = composed.next()?;
                composed.next().is_none().then_some(glyph)
            }
        }
    }
}

const ACUTE: &[char] = &['\u{301}'];
const GRAVE: &[char] = &['\u{300}'];
const DIAERESIS: &[char] = &['\u{308}'];
const CIRCUMFLEX: &[char] = &['\u{302}'];
const MACRON: &[char] = &['\u{304}'];
const ROUGH: &[char] = &['\u{314}'];
const SMOOTH: &[char] = &['\u{313}'];
const CIRCUMFLEX_GREEK: &[char] = &['\u{342}'];
const ROUGH_ACUTE: &[char] = &['\u{314}', '\u{301}'];
const SMOOTH_CIRCUMFLEX: &[char] = &['\u{313}', '\u{342}'];
const DOT_ABOVE: &[char] = &['\u{307}'];
const DOT_BELOW: &[char] = &['\u{323}'];

/// The quoted mark symbols of `「x」は「´」付き`.
fn quoted_mark(symbol: &str) -> Option<&'static [char]> {
    Some(match symbol {
        "´" => ACUTE,
        "｀" => GRAVE,
        "¨" => DIAERESIS,
        "^" => CIRCUMFLEX,
        _ => return None,
    })
}

/// The mark wording of `…は<mark>付き` and `…に<mark>`.
fn named_mark(name: &str) -> Option<&'static [char]> {
    Some(match name {
        "アクサン（´）"
        | "アクサン(´)"
        | "鋭アクセント"
        | "鋭アクセント（´）"
        | "アキュートアクセント"
        | "アキュートアクセント（´）" => ACUTE,
        "アクサン（｀）" | "accent grave（｀）" => GRAVE,
        "ウムラウト（¨）" => DIAERESIS,
        "長音記号" => MACRON,
        "帯気" | "帯気の気息記号（‘の上下が逆さまになったような記号）" => {
            ROUGH
        }
        "平息" => SMOOTH,
        "曲アクセント" => CIRCUMFLEX_GREEK,
        "帯気＋鋭アクセント" => ROUGH_ACUTE,
        "平息＋曲アクセント" => SMOOTH_CIRCUMFLEX,
        "上ドット" => DOT_ABOVE,
        "下ドット" => DOT_BELOW,
        _ => return None,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Scope<'a> {
    /// The word of the preceding clause, or the run that ends the text.
    Inherited,
    /// The nearest occurrence of each word; a word quoted with a final `-`
    /// is a stem hyphenated across a line in the base edition.
    Words(Vec<&'a str>),
    /// A word that consists of the letter alone.
    Standalone,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Select {
    /// The only occurrence; for a final sigma, the only word-final one.
    Unique,
    /// The listed occurrences, counted from one.
    Nth(Vec<usize>),
    Last,
    /// Every occurrence, which must number exactly this many.
    Every(usize),
    /// The character at this position, counted from one, is the letter.
    Position(usize),
    /// The scope ends with the letter.
    Final,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Target<'a> {
    scope: Scope<'a>,
    select: Select,
    letter: char,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Clause<'a> {
    targets: Vec<Target<'a>>,
    mark: Mark,
}

/// Parse `body`, also as repaired when it has an unmatched final `」` or an
/// enclosing pair of `「」`.
fn parse_note(body: &str) -> Option<Vec<Clause<'_>>> {
    let body = body.trim_start_matches([' ', '　']).trim_end();
    parse_clauses(body).or_else(|| {
        let opens = body.matches('「').count();
        let closes = body.matches('」').count();
        let inner = if closes > opens {
            body.strip_suffix('」')?
        } else {
            body.strip_prefix('「')?.strip_suffix('」')?
        };
        parse_clauses(inner)
    })
}

fn parse_clauses(body: &str) -> Option<Vec<Clause<'_>>> {
    let mut clauses = Vec::new();
    let mut rest = body;
    while !rest.is_empty() {
        let (clause, after) = parse_clause(rest)?;
        clauses.push(clause);
        rest = after;
        if !rest.is_empty() {
            rest = rest.strip_prefix(['。', '、'])?;
        }
    }
    (!clauses.is_empty()).then_some(clauses)
}

fn parse_clause(s: &str) -> Option<(Clause<'_>, &str)> {
    let mut targets = Vec::new();
    let mut rest = s;
    loop {
        let (target, after) = parse_target(rest)?;
        targets.push(target);
        let Some(next) = after.strip_prefix('と') else {
            rest = after;
            break;
        };
        rest = next;
    }
    let (mark, rest) = parse_mark(rest)?;
    Some((Clause { targets, mark }, rest))
}

fn parse_target(s: &str) -> Option<(Target<'_>, &str)> {
    let (scope, rest) = parse_scope(s);
    let (select, rest) = parse_select(rest);
    let (letter, rest) = parse_letter(rest)?;
    Some((
        Target {
            scope,
            select,
            letter,
        },
        rest,
    ))
}

fn parse_scope(s: &str) -> (Scope<'_>, &str) {
    if let Some(rest) = s.strip_prefix("単体の") {
        return (Scope::Standalone, rest);
    }
    let mut words = Vec::new();
    let mut rest = s;
    while let Some((word, after)) = quoted(rest) {
        if word.chars().count() < 2 {
            break;
        }
        words.push(word);
        if let Some(next) = after
            .strip_prefix('と')
            .filter(|next| next.starts_with('「'))
        {
            rest = next;
        } else if let Some(next) = after
            .strip_prefix("のうち、")
            .or_else(|| after.strip_prefix('の'))
        {
            return (Scope::Words(words), next);
        } else {
            break;
        }
    }
    let end = s
        .char_indices()
        .find(|(_, ch)| !is_letter(*ch))
        .map_or(s.len(), |(index, _)| index);
    if s[..end].chars().count() >= 2
        && let Some(rest) = s[end..].strip_prefix('の')
    {
        return (Scope::Words(vec![&s[..end]]), rest);
    }
    (Scope::Inherited, s)
}

fn parse_select(s: &str) -> (Select, &str) {
    for (prefix, select) in [
        ("最後の", Select::Last),
        ("最初の", Select::Nth(vec![1])),
        ("始めの", Select::Nth(vec![1])),
        ("語尾の", Select::Final),
        ("二つの", Select::Every(2)),
    ] {
        if let Some(rest) = s.strip_prefix(prefix) {
            return (select, rest);
        }
    }
    let mut ordinals = Vec::new();
    let mut rest = s;
    while let Some((ordinal, after)) =
        ["一", "二", "三"]
            .iter()
            .zip(1..)
            .find_map(|(numeral, value)| {
                Some((value, rest.strip_prefix(numeral)?.strip_prefix("つ目")?))
            })
    {
        ordinals.push(ordinal);
        if let Some(next) = after.strip_prefix('と') {
            rest = next;
        } else if let Some(next) = after.strip_prefix('の') {
            return (Select::Nth(ordinals), next);
        } else {
            break;
        }
    }
    if let Some((number, after)) = leading_number(s) {
        for (suffix, select) in [
            ("番目の", Select::Nth(vec![number])),
            ("文字目の", Select::Position(number)),
            ("つの", Select::Every(number)),
        ] {
            if let Some(rest) = after.strip_prefix(suffix) {
                return (select, rest);
            }
        }
    }
    (Select::Unique, s)
}

fn parse_letter(s: &str) -> Option<(char, &str)> {
    if let Some((quoted, rest)) = quoted(s) {
        let mut chars = quoted.chars();
        let letter = chars.next().filter(|ch| is_letter(*ch))?;
        return chars.next().is_none().then_some((letter, rest));
    }
    let letter = s.chars().next().filter(|ch| is_letter(*ch))?;
    let rest = &s[letter.len_utf8()..];
    rest.chars()
        .next()
        .is_some_and(|ch| !is_letter(ch))
        .then_some((letter, rest))
}

fn parse_mark(s: &str) -> Option<(Mark, &str)> {
    let end = s.find(['。', '、']).unwrap_or(s.len());
    let (phrase, rest) = s.split_at(end);
    let combining = if let Some(symbol) = phrase
        .strip_prefix("は「")
        .and_then(|tail| tail.strip_suffix("」付き"))
    {
        quoted_mark(symbol)?
    } else if let Some(symbol) = phrase
        .strip_prefix("の上に「")
        .and_then(|tail| tail.strip_suffix("」がつく"))
    {
        quoted_mark(symbol)?
    } else if phrase == "はファイナルシグマ" {
        return Some((Mark::FinalSigma, rest));
    } else if phrase == "はマクロン付き（-）E小文字" {
        MACRON
    } else if let Some(name) = phrase
        .strip_prefix('は')
        .and_then(|tail| tail.strip_suffix("付き"))
    {
        named_mark(name.trim_start_matches(' '))?
    } else {
        let name = phrase.strip_prefix('に')?;
        named_mark(name.strip_suffix("付き").unwrap_or(name))?
    };
    Some((Mark::Combining(combining), rest))
}

/// The text between a leading `「` and the next `」`, and what follows it.
fn quoted(s: &str) -> Option<(&str, &str)> {
    let (inner, rest) = s.strip_prefix('「')?.split_once('」')?;
    (!inner.is_empty() && !inner.contains('「')).then_some((inner, rest))
}

fn leading_number(s: &str) -> Option<(usize, &str)> {
    let mut value: usize = 0;
    let mut end = 0;
    for (index, ch) in s.char_indices() {
        let Some(digit) = ch.to_digit(10).or_else(|| {
            ('０'..='９')
                .contains(&ch)
                .then(|| u32::from(ch) - u32::from('０'))
        }) else {
            break;
        };
        value = value.checked_mul(10)?.checked_add(digit as usize)?;
        end = index + ch.len_utf8();
    }
    (end != 0).then(|| (value, &s[end..]))
}

/// A Latin or Greek letter, including precomposed forms.
fn is_letter(ch: char) -> bool {
    ch.is_ascii_alphabetic()
        || matches!(ch, '\u{00C0}'..='\u{024F}' | '\u{1E00}'..='\u{1EFF}')
        || matches!(ch, '\u{0386}'..='\u{03FF}' | '\u{1F00}'..='\u{1FFF}') && ch.is_alphabetic()
}

struct Resolved {
    /// Where the addressed text starts: the earliest scope that a clause used.
    start: usize,
    /// Each selected letter by byte offset, in text order.
    letters: Vec<(usize, char, Mark)>,
}

fn resolve(text: &str, clauses: &[Clause<'_>]) -> Option<Resolved> {
    let run_start = text
        .char_indices()
        .rev()
        .take_while(|(_, ch)| is_letter(*ch) || *ch == '-')
        .last()
        .map(|(index, _)| index);
    let whole = (0, text.len());
    let mut start = text.len();
    let mut letters = Vec::new();
    let mut inherited: Vec<(usize, usize)> = Vec::new();
    for clause in clauses {
        for target in &clause.targets {
            let scopes = match &target.scope {
                Scope::Words(words) => {
                    let spans = words
                        .iter()
                        .map(|word| {
                            let stem = word.strip_suffix('-').unwrap_or(word);
                            text.rfind(stem).map(|index| (index, index + stem.len()))
                        })
                        .collect::<Option<Vec<_>>>()?;
                    inherited.clone_from(&spans);
                    spans
                }
                Scope::Inherited if !inherited.is_empty() => inherited.clone(),
                Scope::Inherited => {
                    // Without a run before the note, only selections that
                    // stay fixed when text before the scope is dropped apply.
                    let scope = run_start.map_or(whole, |run| (run, text.len()));
                    if run_start.is_none()
                        && !matches!(&target.select, Select::Unique | Select::Last)
                        && target.select != Select::Nth(vec![1])
                    {
                        return None;
                    }
                    vec![scope]
                }
                Scope::Standalone => {
                    let mut found = text.char_indices().filter(|&(index, ch)| {
                        ch == target.letter
                            && !text[..index].chars().next_back().is_some_and(is_letter)
                            && !text[index + ch.len_utf8()..]
                                .chars()
                                .next()
                                .is_some_and(is_letter)
                    });
                    let (index, ch) = found.next()?;
                    if found.next().is_some() {
                        return None;
                    }
                    vec![(index, index + ch.len_utf8())]
                }
            };
            let unbounded = matches!(target.scope, Scope::Inherited) && inherited.is_empty();
            for (scope_start, scope_end) in scopes {
                let selected = select(text, scope_start, scope_end, target, clause.mark)?;
                let first = if unbounded && run_start.is_none() {
                    selected.iter().copied().min().unwrap_or(scope_start)
                } else {
                    scope_start
                };
                start = start.min(first);
                letters.extend(
                    selected
                        .into_iter()
                        .map(|index| (index, target.letter, clause.mark)),
                );
            }
        }
    }
    letters.sort_by_key(|(index, _, _)| *index);
    if letters.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return None;
    }
    (!letters.is_empty()).then_some(Resolved { start, letters })
}

/// Byte offsets of the letters `target` selects within `text[start..end]`.
fn select(
    text: &str,
    start: usize,
    end: usize,
    target: &Target<'_>,
    mark: Mark,
) -> Option<Vec<usize>> {
    let scope = &text[start..end];
    let matches = scope
        .char_indices()
        .filter(|(_, ch)| *ch == target.letter)
        .map(|(index, _)| start + index)
        .collect::<Vec<_>>();
    let selected = match &target.select {
        Select::Unique => {
            let candidates = if mark == Mark::FinalSigma {
                matches
                    .iter()
                    .copied()
                    .filter(|&index| {
                        !text[index + target.letter.len_utf8()..end]
                            .chars()
                            .next()
                            .is_some_and(is_letter)
                    })
                    .collect()
            } else {
                matches
            };
            match candidates.as_slice() {
                [only] => vec![*only],
                _ => return None,
            }
        }
        Select::Nth(ordinals) => ordinals
            .iter()
            .map(|ordinal| matches.get(ordinal.checked_sub(1)?).copied())
            .collect::<Option<Vec<_>>>()?,
        Select::Last => vec![*matches.last()?],
        Select::Every(count) => {
            if matches.len() != *count {
                return None;
            }
            matches
        }
        Select::Position(position) => {
            let (index, ch) = scope.char_indices().nth(position.checked_sub(1)?)?;
            if ch != target.letter {
                return None;
            }
            vec![start + index]
        }
        Select::Final => {
            let (index, ch) = scope.char_indices().next_back()?;
            if ch != target.letter {
                return None;
            }
            vec![start + index]
        }
    };
    Some(selected)
}

#[cfg(test)]
mod tests {
    use super::{compose, supplied_mark_target_start};

    fn composed(prefix: &str, body: &str) -> Option<(String, String)> {
        let start = supplied_mark_target_start(prefix, body)?;
        Some((prefix[start..].to_owned(), compose(&prefix[start..], body)?))
    }

    #[test]
    fn latin_ordinals_select_within_the_preceding_run() {
        for (prefix, body, target, expected) in [
            (
                "分かっています。Boheme",
                "一つ目の「e」は「｀」付き",
                "Boheme",
                "Bohème",
            ),
            (
                "二分の一のprobabilite",
                " 最後の「e」は「´」付き",
                "probabilite",
                "probabilité",
            ),
            (
                "態度のrepresentative",
                " 二つ目の「e」は「´」付き",
                "representative",
                "représentative",
            ),
            (
                "或るelegiaque",
                "一つ目と二つ目の「e」は「´」付き",
                "elegiaque",
                "élégiaque",
            ),
            (
                "優柔なeffemine",
                "二つ目と三つ目の「e」は「´」付き",
                "effemine",
                "efféminé",
            ),
            (
                "脚本のtheatral",
                "一つ目の「e」は「´」付き。一つ目の「a」は「^」付き",
                "theatral",
                "théâtral",
            ),
            ("Bresil", "「e」は「´」付き", "Bresil", "Brésil"),
            (
                "Morgenrote",
                "８文字目の「o」はウムラウト（¨）付き",
                "Morgenrote",
                "Morgenröte",
            ),
            ("(Dyradomr", "oにアクサン(´)付き", "Dyradomr", "Dyradómr"),
            ("ツーン(Tun", "「u」の上に「^」がつく", "Tun", "Tûn"),
            ("　A", "「A」は accent grave（｀）付き", "A", "À"),
            (
                "（ethos",
                "「e」はマクロン付き（-）E小文字",
                "ethos",
                "ēthos",
            ),
        ] {
            assert_eq!(
                composed(prefix, body),
                Some((target.to_owned(), expected.to_owned())),
                "{body}"
            );
        }
    }

    #[test]
    fn quoted_words_select_their_nearest_occurrence() {
        for (prefix, body, target, expected) in [
            (
                "Code Napoleon",
                "「Napoleon」の「e」は「´」付き",
                "Napoleon",
                "Napoléon",
            ),
            (
                "トノン氏(L'abbe Thenon)",
                "「L'abbe」の「e」はアクサン（´）付き",
                "L'abbe Thenon)",
                "L'abbé Thenon)",
            ),
            (
                "(Revue archeologique)",
                "「arche-」の「e」はアクサン（´）付き",
                "archeologique)",
                "archéologique)",
            ),
            (
                "(Bulletin de correspondance hellenique)",
                "「helle-」の２番目の「e」はアクサン（´）付き",
                "hellenique)",
                "hellénique)",
            ),
            (
                "Scheil, Delegation en Perse. 1902.",
                "「Delegation」の２つの「e」はアクサン（｀）付き",
                "Delegation en Perse. 1902.",
                "Dèlègation en Perse. 1902.",
            ),
            (
                "esprit non preocupe",
                "「preocupe」の二つの「e」は「´」付き",
                "preocupe",
                "préocupé",
            ),
            (
                "(Chassanee",
                "「ee」のうち、始めのeはアクサン(´)付き",
                "ee",
                "ée",
            ),
            (
                "Kleiden,Kuchen",
                "Kuchenのuにウムラウト（¨）",
                "Kuchen",
                "Küchen",
            ),
            (
                "keine Brucke, die fuhrt",
                "「Brucke」と「fuhrt」の「u」はウムラウト（¨）付き",
                "Brucke, die fuhrt",
                "Brücke, die führt",
            ),
            (
                "L'Illustration Theatrale",
                "「Theatrale」の一つ目の「e」は「´」付き、一つ目の「a」は「^」付き",
                "Theatrale",
                "Théâtrale",
            ),
            ("Escada de mao", "「mao」の「a」に長音記号", "mao", "māo"),
        ] {
            assert_eq!(
                composed(prefix, body),
                Some((target.to_owned(), expected.to_owned())),
                "{body}"
            );
        }
    }

    #[test]
    fn greek_marks_compose_polytonic_letters() {
        for (prefix, body, target, expected) in [
            ("強調はημων", "ηに帯気、ωに曲アクセント", "ημων", "ἡμῶν"),
            (
                "ギリシャ語のπολισ",
                "οに鋭アクセント。σはファイナルシグマ",
                "πολισ",
                "πόλις",
            ),
            (
                "ただしαναμνησισ",
                "最初のαに平息、３文字目のαに鋭アクセント、σはファイナルシグマ",
                "αναμνησισ",
                "ἀνάμνησις",
            ),
            (
                "隠遁主義 λαθε βιωσασ",
                "「λαθε」の「α」と「βιωσασ」の「ω」は鋭アクセント（´）付き。「βιωσασ」の語尾の「σ」はファイナルシグマ",
                "λαθε βιωσασ",
                "λάθε βιώσας",
            ),
            (
                "βαθυκο",
                "「ο」はアキュートアクセント付き",
                "βαθυκο",
                "βαθυκό",
            ),
            (
                "ギリシア語の υποκειμενον",
                "υは帯気の気息記号（‘の上下が逆さまになったような記号）付き、ιはアキュートアクセント（´）付き",
                "υποκειμενον",
                "ὑποκείμενον",
            ),
            (
                "　｛１２｝α　ποτ’　ειδεν　ημων　η　ψυχη(Platon, Phaidros 249c).",
                "最初のαに帯気＋鋭アクセント。ειδενのιに平息＋曲アクセント。ημωνのηに帯気、ωに曲アクセント。単体のηに帯気。ψυχηのηに鋭アクセント",
                "α　ποτ’　ειδεν　ημων　η　ψυχη(Platon, Phaidros 249c).",
                "ἅ　ποτ’　εἶδεν　ἡμῶν　ἡ　ψυχή(Platon, Phaidros 249c).",
            ),
        ] {
            assert_eq!(
                composed(prefix, body),
                Some((target.to_owned(), expected.to_owned())),
                "{body}"
            );
        }
    }

    #[test]
    fn a_stray_or_enclosing_quote_does_not_change_the_reading() {
        assert_eq!(
            composed("、Antigeishaisme", "一つ目の「e」は「´」付き」").map(|(_, text)| text),
            Some("Antigéishaisme".to_owned())
        );
        assert_eq!(
            composed("とMoliere", "「一つ目の「e」は「｀」付き」").map(|(_, text)| text),
            Some("Molière".to_owned())
        );
    }

    #[test]
    fn unresolvable_selections_decline() {
        // Two e in the stem and no ordinal.
        assert_eq!(
            supplied_mark_target_start("Delegation", "「Dele-」の「e」はアクサン（´）付き"),
            None
        );
        // The first character is not the letter.
        assert_eq!(
            supplied_mark_target_start("Μεροπο", "１文字目のοは鋭アクセント付き"),
            None
        );
        // The quoted word lies before notation on the line.
        assert_eq!(
            supplied_mark_target_start(
                "Gotter《ゴッテル》 Griechenlands",
                "「Gotter」の「o」はウムラウト（¨）付き"
            ),
            None
        );
        // The mark wording is absent from the grammar.
        assert_eq!(
            supplied_mark_target_start("Boheme", "一つ目の「e」は「~」付き"),
            None
        );
        assert_eq!(
            supplied_mark_target_start("Boheme", "「e」は小さい「ヱ」"),
            None
        );
    }
}
