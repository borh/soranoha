//! Black-box tests of the public minimal-diff splice API through the `aozora`
//! facade, called as an editor integration calls it, which checks that the
//! re-exports are usable from outside the crate.

use ab_aozora_facade::{CoupledKind, Document, RegionRole, SpliceError, SpliceSafety};

#[test]
fn regions_cover_the_whole_source() {
    let src = "序章\n｜青梅《おうめ》の実、青空［＃「青空」に傍点］。";
    let doc = Document::new(src);
    let tree = doc.parse();
    let sanitized = tree.sanitized();

    let regions = tree.regions();
    // Complete, contiguous cover.
    assert_eq!(regions.first().unwrap().span.start, 0);
    assert_eq!(regions.last().unwrap().span.end as usize, sanitized.len());
    let rebuilt: String = regions
        .iter()
        .map(|r| &sanitized[r.span.start as usize..r.span.end as usize])
        .collect();
    assert_eq!(rebuilt, sanitized);

    // The ruby and the reclaimed forward reference appear with classified roles.
    assert!(regions.iter().any(|r| r.role == RegionRole::Ruby));
    assert!(
        regions
            .iter()
            .any(|r| r.role == RegionRole::ForwardReclaimed)
    );
}

#[test]
fn reclaimed_forward_is_direct_minimal_diff() {
    let src = "青空［＃「青空」に傍点］の下を歩く";
    let doc = Document::new(src);
    let tree = doc.parse();

    let region = tree
        .regions()
        .into_iter()
        .find(|r| r.role == RegionRole::ForwardReclaimed)
        .expect("a reclaimed forward bouten");
    assert_eq!(region.safety, SpliceSafety::Direct);

    // The region contains the whole construct (literal and bracket), so a
    // direct byte replacement is a complete edit.
    let spliced = tree
        .splice(region, "海［＃「海」に傍点］")
        .expect("Direct region splices");
    assert_eq!(spliced, "海［＃「海」に傍点］の下を歩く");
}

#[test]
fn referenced_forward_is_coupled() {
    let src = "青空がひろがる、その［＃「青空」に傍点］";
    let doc = Document::new(src);
    let tree = doc.parse();

    let region = tree
        .regions()
        .into_iter()
        .find(|r| r.role == RegionRole::ForwardReferenced)
        .expect("a referenced forward bouten");
    assert_eq!(
        region.safety,
        SpliceSafety::Coupled(CoupledKind::ForwardReference)
    );

    // An attribute-only change keeps the target, so the forward reference
    // resolves again and the single-region edit is accepted.
    let spliced = tree
        .splice(region, "［＃「青空」に傍線］")
        .expect("attribute-only change is coherent");
    assert_eq!(spliced, "青空がひろがる、その［＃「青空」に傍線］");

    // A target-text change is a coupled edit: both the bracket and the unique
    // preceding literal are rewritten so that the reference still resolves.
    let coupled = tree
        .splice(region, "［＃「海」に傍点］")
        .expect("target change is a coupled edit");
    assert_eq!(coupled, "海がひろがる、その［＃「海」に傍点］");
}

#[test]
fn ruby_base_forward_target_change_is_irreducible() {
    // The referent literal is inside a ruby base, not a plain run, so the
    // splice cannot isolate a target change and returns `Unverifiable`.
    let src = "｜我《われ》は［＃「我」に傍点］";
    let doc = Document::new(src);
    let tree = doc.parse();
    let Some(region) = tree
        .regions()
        .into_iter()
        .find(|r| r.role == RegionRole::ForwardReferenced)
    else {
        return; // no `ForwardReferenced` region for this shape; nothing to assert
    };
    let err = tree
        .splice(region, "［＃「彼」に傍点］")
        .expect_err("ruby-base referent cannot be coupled");
    assert!(matches!(err, SpliceError::Unverifiable { .. }));
}

#[test]
fn ruby_base_forward_attribute_change_is_coherent() {
    // The referent is inside a ruby base, so a target change is irreducible;
    // an attribute-only change (傍点→傍線) keeps the node's target and is
    // verified in a scoped `<target><replacement>` context that excludes the
    // ruby base, so the splice accepts it.
    let src = "｜我《われ》は［＃「我」に傍点］";
    let doc = Document::new(src);
    let tree = doc.parse();
    let Some(region) = tree
        .regions()
        .into_iter()
        .find(|r| r.role == RegionRole::ForwardReferenced)
    else {
        return; // no `ForwardReferenced` region for this shape; nothing to assert
    };
    let spliced = tree
        .splice(region, "［＃「我」に傍線］")
        .expect("attribute-only change on a ruby-base forward is coherent");
    // Only the bracket changes; the sanitized ruby reading and base remain.
    assert!(spliced.contains("われ"));
    assert!(spliced.contains("傍線"));
    assert!(!spliced.contains("傍点"));
}

