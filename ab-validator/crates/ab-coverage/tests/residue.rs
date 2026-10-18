use std::collections::BTreeSet;

use ab_coverage::{
    occurrence_alignment::AlignmentStatus,
    residue::{
        Coverage, Decision, InventoryMismatch, LedgerOccurrence, Locator, OccurrenceClass,
        RuledOccurrence, UndecidableReason, inventory_mismatches, load_decisions, occurrence_class,
        ratchet, row_ledger, row_qualification, unmatched_rulings,
    },
};

const DECISIONS: &str = include_str!("../../../data/residue-decisions.toml");

fn occurrence(form: &str, partitions: &[&str], status: AlignmentStatus) -> LedgerOccurrence {
    LedgerOccurrence {
        form: form.to_owned(),
        partitions: partitions
            .iter()
            .map(|&partition| partition.to_owned())
            .collect(),
        status,
        uncertain_source_retained: false,
        checks_satisfied: false,
        checks_failed: false,
        text_retained: false,
        locator: Locator {
            work_id: "w".to_owned(),
            indexed_path: "w.txt".to_owned(),
            line: 1,
            byte_start: 0,
            raw: form.to_owned(),
        },
    }
}

/// A ruling for `form` in row "row", scoped to the `(byte_start, raw)`
/// occurrences of work "w" when any are given.
fn ruling(form: &str, locators: &[(usize, &str)]) -> Decision {
    Decision {
        row: "row".to_owned(),
        form: form.to_owned(),
        locators: locators
            .iter()
            .map(|&(byte_start, raw)| RuledOccurrence {
                work_id: "w".to_owned(),
                indexed_path: "w.txt".to_owned(),
                byte_start,
                raw: raw.to_owned(),
            })
            .collect(),
        reason: UndecidableReason::AmbiguousTarget,
        evidence: vec!["e".to_owned()],
        reviewer: "x".to_owned(),
        reviewed_at: "2026-09-24".to_owned(),
        notes: "n".to_owned(),
    }
}

/// A retained raw occurrence of `form` at `byte_start`.
fn raw_at(form: &str, byte_start: usize) -> LedgerOccurrence {
    let base = occurrence(form, &["p:x"], AlignmentStatus::RawPreserved);
    LedgerOccurrence {
        uncertain_source_retained: true,
        locator: Locator {
            byte_start,
            ..base.locator.clone()
        },
        ..base
    }
}

fn coverage() -> Coverage {
    Coverage {
        fixture_forms: BTreeSet::from(["fixture".to_owned()]),
        covered_partitions: BTreeSet::from(["p:covered".to_owned()]),
        rulings: vec![ruling("ruled", &[])],
    }
}

#[test]
fn each_occurrence_takes_the_first_class_its_evidence_supports() {
    use AlignmentStatus::{ParseFailed, RawPreserved, Typed, Unaligned};
    use OccurrenceClass as C;
    let checked = |satisfied, failed, form: &str| LedgerOccurrence {
        checks_satisfied: satisfied,
        checks_failed: failed,
        ..occurrence(form, &["p:covered"], Typed)
    };
    let retained = |form: &str| LedgerOccurrence {
        uncertain_source_retained: true,
        ..occurrence(form, &[], RawPreserved)
    };
    let text = |failed| LedgerOccurrence {
        checks_satisfied: true,
        checks_failed: failed,
        text_retained: true,
        ..occurrence("other", &[], Unaligned)
    };
    let cases = [
        (
            "ruled form retained with its uncertainty",
            retained("ruled"),
            C::Undecidable,
        ),
        (
            "ruled form raw without a retained uncertain source",
            occurrence("ruled", &[], RawPreserved),
            C::UndecidableConflict,
        ),
        (
            "ruled form interpreted",
            occurrence("ruled", &[], Typed),
            C::UndecidableConflict,
        ),
        (
            "raw",
            occurrence("fixture", &["p:covered"], RawPreserved),
            C::SemanticGap,
        ),
        (
            "unaligned",
            occurrence("fixture", &[], Unaligned),
            C::AlignmentReview,
        ),
        (
            "unaligned with a satisfied check that satisfies no marker_text check",
            LedgerOccurrence {
                checks_satisfied: true,
                ..occurrence("other", &[], Unaligned)
            },
            C::AlignmentReview,
        ),
        (
            "unaligned marker found in a text node",
            text(false),
            C::PredicateCovered,
        ),
        (
            "unaligned marker found in a text node with a failed check",
            text(true),
            C::AlignmentReview,
        ),
        (
            "parse failure",
            occurrence("fixture", &[], ParseFailed),
            C::AlignmentReview,
        ),
        (
            "failed check outranks a fixture",
            checked(true, true, "fixture"),
            C::CheckFailed,
        ),
        (
            "satisfied check",
            checked(true, false, "other"),
            C::PredicateCovered,
        ),
        (
            "fixture form",
            occurrence("fixture", &[], Typed),
            C::FixtureCovered,
        ),
        (
            "covered partitions",
            occurrence("other", &["p:covered"], Typed),
            C::AliasOfCovered,
        ),
        (
            "one uncovered partition",
            occurrence("other", &["p:covered", "p:open"], Typed),
            C::Unqualified,
        ),
        (
            "no partitions",
            occurrence("other", &[], Typed),
            C::Unqualified,
        ),
    ];
    for (name, occurrence, expected) in cases {
        assert_eq!(
            occurrence_class(&occurrence, &coverage()),
            expected,
            "{name}"
        );
    }
}

