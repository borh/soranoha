//! Build script that bakes the source revision into --version.

use std::env;

fn main() {
    // Bakes the source revision into --version so gate evidence and the
    // registry row identify the measured code. Gate builds must set
    // AB_AOZORA_GIT_REV (Global Constraints); when it is absent the revision
    // is "unknown" and the build succeeds (dev builds). rerun-if-env-changed makes cargo rebuild
    // when the rev changes despite an otherwise-clean cache.
    println!("cargo:rerun-if-env-changed=AB_AOZORA_GIT_REV");
    let rev = env::var("AB_AOZORA_GIT_REV").unwrap_or_else(|_| "unknown".into());
    println!("cargo:rustc-env=AB_AOZORA_GIT_REV={rev}");
}
