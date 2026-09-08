//! Independent process that applies a verified update after Teshi exits.

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
struct Options {
    /// Journal directory inside `.teshi-update`.
    #[arg(long)]
    transaction: PathBuf,
    /// SHA256 of the accepted journal.
    #[arg(long)]
    plan_sha256: String,
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--update-identity") {
        match serde_json::to_string(&teshi_core::version::build_identity()) {
            Ok(identity) => println!("{identity}"),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    let options = Options::parse();
    if let Err(error) =
        teshi_update::transaction::run_helper(&options.transaction, &options.plan_sha256)
    {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