#[test]
fn the_ledger_accounts_for_every_occurrence_and_lists_only_residue() {
    use AlignmentStatus::{RawPreserved, Typed};
    let mut occurrences = vec![
        occurrence("fixture", &[], Typed),
        occurrence("gap-a", &["p:x"], RawPreserved),
        occurrence("gap-b", &["p:x"], RawPreserved),
        occurrence("gap-c", &["p:y"], RawPreserved),
        occurrence("other", &["p:open"], Typed),
    ];
    for byte_start in 0..5 {
        occurrences.push(LedgerOccurrence {
            locator: Locator {
                byte_start,
                ..occurrence("ruled", &[], RawPreserved).locator
            },
            uncertain_source_retained: true,
            ..occurrence("ruled", &[], RawPreserved)
        });
        occurrences.push(LedgerOccurrence {
            locator: Locator {
                byte_start,
                ..occurrence("gap-a", &[], RawPreserved).locator
            },
            ..occurrence("gap-a", &["p:x"], RawPreserved)
        });
    }
    let ledger = row_ledger(&occurrences, &coverage(), 2);

    let classified = ledger
        .classes
        .values()
        .map(|total| total.occurrences)
        .sum::<u64>();
    assert_eq!(classified, occurrences.len() as u64);
    assert_eq!(ledger.occurrences, 15);
    assert_eq!(ledger.residue_occurrences, 14);
    assert!(
        !ledger
            .residue
            .contains_key(&OccurrenceClass::FixtureCovered)
    );

    let undecidable = &ledger.residue[&OccurrenceClass::Undecidable][0];
    assert_eq!(undecidable.reason, Some(UndecidableReason::AmbiguousTarget));
    assert_eq!(undecidable.locators.len(), 5, "rulings list every locator");
    let gaps = &ledger.residue[&OccurrenceClass::SemanticGap];
    assert_eq!(gaps[0].form, "gap-a");
    assert_eq!(gaps[0].occurrences, 6);
    assert_eq!(gaps[0].locators.len(), 2, "other residue is sampled");

    let batches = ledger
        .gap_batches
        .iter()
        .map(|batch| (batch.partitions.join(","), batch.forms, batch.occurrences))
        .collect::<Vec<_>>();
    assert_eq!(
        batches,
        [("p:x".to_owned(), 2, 7), ("p:y".to_owned(), 1, 1)],
        "gaps batch by partition set across spellings"
    );
}

#[test]
fn ledger_totals_reconcile_with_every_inventory_form_and_partition() {
    use AlignmentStatus::{RawPreserved, Typed};
    let occurrences = [
        occurrence("a", &["p:x"], Typed),
        occurrence("a", &["p:x", "p:y"], RawPreserved),
        occurrence("", &["p:y"], Typed),
    ];
    let inventory = |a: u64, x: u64| {
        serde_json::json!({"rows": {"row": {
            "variants": {"a": {"occurrences": a}},
            "semantic_partitions": {"p:x": {"occurrences": x}, "p:y": {"occurrences": 2}},
        }}})
    };
    assert!(
        inventory_mismatches(&inventory(2, 2), "row", &occurrences).is_empty(),
        "an occurrence without a form counts only toward its partitions"
    );
    let mismatch = |section, key: &str, inventory, ledger| InventoryMismatch {
        row: "row".to_owned(),
        section,
        key: key.to_owned(),
        inventory,
        ledger,
    };
    assert_eq!(
        inventory_mismatches(&inventory(3, 1), "row", &occurrences),
        [
            mismatch("semantic_partitions", "p:x", 1, 2),
            mismatch("variants", "a", 3, 2),
        ]
    );
    assert_eq!(
        inventory_mismatches(&serde_json::json!({"rows": {}}), "row", &occurrences[..1]),
        [
            mismatch("semantic_partitions", "p:x", 0, 1),
            mismatch("variants", "a", 0, 1),
        ],
        "a row the inventory lacks reconciles against zero"
    );
}

#[test]
fn a_located_ruling_classes_only_the_occurrences_it_identifies() {
    let coverage = Coverage {
        rulings: vec![ruling("shared", &[(7, "shared")])],
        ..Coverage::default()
    };
    assert_eq!(
        occurrence_class(&raw_at("shared", 7), &coverage),
        OccurrenceClass::Undecidable
    );
    assert_eq!(
        occurrence_class(&raw_at("shared", 8), &coverage),
        OccurrenceClass::SemanticGap,
        "another occurrence of the same normalized form"
    );
}

