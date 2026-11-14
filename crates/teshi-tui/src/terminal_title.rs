//! Terminal title reporting for the TUI.
//!
//! While an agent is working, the title is the only cross-window status
//! signal: the in-app indicators disappear as soon as the user switches tabs
//! or windows. The title therefore aggregates every agent thread instead of
//! following the focused conversation, and never includes chat content.

use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

use crate::app::{AiStatus, App};

/// Minimum time between title writes while a frame is animating. Writing per
/// draw would spam terminals and multiplexer logs; 250 ms keeps the spinner
/// observably alive without that cost.
const TICK: Duration = Duration::from_millis(250);

/// OSC strings may be logged by the terminal or a multiplexer, so bound each
/// write and strip control characters before emitting it.
const MAX_TITLE_CHARS: usize = 100;

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Fallback when the terminal cannot be assumed to animate.
const STATIC_RUNNING: &str = "●";

/// Result of the most recently completed turn. Shown until the next turn
/// starts so a run that finished while the user was elsewhere stays visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    None,
    Succeeded,
    Failed,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Activity {
    /// Every conversation in the AI tab, including idle ones nobody is
    /// waiting for.
    total: usize,
    running: usize,
    /// Finished turns whose answer is not on screen yet.
    unseen: usize,
    approval: usize,
    failed: usize,
}

impl Activity {
    fn from_app(app: &App) -> Self {
        let mut activity = Self {
            total: app.agents.len(),
            ..Self::default()
        };
        for agent in &app.agents {
            if agent.unseen_completion {
                activity.unseen += 1;
            }
            match agent.status {
                AiStatus::Waiting => activity.running += 1,
                AiStatus::AwaitingApproval => activity.approval += 1,
                AiStatus::Error => activity.failed += 1,
                AiStatus::Idle => {}
            }
        }
        activity
    }

    /// Conversations the user is being asked to look at: still running, or
    /// finished with an unread answer. Idle ones nobody waits for are left out,
    /// so the count never grows just because a conversation stays open.
    fn tracked(self) -> usize {
        self.running + self.unseen
    }

    fn active(self) -> bool {
        self.running > 0 || self.approval > 0
    }
}

/// Owns the current title plus the throttle and transition state around it.
pub(crate) struct TerminalTitle {
    enabled: bool,
    animate: bool,
    last: String,
    last_write: Option<Instant>,
    frame: usize,
    outcome: Outcome,
    was_active: bool,
}

impl TerminalTitle {
    /// Prepare title reporting when the environment allows it.
    pub(crate) fn new() -> Self {
        Self {
            enabled: title_enabled(),
            animate: supports_animation(),
            last: String::new(),
            last_write: None,
            frame: 0,
            outcome: Outcome::None,
            was_active: false,
        }
    }

    /// Recompute the title and write it when it changed and the throttle
    /// allows. IO failures are ignored: a terminal that rejects OSC must not
    /// take the UI down.
    pub(crate) fn sync(&mut self, app: &App) {
        if !self.enabled {
            return;
        }

        let activity = Activity::from_app(app);
        let active = activity.active();
        if active && !self.was_active {
            self.outcome = Outcome::None;
        } else if !active && self.was_active {
            self.outcome = if activity.failed > 0 {
                Outcome::Failed
            } else {
                Outcome::Succeeded
            };
        }
        self.was_active = active;

        let now = Instant::now();
        if active
            && self.animate
            && self
                .last_write
                .is_none_or(|at| now.duration_since(at) >= TICK)
        {
            self.frame = self.frame.wrapping_add(1);
        }
        let mark = if self.animate {
            SPINNER[self.frame % SPINNER.len()]
        } else {
            STATIC_RUNNING
        };
        let title = compose(
            &app.project_switch.active.identity.display_name(),
            activity,
            self.outcome,
            mark,
        );
        if title == self.last {
            return;
        }
        // State changes are written immediately; only the animating running
        // frame waits for the next tick.
        if active
            && self
                .last_write
                .is_some_and(|at| now.duration_since(at) < TICK)
        {
            return;
        }
        if write_title(&title).is_ok() {
            self.last = title;
            self.last_write = Some(now);
        } else {
            // Retry the same title on the next iteration.
            self.last.clear();
        }
    }

