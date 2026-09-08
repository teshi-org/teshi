//! Rebuild when CI injects nightly build identity.

fn main() {
    println!("cargo:rerun-if-env-changed=TESHI_BUILD_CHANNEL");
    println!("cargo:rerun-if-env-changed=TESHI_BUILD_DATE");
    println!("cargo:rerun-if-env-changed=TESHI_GIT_SHA");
    println!("cargo:rerun-if-env-changed=TESHI_BUILD_TIMESTAMP");
    println!("cargo:rerun-if-env-changed=TESHI_BUILD_SEQUENCE");
    if let Ok(target) = std::env::var("TARGET") {
        println!("cargo:rustc-env=TESHI_TARGET={target}");
    }
}
