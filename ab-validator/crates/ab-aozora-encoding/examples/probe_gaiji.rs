//! Data-driven probe for the gaiji-resolution path.
//!
//! Prints the gaiji table sizes, then the `lookup` result for a fixed
//! list of (description, mencode) pairs reported as not resolving: once
//! with both inputs, once with the mencode only, and once with the
//! description only. Output is plain text on stderr, suitable for `awk`
//! or `sort`.
//!
//! Run with:
//! ```text
//! cargo run -p aozora-encoding --example probe_gaiji
//! ```

use ab_aozora_encoding::gaiji::{Resolved, lookup, table_sizes};

fn main() {
    let (single, combo, desc) = table_sizes();
    eprintln!("=== gaiji table sizes ===");
    eprintln!("  mencode → char (single):  {single}");
    eprintln!("  mencode → str  (combo):   {combo}");
    eprintln!("  description → char:        {desc}");
    eprintln!();

    let failing = [
        // The three plane-2 entries reported as not resolving:
        ("丂", Some("第4水準2-16-1")),
        ("畺", Some("第4水準2-45-30")),
        ("龔", Some("第4水準2-77-60")),
        // Plane-1 control case (known to resolve):
        ("木＋吶のつくり", Some("第3水準1-85-54")),
        // Plane-2 first-cell baseline:
        ("人の異体", Some("第4水準2-1-1")),
        // Description fallback (no mencode):
        ("木＋吶のつくり", None),
        // Canonical chuki naming: 鄧 = 登 + おおざと radical (阝).
        // Literal "邦＋登" is not in the dictionary because the convention is
        // <left-component> + <right-radical-name>. Both forms are
        // probed so the output shows the difference.
        ("登＋おおざと", None),
        ("邦＋登", None),
    ];

    eprintln!("=== flagged-failing probe ===");
    for (desc, men) in failing {
        let r = lookup(None, men, desc);
        match r {
            Some(Resolved::Char(c)) => eprintln!(
                "  desc={desc:?} men={men:?} → CHAR '{c}' (U+{:04X})",
                c as u32
            ),
            Some(Resolved::Multi(s)) => eprintln!("  desc={desc:?} men={men:?} → MULTI {s:?}"),
            Some(Resolved::Voiced { base, mark }) => {
                eprintln!("  desc={desc:?} men={men:?} → VOICED {base}{mark}");
            }
            None => eprintln!("  desc={desc:?} men={men:?} → NONE"),
        }
    }

    // Look up each mencode without the description, to separate a
    // mencode-tier miss from a description-tier miss.
    eprintln!();
    eprintln!("=== mencode-only probe (description='dummy') ===");
    for (_, men) in failing {
        let Some(m) = men else { continue };
        let r = lookup(None, Some(m), "dummy");
        eprintln!("  men={m:?} → {r:?}");
    }

    // Look up each description without the mencode, to show whether the
    // description table covers these characters.
    eprintln!();
    eprintln!("=== description-only probe (mencode=None) ===");
    for (desc, _) in failing {
        let r = lookup(None, None, desc);
        eprintln!("  desc={desc:?} → {r:?}");
    }
}