    /// Restore the shell's title slot. Idempotent so both the explicit exit
    /// path and the Drop implementation can call it.
    pub(crate) fn reset(&mut self) {
        if !self.enabled {
            return;
        }
        self.enabled = false;
        // Never clear a title that Teshi did not write.
        if self.last_write.is_some() {
            let _ = write_title("");
        }
    }
}

impl Drop for TerminalTitle {
    fn drop(&mut self) {
        self.reset();
    }
}

/// Report the title only on a real terminal and only when neither the BDD
/// harness nor the user asked Teshi to keep its hands off the title.
fn title_enabled() -> bool {
    io::stdout().is_terminal()
        && std::env::var_os("TESHI_NO_TITLE").is_none()
        && std::env::var_os("TESHI_TUI_RUNNER").is_none()
}

/// A dumb terminal cannot render a moving frame; keep a static marker and let
/// the title change only with the state.
fn supports_animation() -> bool {
    std::env::var("TERM").map_or(true, |term| term != "dumb")
}

/// Build the title from identity and aggregate state.
///
/// The body deliberately contains only the project name and counts: agent
/// titles and chat text can carry user content.
fn compose(identity: &str, activity: Activity, outcome: Outcome, mark: &str) -> String {
    let base = if identity.is_empty() || identity.eq_ignore_ascii_case("teshi") {
        "teshi".to_string()
    } else {
        format!("teshi · {identity}")
    };

    if activity.tracked() > 0 {
        // The numerator is what the user owes a look at, so the badge never
        // claims progress that is already read: two running conversations read
        // `0/2`, one of them finishing unread reads `1/2`, reading it `0/1`.
        let mut title = if activity.running == 0 {
            // Nothing runs, so a `N/M running` count would be a lie.
            format!("{mark} {base} · {} unread", activity.unseen)
        } else if activity.tracked() > 1 || activity.unseen > 0 {
            format!(
                "{mark} {base} · {}/{} running",
                activity.unseen,
                activity.tracked()
            )
        } else {
            format!("{mark} {base}")
        };
        if activity.approval > 0 {
            title.push_str(&format!(", {} waiting", activity.approval));
        }
        return title;
    }
    if activity.approval > 0 {
        return if activity.total > 1 {
            format!(
                "? {base} · {}/{} waiting",
                activity.approval, activity.total
            )
        } else {
            format!("? {base} · waiting")
        };
    }
    if activity.failed > 0 {
        return format!("✗ {base} · failed");
    }
    match outcome {
        Outcome::Succeeded => format!("✓ {base}"),
        Outcome::Failed => format!("✗ {base}"),
        Outcome::None => base,
    }
}

/// Strip control characters (OSC injection) and cap the length.
fn sanitize(title: &str) -> String {
    title
        .chars()
        .filter(|ch| !ch.is_control())
        .take(MAX_TITLE_CHARS)
        .collect()
}

/// OSC 0 and 2 are emitted together for terminals that distinguish the icon
/// name from the window title. tmux consumes OSC 2 for its own window name,
/// so the same title also goes through a passthrough wrapper for the outer
/// terminal. Without allow-passthrough tmux drops the wrapped copy and still
/// updates its own window name from the plain sequence.
fn osc_sequence(title: &str, tmux: bool) -> String {
    let title = sanitize(title);
    let direct = format!("\x1b]0;{title}\x07\x1b]2;{title}\x07");
    if tmux {
        format!("{direct}\x1bPtmux;\x1b\x1b]0;{title}\x07\x1b\x1b]2;{title}\x07\x1b\\")
    } else {
        direct
    }
}

