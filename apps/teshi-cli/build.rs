//! Link settings for the `teshi` binary.

/// Main-thread stack reserved on Windows, matching the Linux default.
const MAIN_STACK_BYTES: u32 = 8 * 1024 * 1024;

fn main() {
    // Unoptimized builds of the clap command tree need close to Windows' default
    // 1 MiB main-thread stack, so one more subcommand overflows it at startup.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    match std::env::var("CARGO_CFG_TARGET_ENV").as_deref() {
        Ok("msvc") => println!("cargo:rustc-link-arg-bins=/STACK:{MAIN_STACK_BYTES}"),
        Ok("gnu") => println!("cargo:rustc-link-arg-bins=-Wl,--stack,{MAIN_STACK_BYTES}"),
        _ => {}
    }
}
