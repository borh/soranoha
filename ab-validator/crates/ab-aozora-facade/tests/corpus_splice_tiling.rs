//! Walks `AOZORA_CORPUS_ROOT` and checks the source-region ownership tiling and
//! the minimal-diff splice for every document:
//!
//! * [`ab_aozora_facade::Tree::regions`] is a complete, gap-free, ordered,
//!   non-overlapping cover of the sanitized source; the region byte slices
//!   concatenate to it exactly.
//! * No region is unclassified (`Opaque`), so every construct is editable.
//! * The identity splice reproduces the sanitized source. `check_tiling` checks
//!   the byte-level tiling for every region; the splice check runs
//!   `Tree::splice` (a `Direct` byte replacement, or a `Coupled` partner
//!   derivation with scoped verification) on one region per safety class per
//!   document. Each splice is `O(marker)` because it verifies in a scoped
//!   context without re-parsing the whole document; the sample bounds the number
//!   of splice calls, which keeps the full-corpus sweep under a minute instead of
//!   one call for each of the corpus's millions of coupled regions.
//!
//! `tests/splice_api.rs` checks the same tiling on hand-written inputs; this
//! test applies it to the full 青空文庫 corpus. The test is skipped when `AOZORA_CORPUS_ROOT` is
//! unset and does not fail on a missing corpus.

use ab_aozora_encoding::decode_auto;
use ab_aozora_facade::{CoupledKind, Document, Region, RegionRole, SpliceSafety};

/// Cap the collected failures so a systemic regression does not produce
/// a multi-megabyte assertion message.
const MAX_REPORTED: usize = 50;

#[test]
fn corpus_regions_tile_the_source() {
    let Some(source) = ab_aozora_corpus::from_env() else {
        eprintln!("AOZORA_CORPUS_ROOT not set; skipping splice tiling sweep");
        return;
    };

    let mut count: usize = 0;
    let mut failures: Vec<String> = Vec::new();

    for item in source.iter() {
        let item = item.expect("corpus iteration must not error");
        let Ok(utf8) = decode_auto(&item.bytes) else {
            eprintln!("skip (neither UTF-8 nor Shift_JIS): {}", item.label);
            continue;
        };

        let doc = Document::new(utf8);
        let tree = doc.parse();
        let sanitized = tree.sanitized();
        let regions = tree.regions();

        if let Err(why) = check_tiling(sanitized, &regions) {
            failures.push(format!("{}: {why}", item.label));
        } else if let Err(why) = check_splice_sampled(&tree, sanitized, &regions) {
            failures.push(format!("{}: {why}", item.label));
        }

        count += 1;
        if failures.len() >= MAX_REPORTED {
            break;
        }
    }

    eprintln!("splice tiling sweep: {count} docs walked");
    assert!(
        failures.is_empty(),
        "{} tiling/splice failure(s):\n  {}",
        failures.len(),
        failures.join("\n  "),
    );
    eprintln!("splice tiling sweep: owned-region tiling + identity splice hold");
}

/// Verify the regions form a complete, ordered, gap-free, non-overlapping
/// cover whose byte-slices concatenate back to `sanitized`.
fn check_tiling(sanitized: &str, regions: &[Region]) -> Result<(), String> {
    if sanitized.is_empty() {
        return if regions.is_empty() {
            Ok(())
        } else {
            Err("empty source yielded regions".to_owned())
        };
    }
    let Some(first) = regions.first() else {
        return Err("non-empty source yielded no regions".to_owned());
    };
    if first.span.start != 0 {
        return Err(format!("tiling starts at {} not 0", first.span.start));
    }
    let end = regions[regions.len() - 1].span.end as usize;
    if end != sanitized.len() {
        return Err(format!("tiling ends at {end} not {}", sanitized.len()));
    }
    for pair in regions.windows(2) {
        if pair[0].span.end != pair[1].span.start {
            return Err(format!(
                "gap/overlap between {}..{} and {}..{}",
                pair[0].span.start, pair[0].span.end, pair[1].span.start, pair[1].span.end,
            ));
        }
    }
    let rebuilt: String = regions
        .iter()
        .map(|r| &sanitized[r.span.start as usize..r.span.end as usize])
        .collect();
    if rebuilt != sanitized {
        return Err("region concatenation != sanitized source".to_owned());
    }
    Ok(())
}

/// Sample bucket per safety class. Container regions split into open and close
/// because the edit path handles the two marker shapes differently.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Sample {
    Direct,
    ContainerOpen,
    ContainerClose,
    Forward,
    HeadingHint,
    MarginNote,
}

/// The sample bucket for a region, or `None` for ones covered structurally
/// elsewhere (e.g. a leaf container).
fn sample_of(r: &Region) -> Option<Sample> {
    match (r.safety, r.role) {
        (SpliceSafety::Direct, _) => Some(Sample::Direct),
        (SpliceSafety::Coupled(CoupledKind::Container), RegionRole::ContainerOpen) => {
            Some(Sample::ContainerOpen)
        }
        (SpliceSafety::Coupled(CoupledKind::Container), RegionRole::ContainerClose) => {
            Some(Sample::ContainerClose)
        }
        (SpliceSafety::Coupled(CoupledKind::ForwardReference), _) => Some(Sample::Forward),
        (SpliceSafety::Coupled(CoupledKind::HeadingHint), _) => Some(Sample::HeadingHint),
        (SpliceSafety::Coupled(CoupledKind::MarginNote), _) => Some(Sample::MarginNote),
        _ => None,
    }
}

/// Checks that no region is `Opaque`, and that the identity splice of one region
/// per safety class reproduces the sanitized source through `Tree::splice`, with
/// at most six `splice` calls per document.
fn check_splice_sampled(
    tree: &ab_aozora_facade::Tree<'_>,
    sanitized: &str,
    regions: &[Region],
) -> Result<(), String> {
    let mut sampled: Vec<Sample> = Vec::with_capacity(6);
    for r in regions {
        if r.safety == SpliceSafety::Opaque {
            return Err(format!("region {:?} classified Opaque", r.role));
        }
        let Some(bucket) = sample_of(r) else { continue };
        if sampled.contains(&bucket) {
            continue;
        }
        sampled.push(bucket);

        let same = &sanitized[r.span.start as usize..r.span.end as usize];
        match tree.splice(*r, same) {
            Ok(out) if out == sanitized => {}
            Ok(_) => return Err(format!("identity splice of {:?} changed bytes", r.role)),
            Err(e) => return Err(format!("identity splice of {:?} failed: {e}", r.role)),
        }
    }
    Ok(())
}
