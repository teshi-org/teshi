//! Rebuild when CI injects nightly build identity.

fn main() {
    println!("cargo:rerun-if-env-changed=TESHI_BUILD_CHANNEL");
    println!("cargo:rerun-if-env-changed=TESHI_BUILD_DATE");
    println!("cargo:rerun-if-env-changed=TESHI_GIT_SHA");
}