#[test]
fn rulings_that_identify_no_occurrence_of_their_row_are_reported() {
    let occurrences = [raw_at("shared", 7)];
    let other_row = Decision {
        row: "other".to_owned(),
        ..ruling("gone", &[])
    };
    let decisions = [
        ruling("shared", &[(7, "shared")]),
        ruling("shared", &[(9, "shared")]),
        ruling("shared", &[(7, "changed")]),
        ruling("other-form", &[(7, "shared")]),
        ruling("gone", &[]),
        other_row,
    ];
    assert_eq!(
        unmatched_rulings(&decisions, "row", &occurrences),
        [
            "row shared at w.txt byte 9",
            "row shared at w.txt byte 7",
            "row other-form at w.txt byte 7",
            "row gone",
        ]
    );
}

#[test]
fn decisions_load_only_at_the_supported_version_with_a_known_reason() {
    assert!(load_decisions(DECISIONS).is_ok());
    let decision = |reason: &str| {
        format!(
            "decisions_version = 1\n[[decision]]\nrow = \"r\"\nform = \"f\"\nreason = \"{reason}\"\nevidence = [\"e\"]\nreviewer = \"x\"\nreviewed_at = \"2026-09-23\"\nnotes = \"n\"\n"
        )
    };
    assert_eq!(
        load_decisions(&decision("malformed-boundary")).unwrap()[0].reason,
        UndecidableReason::MalformedBoundary
    );
    assert!(load_decisions(&decision("looks-hard")).is_err());
    let located = |key: &str| {
        format!(
            "{}locators = [{{ work_id = \"w\", indexed_path = \"w.txt\", byte_start = 7, {key} = \"f\" }}]\n",
            decision("malformed-boundary")
        )
    };
    assert_eq!(
        load_decisions(&located("raw")).unwrap()[0].locators[0].byte_start,
        7
    );
    assert!(load_decisions(&located("text")).is_err());
    assert!(load_decisions("decisions_version = 2\n").is_err());
}

#[test]
fn the_ratchet_fails_a_row_only_when_its_residue_grows() {
    use AlignmentStatus::{RawPreserved, Typed};
    let ledger = |occurrences: &[LedgerOccurrence]| {
        let ledger = row_ledger(occurrences, &coverage(), 1);
        serde_json::json!({"residue_ledger": {"row": ledger}})
    };
    let gap = occurrence("gap", &["p:x"], RawPreserved);
    let before = ledger(&[gap.clone(), occurrence("fixture", &[], Typed)]);
    let current = |occurrences: &[LedgerOccurrence]| {
        [("row".to_owned(), row_ledger(occurrences, &coverage(), 1))].into()
    };

    let fixed = ratchet(&before, &current(&[occurrence("fixture", &[], Typed)]));
    assert!(!fixed["row"].regressed);
    assert_eq!(
        fixed["row"].class_deltas,
        [("semantic-gap".to_owned(), -1)].into()
    );

    let ruled = ratchet(
        &before,
        &current(&[
            occurrence("ruled", &[], RawPreserved),
            occurrence("fixture", &[], Typed),
        ]),
    );
    assert!(
        !ruled["row"].regressed,
        "reclassifying residue is not growth"
    );

    let grown = ratchet(
        &before,
        &current(&[gap.clone(), gap, occurrence("fixture", &[], Typed)]),
    );
    assert!(grown["row"].regressed);
    assert!(ratchet(&serde_json::json!({}), &current(&[])).is_empty());
}

#[test]
fn a_row_is_qualified_only_without_report_gaps_or_residue_beyond_ruled_undecidables() {
    use AlignmentStatus::{RawPreserved, Typed};
    let covered = row_ledger(&[occurrence("fixture", &[], Typed)], &coverage(), 1);
    assert!(row_qualification(&[], &covered).qualified);

    let gapped = row_qualification(&["xml-not-assessed".to_owned()], &covered);
    assert!(!gapped.qualified);
    assert_eq!(gapped.report_gaps, ["xml-not-assessed"]);
    assert!(gapped.residue.is_empty());

    let undecidable = row_ledger(
        &[raw_at("ruled", 0), occurrence("fixture", &[], Typed)],
        &coverage(),
        1,
    );
    assert_eq!(
        undecidable.classes[&OccurrenceClass::Undecidable].occurrences,
        1
    );
    assert!(
        row_qualification(&[], &undecidable).qualified,
        "a ruled undecidable is a resolved outcome"
    );

    let residue = row_qualification(
        &[],
        &row_ledger(&[occurrence("gap", &["p:x"], RawPreserved)], &coverage(), 1),
    );
    assert!(!residue.qualified);
    assert_eq!(residue.residue, [(OccurrenceClass::SemanticGap, 1)].into());
}