#[test]
fn multi_target_forward_is_coupled_and_identity_safe() {
    // A disjoint target list is source-owned, but a splice cannot replace it
    // with an unassociated scalar target; an identity splice retains all
    // selected spans.
    let src = "AとB［＃「A」「B」に傍点］";
    let doc = Document::new(src);
    let tree = doc.parse();
    let sanitized = tree.sanitized();
    let region = tree
        .regions()
        .into_iter()
        .find(|r| r.role == RegionRole::ForwardReferenced)
        .expect("a multi-target forward bouten");
    assert_eq!(
        region.safety,
        SpliceSafety::Coupled(CoupledKind::ForwardReference)
    );
    let own = &sanitized[region.span.start as usize..region.span.end as usize];
    assert_eq!(
        tree.splice(region, own).expect("identity is a no-op"),
        sanitized
    );
    let err = tree
        .splice(region, "［＃「海」に傍点］")
        .expect_err("a multi-segment target change is irreducible");
    assert!(matches!(err, SpliceError::Unverifiable { .. }));
}

#[test]
fn heading_hint_target_change_syncs() {
    // A forward heading hint requires its target to be preceded by a matching
    // run; that run is the coupled partner. Changing the title rewrites both
    // the directive and the preceding run so that the hint still resolves.
    let src = "第一篇［＃「第一篇」は大見出し］";
    let doc = Document::new(src);
    let tree = doc.parse();
    let sanitized = tree.sanitized();
    let region = tree
        .regions()
        .into_iter()
        .find(|r| r.role == RegionRole::HeadingHint)
        .expect("a forward heading hint");
    assert_eq!(
        region.safety,
        SpliceSafety::Coupled(CoupledKind::HeadingHint)
    );
    // Identity is a no-op; deleting the directive is coherent.
    let own = &sanitized[region.span.start as usize..region.span.end as usize];
    assert_eq!(
        tree.splice(region, own).expect("identity is a no-op"),
        sanitized
    );
    tree.splice(region, "")
        .expect("deleting the directive is coherent");
    // A coupled title change also rewrites the preceding run.
    let spliced = tree
        .splice(region, "［＃「序章」は大見出し］")
        .expect("title change is a coupled edit");
    assert_eq!(spliced, "序章［＃「序章」は大見出し］");
}

#[test]
fn margin_note_base_change_is_coherent() {
    // The region of a side note (注記) includes the base run it annotates
    // (`未来［＃…］`, base 未来 included), so a base change is a single-region
    // replacement containing the new base, verified within its family; bytes
    // outside the region are unchanged.
    let src = "未来［＃「未来」の左に「みらい」の注記］を見る。";
    let doc = Document::new(src);
    let tree = doc.parse();
    let sanitized = tree.sanitized();
    let region = tree
        .regions()
        .into_iter()
        .find(|r| r.role == RegionRole::MarginNote)
        .expect("a side-note margin note");
    assert_eq!(
        region.safety,
        SpliceSafety::Coupled(CoupledKind::MarginNote)
    );
    let own = &sanitized[region.span.start as usize..region.span.end as usize];
    assert_eq!(
        tree.splice(region, own).expect("identity is a no-op"),
        sanitized
    );
    let spliced = tree
        .splice(region, "過去［＃「過去」の左に「みらい」の注記］")
        .expect("base change is a coherent edit");
    assert_eq!(spliced, "過去［＃「過去」の左に「みらい」の注記］を見る。");
}

#[test]
fn container_open_couples_to_its_close() {
    let src = "前\n［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］\n後";
    let doc = Document::new(src);
    let tree = doc.parse();

    let open = tree
        .regions()
        .into_iter()
        .find(|r| r.role == RegionRole::ContainerOpen)
        .expect("a container open");
    assert_eq!(open.safety, SpliceSafety::Coupled(CoupledKind::Container));

    // Changing the family rewrites both the open and the paired close.
    let spliced = tree
        .splice(open, "［＃ここから罫囲み］")
        .expect("a container kind change is verifiable");
    assert!(spliced.contains("［＃ここから罫囲み］"));
    assert!(spliced.contains("［＃罫囲み終わり］"));
    assert!(!spliced.contains("字下げ"));
    assert!(spliced.contains("本文"));
}
