//! Automatic recovery for ACP turns that a transient network failure ended.
//!
//! Agents do not all report such a failure as a JSON-RPC error. Cursor's ACP
//! adapter streams `Error: RetriableError: ...` as ordinary message text and
//! still ends the turn with `end_turn`, so that notice is recognized by its
//! text and held back from the transcript while a retry is possible.

use std::time::Duration;

use teshi_acp::AcpError;

/// Backoff schedule for automatically retrying a transiently failed ACP turn.
///
/// Retry `n` (zero-based) waits `backoff[n]`; the schedule length is the
/// retry budget, so an empty schedule disables automatic retries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnRetryPolicy {
    /// Delay before each retry, in order.
    pub backoff: Vec<Duration>,
}

impl Default for TurnRetryPolicy {
    /// Waits 3, 7, 15, 30, then 60 seconds: long enough to ride out a brief
    /// network drop without hiding a lasting outage for minutes.
    fn default() -> Self {
        Self {
            backoff: [3, 7, 15, 30, 60].map(Duration::from_secs).to_vec(),
        }
    }
}

impl TurnRetryPolicy {
    /// A policy that reports the first transient failure without retrying.
    pub fn disabled() -> Self {
        Self {
            backoff: Vec::new(),
        }
    }

    /// Number of automatic retries one user turn may spend.
    pub fn max_retries(&self) -> usize {
        self.backoff.len()
    }

    /// Delay before the zero-based `retry`, or `None` once the budget is spent.
    pub fn delay(&self, retry: usize) -> Option<Duration> {
        self.backoff.get(retry).copied()
    }
}

/// Prompt for a retry after the failed attempt already showed output or ran
/// tools: resending the original request could repeat that work.
pub(crate) const CONTINUE_AFTER_INTERRUPTION: &str = "The previous attempt was interrupted by a network error. Continue the task from where it stopped. First verify which operations have already completed; do not repeat completed tool actions.";

/// Cursor's ACP adapter prefixes failures it classifies as retriable with this.
const RETRIABLE_NOTICE_PREFIX: &str = "Error: RetriableError:";

/// Reported while a worker tears down the channel of a turn that already
/// finished; repeating the turn would duplicate a completed answer.
const CLOSED_CHANNEL_NOISE: &str = "WritableIterable is closed";

/// Lowercase markers of a lost or unreachable connection in remote error text.
const TRANSIENT_MARKERS: &[&str] = &[
    "retriableerror",
    "econnreset",
    "econnrefused",
    "econnaborted",
    "etimedout",
    "enotfound",
    "eai_again",
    "enetunreach",
    "enetdown",
    "ehostunreach",
    "socket hang up",
    "connection reset",
    "connection closed",
    "server disconnected",
    "stream closed",
    "network error",
    "fetch failed",
];

/// Returns the notice text when `chunk` is an agent's transient-failure
/// notice rather than model output.
pub(crate) fn transient_notice(chunk: &str) -> Option<&str> {
    let text = chunk.trim();
    (text.starts_with(RETRIABLE_NOTICE_PREFIX) && !text.contains(CLOSED_CHANNEL_NOISE))
        .then_some(text)
}

/// Whether a failed `session/prompt` reported a network failure that a
/// repeat can overcome.
///
/// Remote JSON-RPC errors arrive as `ProtocolViolation` with the agent's
/// message. Process exits, timeouts, and local failures are excluded: they
/// need a reconnect or the user's judgment, not a resend on this session.
pub(crate) fn is_transient_prompt_error(error: &AcpError) -> bool {
    match error {
        AcpError::ProtocolViolation(message) => is_transient_message(message),
        _ => false,
    }
}

fn is_transient_message(message: &str) -> bool {
    if message.contains(CLOSED_CHANNEL_NOISE) {
        return false;
    }
    let lower = message.to_ascii_lowercase();
    TRANSIENT_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transient_notice_matches_only_retriable_errors() {
        let cases = [
            (
                "\n\nError: RetriableError: [canceled] http/2 stream closed with error code CANCEL (0x8)",
                true,
            ),
            ("Error: RetriableError: WritableIterable is closed", false),
            (
                "\n\nError: [unauthenticated] Backend rejected authentication.",
                false,
            ),
            ("Error: NonRetriableError: quota exceeded", false),
            ("The RetriableError type is defined in client.ts", false),
            ("", false),
        ];
        for (chunk, expected) in cases {
            assert_eq!(transient_notice(chunk).is_some(), expected, "{chunk:?}");
        }
    }

    #[test]
    fn test_prompt_errors_retry_only_on_remote_network_failures() {
        let cases = [
            (
                AcpError::ProtocolViolation("Error: RetriableError: ECONNRESET".into()),
                true,
            ),
            (
                AcpError::ProtocolViolation("request to https://api failed: socket hang up".into()),
                true,
            ),
            (
                AcpError::ProtocolViolation(
                    "Error: RetriableError: WritableIterable is closed".into(),
                ),
                false,
            ),
            (AcpError::ProtocolViolation("prompt rejected".into()), false),
            (AcpError::ProcessExited, false),
            (AcpError::Timeout(Duration::from_secs(1)), false),
            (AcpError::AuthenticationRequired, false),
        ];
        for (error, expected) in cases {
            assert_eq!(is_transient_prompt_error(&error), expected, "{error}");
        }
    }

    #[test]
    fn test_policy_budget_follows_the_schedule() {
        let policy = TurnRetryPolicy::default();
        assert_eq!(policy.max_retries(), 5);
        assert_eq!(policy.delay(0), Some(Duration::from_secs(3)));
        assert_eq!(policy.delay(4), Some(Duration::from_secs(60)));
        assert_eq!(policy.delay(5), None);
        assert_eq!(TurnRetryPolicy::disabled().delay(0), None);
    }
}
