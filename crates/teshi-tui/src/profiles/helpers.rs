//! Convenience wrappers around `teshi_engine::model_profile` for the TUI.

use anyhow::Result;
use serde_json::json;
use teshi_engine::{
    ModelProfile, delete_profile as engine_delete, list_profiles, load_profile,
    map_legacy_provider_id, read_active_id as engine_read_active, save_profile, set_active_id,
};

/// Load all profiles from the shared store (full keys for local editing).
pub fn load_all() -> Vec<ModelProfile> {
    let Ok(list) = list_profiles() else {
        return Vec::new();
    };
    let mut profiles = Vec::new();
    for public in list.profiles {
        if let Ok(full) = load_profile(&public.id) {
            profiles.push(full);
        }
    }
    profiles.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    profiles
}

/// Read the active profile id from the shared store.
pub fn read_active_id() -> Option<String> {
    engine_read_active().ok().flatten()
}

/// Persist the active-profile pointer.
pub fn write_active_id(id: &str) -> Result<()> {
    set_active_id(id)?;
    Ok(())
}

/// Delete a profile from the shared store.
pub fn delete_profile(id: &str) -> Result<()> {
    engine_delete(id)?;
    Ok(())
}

/// Map free-form / legacy provider names to built-in engine ids.
pub fn normalize_provider(name: &str) -> String {
    map_legacy_provider_id(name)
}

/// Temperature stored in `chat_options`, defaulting to `0.7`.
pub fn profile_temperature(profile: &ModelProfile) -> f32 {
    profile
        .chat_options
        .get("temperature")
        .and_then(|v| v.as_f64())
        .map(|v| v as f32)
        .unwrap_or(0.7)
}

/// Write temperature into `chat_options`.
pub fn set_profile_temperature(profile: &mut ModelProfile, temperature: f32) {
    profile
        .chat_options
        .insert("temperature".into(), json!(temperature));
}

/// Save a profile to the shared store (preserves empty-key semantics).
pub fn save(profile: &mut ModelProfile) -> Result<()> {
    save_profile(profile)?;
    Ok(())
}
