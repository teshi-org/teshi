//! Internal transport-only Broker process entry point.

use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use teshi_browser_broker::{
    BrokerRuntime, BrokerServerConfig, CHROME_DISCOVERY_PORT, PrivateCredentialStore,
};

#[derive(Debug, Parser)]
#[command(name = "teshi", disable_help_flag = true, disable_version_flag = true)]
pub(crate) struct InternalBrokerOptions {
    /// Per-OS-user state directory, never a project directory.
    #[arg(long, value_name = "PATH")]
    state_dir: PathBuf,
    /// Exact Chrome extension origin authorized for credential delivery; repeat to pair installs.
    #[arg(
        long = "trusted-extension-origin",
        value_name = "ORIGIN",
        required = true,
        action = clap::ArgAction::Append
    )]
    trusted_extension_origins: Vec<String>,
    /// Discovery port; zero selects an ephemeral port for isolated tests only.
    #[arg(long, default_value_t = CHROME_DISCOVERY_PORT)]
    discovery_port: u16,
    /// Enable the migration-only P0 Navigation/Snapshot command surface.
    #[arg(long)]
    enable_p0_control: bool,
}

impl InternalBrokerOptions {
    pub(crate) fn parse(args: impl IntoIterator<Item = String>) -> Result<Self> {
        Self::try_parse_from(args).context("parse internal browser broker arguments")
    }
}

pub(crate) async fn run(options: InternalBrokerOptions) -> Result<()> {
    let mut config =
        BrokerServerConfig::with_trusted_extension_origins(options.trusted_extension_origins);
    config.discovery_addr =
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), options.discovery_port);
    config.broker_features = vec!["transport.v1".into()];
    if options.enable_p0_control {
        config.broker_features.push("p0.control".into());
    }
    let runtime = BrokerRuntime::start(config)
        .await
        .map_err(|error| anyhow::anyhow!(error.message))?;
    let credentials = PrivateCredentialStore::new(options.state_dir);
    runtime
        .persist_private_credential(&credentials)
        .map_err(|error| anyhow::anyhow!(error.message))?;
    println!(
        "BROWSER_BROKER_READY {}",
        serde_json::to_string(&runtime.endpoint_record())?
    );
    std::io::stdout()
        .flush()
        .context("flush internal broker readiness record")?;

    runtime.run_state_machine().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_arguments_require_an_explicit_exact_origin_and_state_directory() {
        let first_origin = "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let second_origin = "chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let options = InternalBrokerOptions::parse([
            "teshi".into(),
            "--state-dir".into(),
            "C:/Users/test/AppData/Local/Teshi".into(),
            "--trusted-extension-origin".into(),
            second_origin.into(),
            "--trusted-extension-origin".into(),
            first_origin.into(),
            "--enable-p0-control".into(),
        ])
        .unwrap();
        assert_eq!(
            options.state_dir,
            PathBuf::from("C:/Users/test/AppData/Local/Teshi")
        );
        assert_eq!(options.trusted_extension_origins.len(), 2);
        assert!(
            options
                .trusted_extension_origins
                .contains(&first_origin.into())
        );
        assert!(
            options
                .trusted_extension_origins
                .contains(&second_origin.into())
        );
        assert!(options.enable_p0_control);

        assert!(
            InternalBrokerOptions::parse(
                ["teshi".into(), "--state-dir".into(), "C:/state".into(),]
            )
            .is_err()
        );
    }
}
