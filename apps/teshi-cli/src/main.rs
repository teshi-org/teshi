use anyhow::{Context, Result};
use clap::Parser;

mod internal_broker;

#[derive(Debug, Parser)]
#[command(name = "teshi", disable_help_flag = true, disable_version_flag = true)]
struct WebCommand {
    #[command(flatten)]
    options: teshi_daemon::WebOptions,
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.get(1).is_some_and(|arg| arg == "--update-identity") {
        println!(
            "{}",
            serde_json::to_string(&teshi_core::version::build_identity())?
        );
        return Ok(());
    }
    if args
        .get(1)
        .is_some_and(|arg| arg == "--browser-broker-internal")
    {
        let options = internal_broker::InternalBrokerOptions::parse(
            std::iter::once(args[0].clone()).chain(args.iter().skip(2).cloned()),
        )?;
        let runtime = tokio::runtime::Runtime::new().context("create tokio runtime")?;
        return runtime.block_on(internal_broker::run(options));
    }
    let _update_participant = if teshi_update::transaction::registers_install_participant(&args) {
        teshi_update::transaction::participate(&std::env::current_exe()?)?
    } else {
        None
    };

    if args.iter().any(|arg| arg == "--daemon-internal") {
        let options = teshi_daemon::DaemonInternalOptions::parse_from(args);
        let runtime = tokio::runtime::Runtime::new().context("create tokio runtime")?;
        return runtime.block_on(teshi_daemon::run_daemon_internal(options));
    }

    if args.get(1).is_some_and(|arg| arg == "web") {
        let forwarded = std::iter::once(args[0].clone()).chain(args.into_iter().skip(2));
        let command = WebCommand::parse_from(forwarded);
        let runtime = tokio::runtime::Runtime::new().context("create tokio runtime")?;
        return runtime.block_on(teshi_daemon::run_client(command.options));
    }

    teshi_tui::run()
}