fn write_title(title: &str) -> io::Result<()> {
    let payload = osc_sequence(title, std::env::var_os("TMUX").is_some());
    let mut stdout = io::stdout().lock();
    stdout.write_all(payload.as_bytes())?;
    stdout.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn activity(total: usize, running: usize, approval: usize, failed: usize) -> Activity {
        Activity {
            total,
            running,
            approval,
            failed,
            ..Activity::default()
        }
    }

    fn with_unseen(mut activity: Activity, unseen: usize) -> Activity {
        activity.unseen = unseen;
        activity
    }

    #[test]
    fn idle_title_is_static_identity() {
        assert_eq!(
            compose("proj", activity(3, 0, 0, 0), Outcome::None, "⠋"),
            "teshi · proj"
        );
        assert_eq!(
            compose("teshi", activity(1, 0, 0, 0), Outcome::None, "⠋"),
            "teshi"
        );
    }

    #[test]
    fn running_title_animates_and_counts_tracked_agents() {
        let first = compose("proj", activity(3, 2, 0, 0), Outcome::None, "⠋");
        assert!(first.starts_with('⠋'));
        assert!(first.contains("0/2 running"));
        assert_ne!(
            first,
            compose("proj", activity(3, 2, 0, 0), Outcome::None, "⠙")
        );
    }

    #[test]
    fn unread_answers_are_the_numerator() {
        // One conversation finished unread while another still runs.
        assert_eq!(
            compose(
                "proj",
                with_unseen(activity(3, 1, 0, 0), 1),
                Outcome::None,
                "⠋"
            ),
            "⠋ teshi · proj · 1/2 running"
        );
        // Reading it leaves only the running conversation.
        assert_eq!(
            compose("proj", activity(3, 1, 0, 0), Outcome::None, "⠋"),
            "⠋ teshi · proj"
        );
        // A lone unread answer still has to surface, with no running count to
        // inflate it.
        assert_eq!(
            compose(
                "proj",
                with_unseen(activity(1, 0, 0, 0), 1),
                Outcome::None,
                "⠋"
            ),
            "⠋ teshi · proj · 1 unread"
        );
    }

    #[test]
    fn idle_conversations_stay_out_of_the_denominator() {
        assert_eq!(
            compose("proj", activity(9, 2, 0, 0), Outcome::None, "⠋"),
            "⠋ teshi · proj · 0/2 running"
        );
    }

    #[test]
    fn dumb_terminal_uses_a_static_marker() {
        assert_eq!(
            compose("proj", activity(1, 1, 0, 0), Outcome::None, STATIC_RUNNING),
            "● teshi · proj"
        );
    }

    #[test]
    fn single_agent_run_omits_counts() {
        assert_eq!(
            compose("proj", activity(1, 1, 0, 0), Outcome::None, "⠋"),
            "⠋ teshi · proj"
        );
    }

    #[test]
    fn approval_takes_over_when_nothing_runs() {
        assert_eq!(
            compose("proj", activity(2, 0, 2, 0), Outcome::None, "⠋"),
            "? teshi · proj · 2/2 waiting"
        );
    }

    #[test]
    fn running_with_approval_mentions_both() {
        assert_eq!(
            compose("proj", activity(3, 2, 1, 0), Outcome::None, "⠋"),
            "⠋ teshi · proj · 0/2 running, 1 waiting"
        );
    }

    #[test]
    fn failure_mark_wins_over_previous_success() {
        assert_eq!(
            compose("proj", activity(1, 0, 0, 1), Outcome::Succeeded, "⠋"),
            "✗ teshi · proj · failed"
        );
    }

    #[test]
    fn completed_outcome_is_kept_until_next_run() {
        assert_eq!(
            compose("proj", activity(1, 0, 0, 0), Outcome::Succeeded, "⠋"),
            "✓ teshi · proj"
        );
        assert_eq!(
            compose("proj", activity(1, 0, 0, 0), Outcome::Failed, "⠋"),
            "✗ teshi · proj"
        );
    }

    #[test]
    fn title_is_sanitized_and_bounded() {
        assert_eq!(sanitize("a\u{1b}]2;x\u{07}b"), "a]2;xb");
        assert_eq!(sanitize(&"x".repeat(200)).chars().count(), MAX_TITLE_CHARS);
    }

    #[test]
    fn osc_writes_both_title_slots() {
        assert_eq!(osc_sequence("t", false), "\x1b]0;t\x07\x1b]2;t\x07");
        assert_eq!(
            osc_sequence("t", true),
            "\x1b]0;t\x07\x1b]2;t\x07\x1bPtmux;\x1b\x1b]0;t\x07\x1b\x1b]2;t\x07\x1b\\"
        );
    }
}
