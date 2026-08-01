//! `teshi auth` — manage API keys on the shared model-profile store.
//!
//! Credentials live on engine [`ModelProfile`] files under app data
//! `model-profiles/`, the same store used by Desktop and the daemon.

use anyhow::{Context, Result, bail};
use teshi_engine::{
    ModelProfile, PROVIDER_ANTHROPIC, PROVIDER_DEEPSEEK_OPENAI, PROVIDER_OPENAI, app_data_dir,
    list_profiles, load_profile, map_legacy_provider_id, model_profiles_dir, save_profile,
    set_active_id,
};

/// Prompts the user to pick a provider and enter an API key, then upserts a profile.
fn interactive_login(provider_arg: Option<String>) -> Result<()> {
    let provider_label = if let Some(p) = provider_arg {
        p
    } else {
        let providers = vec![
            PROVIDER_OPENAI.to_string(),
            PROVIDER_ANTHROPIC.to_string(),
            PROVIDER_DEEPSEEK_OPENAI.to_string(),
            "ollama (openai + custom base_url)".to_string(),
        ];
        inquire::Select::new("Select provider:", providers).prompt()?
    };

    let provider = if provider_label.starts_with("ollama") {
        PROVIDER_OPENAI.to_string()
    } else {
        map_legacy_provider_id(&provider_label)
    };

    let key = inquire::Password::new(&format!("Enter API key for {provider}:"))
        .without_confirmation()
        .prompt()
        .context("failed to read API key")?;
    if key.trim().is_empty() {
        bail!("API key must not be empty");
    }

    let mut base_url = String::new();
    if (provider_label.starts_with("ollama") || provider == PROVIDER_OPENAI)
        && let Ok(url) = inquire::Text::new("Base URL (empty for provider default):")
            .with_default("")
            .prompt()
    {
        base_url = url.trim().to_string();
    }
    if provider_label.starts_with("ollama") && base_url.is_empty() {
        base_url = "http://localhost:11434/v1".into();
    }

    let model_default = match provider.as_str() {
        PROVIDER_DEEPSEEK_OPENAI => "deepseek-chat",
        PROVIDER_ANTHROPIC => "claude-sonnet-4-20250514",
        _ if provider_label.starts_with("ollama") => "llama3",
        _ => "gpt-4o-mini",
    };
    let model = inquire::Text::new("Model id:")
        .with_default(model_default)
        .prompt()
        .unwrap_or_else(|_| model_default.to_string());

    // Prefer updating an existing profile whose provider AND base_url match the input.
    // Using base_url here prevents a different-host openai profile from being overwritten.
    let mut profile = find_profile_for_provider(&provider, &base_url)?.unwrap_or_else(|| {
        let mut p = ModelProfile::new(format!("{provider} ({model})"));
        p.provider = provider.clone();
        p
    });
    profile.provider = provider.clone();
    profile.model_id = model.trim().to_string();
    if !base_url.is_empty() {
        profile.base_url = base_url;
    }
    profile.api_key = key.trim().to_string();

    save_profile(&mut profile)?;
    set_active_id(&profile.id)?;

    println!(
        "Saved and activated profile '{}' (provider={provider}).",
        profile.name
    );
    println!("  Store: {}", model_profiles_dir()?.display());
    Ok(())
}

/// Normalize a base URL for comparison: strip trailing slashes and lowercase.
fn normalize_base_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_lowercase()
}

/// Find an existing profile matching `provider` and `base_url`.
///
/// When `base_url` is non-empty, requires an exact normalized match.
/// When `base_url` is empty, prefers the first profile with an empty base_url
/// for that provider (i.e. the provider-default endpoint).
fn find_profile_for_provider(provider: &str, base_url: &str) -> Result<Option<ModelProfile>> {
    let normalized_input = normalize_base_url(base_url);
    let list = list_profiles()?;

    if !normalized_input.is_empty() {
        // Non-empty base_url: require an exact normalized match to avoid overwriting a
        // profile that points at a different host.
        for public in &list.profiles {
            if public.provider == provider
                && normalize_base_url(&public.base_url) == normalized_input
            {
                return Ok(Some(load_profile(&public.id)?));
            }
        }
        return Ok(None);
    }

    // Empty base_url: prefer the first profile with an empty (provider-default) base_url.
    for public in &list.profiles {
        if public.provider == provider && public.base_url.trim().is_empty() {
            return Ok(Some(load_profile(&public.id)?));
        }
    }
    Ok(None)
}

