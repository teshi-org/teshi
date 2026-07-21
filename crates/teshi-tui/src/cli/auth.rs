use anyhow::{Context, Result, bail};

use crate::auth::CredentialManager;
use crate::config::{self};

/// Prompts the user to pick a provider interactively and enter an API key.
fn interactive_login(provider_arg: Option<String>) -> Result<()> {
    let provider = if let Some(p) = provider_arg {
        p
    } else {
        let providers = vec![
            "deepseek".to_string(),
            "openai".to_string(),
            "ollama".to_string(),
        ];
        inquire::Select::new("Select provider:", providers).prompt()?
    };

    let key = inquire::Password::new(&format!("Enter API key for {}:", provider))
        .without_confirmation()
        .prompt()
        .context("failed to read API key")?;

    if key.trim().is_empty() {
        bail!("API key must not be empty");
    }

    let mgr = CredentialManager::new()?;
    let mut creds = mgr.load()?;

    creds.insert(
        provider.clone(),
        crate::auth::manager::CredentialEntry {
            credential_type: "api_key".into(),
            key: key.trim().to_string(),
        },
    );

    mgr.save(&creds)?;
    println!("Credentials saved for provider '{}'.", provider);
    println!("  Path: {}", mgr.path().display());
    Ok(())
}

fn list_credentials() -> Result<()> {
    let mgr = CredentialManager::new()?;
    let creds = mgr.load()?;

    if creds.is_empty() {
        println!("No credentials stored.");
        println!("Use 'teshi auth login' to add a provider API key.");
        return Ok(());
    }

    println!("{:<16} {:>8}  Key (masked)", "Provider", "Type");
    println!("{}", "-".repeat(54));
    for (name, entry) in &creds {
        let masked = CredentialManager::mask_key(&entry.key);
        println!("{:<16} {:>8}  {}", name, entry.credential_type, masked);
    }
    println!("{}", "-".repeat(54));
    println!("  Path: {}", mgr.path().display());
    Ok(())
}

fn remove_credential(provider: &str) -> Result<()> {
    let mgr = CredentialManager::new()?;
    let mut creds = mgr.load()?;

    if creds.remove(provider).is_none() {
        bail!("no credentials stored for provider '{}'", provider);
    }

    mgr.save(&creds)?;
    println!("Removed credentials for provider '{}'.", provider);
    Ok(())
}

fn show_status() -> Result<()> {
    let mgr = CredentialManager::new()?;
    let creds = mgr.load()?;
    let config = config::load_config()?;

    println!("teshi credential & configuration status\n");

    // Show configured providers
    if config.providers.is_empty() {
        println!("No providers configured.");
    } else {
        println!("{:<16} {:>6}  {:>8}  Base URL", "Provider", "Key?", "Model");
        println!("{}", "-".repeat(80));
        for (name, provider) in &config.providers {
            let has_key = provider
                .api_key
                .as_ref()
                .map(|k| !k.is_empty())
                .unwrap_or(false)
                || creds.contains_key(name.as_str());
            let key_status = if has_key { "yes" } else { "no" };
            let model = provider.model.as_deref().unwrap_or("-");
            let url = provider.base_url.as_deref().unwrap_or("-");
            println!("{:<16} {:>6}  {:>8}  {}", name, key_status, model, url);
        }
        println!("{}", "-".repeat(80));
    }

    if let Some(ref default) = config.default_provider {
        println!("Default provider: {}", default);
    } else {
        println!("Default provider: (none set)");
    }

    println!("\nCredential store: {}", mgr.path().display());
    println!("  Stored providers: {}", creds.len());

    Ok(())
}

/// Migrates API keys from environment variables to `auth.json`.
///
/// Scans known variables like `TESHI_OPENAI_API_KEY`, `TESHI_DEEPSEEK_API_KEY`
/// and writes matching entries into the credential store.
fn migrate_from_env() -> Result<()> {
    let mgr = CredentialManager::new()?;
    let mut creds = mgr.load()?;

    let known_env_pairs = [
        ("openai", "TESHI_OPENAI_API_KEY"),
        ("deepseek", "TESHI_DEEPSEEK_API_KEY"),
        ("ollama", "TESHI_OLLAMA_API_KEY"),
        ("anthropic", "TESHI_ANTHROPIC_API_KEY"),
        ("openai", "OPENAI_API_KEY"),
    ];

    let mut migrated = 0u32;

    for (provider_name, env_var) in &known_env_pairs {
        if let Ok(val) = std::env::var(env_var) {
            if creds.contains_key(*provider_name) {
                println!(
                    "  skipping {} (env var {}): already has credentials",
                    provider_name, env_var
                );
                continue;
            }
            creds.insert(
                provider_name.to_string(),
                crate::auth::manager::CredentialEntry {
                    credential_type: "api_key".into(),
                    key: val,
                },
            );
            migrated += 1;
            println!("  migrated {} from env var {}", provider_name, env_var);
        }
    }

    if migrated == 0 {
        println!("No environment variable API keys found to migrate.");
        return Ok(());
    }

    mgr.save(&creds)?;
    println!(
        "\n{} credential(s) migrated to {}.",
        migrated,
        mgr.path().display()
    );
    println!("You should now remove the original environment variables for security.");
    Ok(())
}

/// Dispatches an `AuthCommand` to its implementation.
pub fn handle_auth_command(cmd: &crate::cli::AuthCommand) -> Result<()> {
    match cmd {
        crate::cli::AuthCommand::Login { provider } => interactive_login(provider.clone()),
        crate::cli::AuthCommand::List => list_credentials(),
        crate::cli::AuthCommand::Remove { provider } => remove_credential(provider),
        crate::cli::AuthCommand::Status => show_status(),
        crate::cli::AuthCommand::Migrate => migrate_from_env(),
    }
}
