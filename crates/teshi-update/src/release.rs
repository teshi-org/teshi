//! Deterministic channel-aware release comparison.

use crate::{Result, invalid, manifest::validate_identity};
use teshi_core::version::{BuildIdentity, ReleaseChannel};

/// Determines whether a validated candidate can replace the current build.
///
/// # Errors
/// Rejects implicit channel switches and inconsistent nightly sequence identity.
pub fn is_update(
    current: &BuildIdentity,
    candidate: &BuildIdentity,
    explicit_channel: bool,
) -> Result<bool> {
    validate_identity(candidate)?;
    let current_version =
        semver::Version::parse(&current.semver).map_err(|e| invalid(e.to_string()))?;
    let candidate_version =
        semver::Version::parse(&candidate.semver).map_err(|e| invalid(e.to_string()))?;
    if candidate_version < current_version {
        return Ok(false);
    }
    if current.channel != candidate.channel {
        if current.channel == ReleaseChannel::Dev {
            return Ok(true);
        }
        if !explicit_channel {
            return Err(invalid("Changing release channel requires --channel"));
        }
        return Ok(true);
    }
    if current.channel == ReleaseChannel::Stable {
        return Ok(candidate_version > current_version);
    }
    if current.build_sequence == candidate.build_sequence && current.git_sha != candidate.git_sha {
        return Err(invalid(
            "Nightly sequence has conflicting commit identities",
        ));
    }
    Ok(candidate.build_sequence > current.build_sequence && candidate.git_sha != current.git_sha)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn nightly(sequence: u64, sha: char) -> BuildIdentity {
        BuildIdentity {
            semver: "0.7.10".into(),
            channel: ReleaseChannel::Nightly,
            git_sha: sha.to_string().repeat(40),
            build_timestamp: "2026-09-08T00:00:00Z".into(),
            build_sequence: sequence,
        }
    }
    #[test]
    fn nightly_orders_same_day_by_sequence_not_sha() {
        assert!(is_update(&nightly(10, 'f'), &nightly(11, 'a'), false).unwrap());
        assert!(!is_update(&nightly(11, 'a'), &nightly(10, 'f'), false).unwrap());
        assert!(!is_update(&nightly(10, 'a'), &nightly(11, 'a'), false).unwrap());
        assert!(is_update(&nightly(10, 'a'), &nightly(10, 'b'), false).is_err());
    }
    #[test]
    fn channel_switch_is_explicit_and_never_lowers_base_version() {
        let current = nightly(10, 'a');
        let mut candidate = nightly(11, 'b');
        candidate.channel = ReleaseChannel::Stable;
        assert!(is_update(&current, &candidate, false).is_err());
        assert!(is_update(&current, &candidate, true).unwrap());
        candidate.semver = "0.7.9".into();
        assert!(!is_update(&current, &candidate, true).unwrap());
    }
}