fn list_credentials() -> Result<()> {
    let list = list_profiles()?;
    if list.profiles.is_empty() {
        println!("No model profiles stored.");
        println!("Use 'teshi auth login' or the TUI model panel (m) to add one.");
        return Ok(());
    }

    println!(
        "{:<24} {:<18} {:<16} Key (masked)",
        "Name", "Provider", "Active"
    );
    println!("{}", "-".repeat(72));
    for p in &list.profiles {
        let active = if p.active { "yes" } else { "" };
        let key = if p.api_key_configured {
            &p.api_key_masked
        } else {
            "(none)"
        };
        println!(
            "{:<24} {:<18} {:<16} {}",
            truncate(&p.name, 24),
            p.provider,
            active,
            key
        );
    }
    println!("{}", "-".repeat(72));
    println!("  Store: {}", model_profiles_dir()?.display());
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn remove_credential(provider: &str) -> Result<()> {
    let provider = map_legacy_provider_id(provider);
    let list = list_profiles()?;
    let matches: Vec<_> = list
        .profiles
        .into_iter()
        .filter(|p| p.provider == provider)
        .collect();
    if matches.is_empty() {
        bail!("no profiles found for provider '{provider}'");
    }

    let target = if matches.len() == 1 {
        matches.into_iter().next().unwrap()
    } else {
        let labels: Vec<String> = matches
            .iter()
            .map(|p| format!("{} ({})", p.name, p.id))
            .collect();
        let choice = inquire::Select::new("Select profile to clear key from:", labels).prompt()?;
        let idx = matches
            .iter()
            .position(|p| choice.contains(&p.id))
            .unwrap_or(0);
        matches.into_iter().nth(idx).unwrap()
    };

    let mut profile = load_profile(&target.id)?;
    profile.api_key.clear();
    save_profile(&mut profile)?;
    println!(
        "Cleared API key on profile '{}' (provider={provider}).",
        profile.name
    );
    Ok(())
}

fn show_status() -> Result<()> {
    let list = list_profiles().unwrap_or_else(|_| teshi_engine::ModelProfileList {
        profiles: Vec::new(),
        active_id: None,
    });
    let data_dir = app_data_dir()?;
    let profiles_dir = model_profiles_dir()?;

    println!("teshi LLM profile status\n");
    println!("App data:      {}", data_dir.display());
    println!("Profiles:      {}", profiles_dir.display());
    println!("Profile count: {}", list.profiles.len());
    if let Some(active) = &list.active_id {
        println!("Active id:     {active}");
    } else {
        println!("Active id:     (none)");
    }

    if list.profiles.is_empty() {
        println!("\nNo profiles yet. Run 'teshi auth login' to create one.");
    } else {
        println!();
        println!("{:<24} {:<18} {:<8} Model", "Name", "Provider", "Key?");
        println!("{}", "-".repeat(72));
        for p in &list.profiles {
            println!(
                "{:<24} {:<18} {:<8} {}",
                truncate(&p.name, 24),
                p.provider,
                if p.api_key_configured { "yes" } else { "no" },
                p.model_id
            );
        }
    }

    println!("\nEnv fallback: TESHI_LLM_API_KEY / TESHI_LLM_BASE_URL / TESHI_LLM_MODEL");
    Ok(())
}

/// Migrate API keys from well-known environment variables into model profiles.
fn migrate_from_env() -> Result<()> {
    let known_env_pairs = [
        (PROVIDER_OPENAI, "TESHI_OPENAI_API_KEY"),
        (PROVIDER_DEEPSEEK_OPENAI, "TESHI_DEEPSEEK_API_KEY"),
        (PROVIDER_ANTHROPIC, "TESHI_ANTHROPIC_API_KEY"),
        (PROVIDER_OPENAI, "OPENAI_API_KEY"),
        (PROVIDER_ANTHROPIC, "ANTHROPIC_API_KEY"),
        (PROVIDER_DEEPSEEK_OPENAI, "DEEPSEEK_API_KEY"),
    ];

    let mut migrated = 0u32;
    for (provider, env_var) in &known_env_pairs {
        let Ok(val) = std::env::var(env_var) else {
            continue;
        };
        if val.trim().is_empty() {
            continue;
        }
        if let Some(existing) = find_profile_for_provider(provider, "")? {
            if !existing.api_key.is_empty() {
                println!("  skipping {provider} (env var {env_var}): profile already has a key");
                continue;
            }
            let mut profile = existing;
            profile.api_key = val;
            save_profile(&mut profile)?;
        } else {
            let mut profile = ModelProfile::new(format!("{provider} (migrated)"));
            profile.provider = (*provider).into();
            profile.api_key = val;
            save_profile(&mut profile)?;
            if migrated == 0 {
                set_active_id(&profile.id)?;
            }
        }
        migrated += 1;
        println!("  migrated {provider} from env var {env_var}");
    }

    if migrated == 0 {
        println!("No environment variable API keys found to migrate.");
        return Ok(());
    }
    println!(
        "\n{migrated} profile key(s) migrated into {}.",
        model_profiles_dir()?.display()
    );
    println!("You should now remove the original environment variables for security.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_base_url_strips_slash_and_lowercases() {
        assert_eq!(
            normalize_base_url("https://api.openai.com/v1/"),
            "https://api.openai.com/v1"
        );
        assert_eq!(
            normalize_base_url("HTTPS://API.OPENAI.COM/V1"),
            "https://api.openai.com/v1"
        );
        assert_eq!(normalize_base_url("  https://x.com/  "), "https://x.com");
        assert_eq!(normalize_base_url(""), "");
        assert_eq!(normalize_base_url("  "), "");
    }

    #[test]
    fn test_normalize_base_url_multiple_trailing_slashes() {
        assert_eq!(
            normalize_base_url("https://example.com/v1///"),
            "https://example.com/v1"
        );
    }
}

/// Dispatches an `AuthCommand` to its implementation.
pub fn handle_auth_command(cmd: &crate::cli::AuthCommand) -> Result<()> {
    // Ensure desktop→teshi and TUI legacy imports have run before CLI edits.
    let _ = list_profiles();
    match cmd {
        crate::cli::AuthCommand::Login { provider } => interactive_login(provider.clone()),
        crate::cli::AuthCommand::List => list_credentials(),
        crate::cli::AuthCommand::Remove { provider } => remove_credential(provider),
        crate::cli::AuthCommand::Status => show_status(),
        crate::cli::AuthCommand::Migrate => migrate_from_env(),
    }
}
