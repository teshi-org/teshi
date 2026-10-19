//! Project identity, generation, and switch-transaction state for the TUI.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use teshi_engine::{ProjectGeneration, ProjectIdentity, ProjectStamp, RecentProjectEntry};

/// A project-scoped object that must be resolved before activation can change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirtyProjectObject {
    Feature(PathBuf),
    Requirement(String),
    TestPoint(String),
    AgentProposal,
}

impl DirtyProjectObject {
    /// Describe the unsaved project-owned object in a switch confirmation.
    pub fn label(&self) -> String {
        match self {
            Self::Feature(path) => format!("Feature buffer: {}", path.display()),
            Self::Requirement(id) => format!("Requirement draft: {id}"),
            Self::TestPoint(id) => format!("Test-point edit: {id}"),
            Self::AgentProposal => "Unapplied Agent proposal".to_string(),
        }
    }
}

/// Lifecycle stage of a project switch request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchPhase {
    Active,
    ResolveTarget,
    ResolveDirty,
    QuiesceTasks,
    Prepare,
    Commit,
}

/// User-visible switch overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitchOverlay {
    None,
    Picker { selection: usize },
    OpenPath { input: String },
    Dirty { target: ProjectIdentity },
    Busy { target: ProjectIdentity },
}

/// A pending request that has not reached commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingProjectSwitch {
    pub target: ProjectIdentity,
    pub dirty: Vec<DirtyProjectObject>,
}

/// State shared by the switch UI and project-scoped event guards.
#[derive(Debug, Clone)]
pub struct ProjectSwitchState {
    pub active: ProjectStamp,
    pub phase: SwitchPhase,
    pub overlay: SwitchOverlay,
    pub pending: Option<PendingProjectSwitch>,
    pub recent: Vec<RecentProjectEntry>,
    quiesce_deadline: Option<Instant>,
}

impl ProjectSwitchState {
    /// Create active state for the initial normalized project identity.
    pub fn new(identity: ProjectIdentity) -> Self {
        Self {
            active: ProjectStamp {
                identity,
                generation: ProjectGeneration::INITIAL,
            },
            phase: SwitchPhase::Active,
            overlay: SwitchOverlay::None,
            pending: None,
            recent: Vec::new(),
            quiesce_deadline: None,
        }
    }

    /// Return whether input and results belong to an active committed project.
    pub fn is_active(&self) -> bool {
        self.phase == SwitchPhase::Active && matches!(self.overlay, SwitchOverlay::None)
    }

    /// Compare a requested identity with the active project.
    pub fn same_project(&self, target: &ProjectIdentity) -> bool {
        self.active.identity == *target
    }

    /// Start a switch request without discarding the active context.
    pub fn begin(
        &mut self,
        target: ProjectIdentity,
        dirty: Vec<DirtyProjectObject>,
        busy: bool,
    ) -> SwitchDecision {
        if self.same_project(&target) {
            self.overlay = SwitchOverlay::None;
            self.pending = None;
            self.phase = SwitchPhase::Active;
            self.quiesce_deadline = None;
            return SwitchDecision::Noop;
        }
        self.phase = if dirty.is_empty() {
            SwitchPhase::QuiesceTasks
        } else {
            SwitchPhase::ResolveDirty
        };
        self.pending = Some(PendingProjectSwitch {
            target: target.clone(),
            dirty,
        });
        self.quiesce_deadline = None;
        self.overlay = if busy {
            SwitchOverlay::Busy { target }
        } else if self
            .pending
            .as_ref()
            .is_some_and(|request| !request.dirty.is_empty())
        {
            SwitchOverlay::Dirty { target }
        } else {
            SwitchOverlay::None
        };
        if matches!(self.overlay, SwitchOverlay::None) {
            SwitchDecision::Ready
        } else {
            SwitchDecision::NeedsConfirmation
        }
    }

    /// Open recent projects and clear any earlier uncommitted request.
    pub fn begin_picker(&mut self, recent: Vec<RecentProjectEntry>) {
        self.recent = recent;
        self.overlay = SwitchOverlay::Picker { selection: 0 };
        self.phase = SwitchPhase::ResolveTarget;
        self.pending = None;
        self.quiesce_deadline = None;
    }

    /// Replace the picker with a directory text input.
    pub fn begin_open_path(&mut self) {
        self.overlay = SwitchOverlay::OpenPath {
            input: String::new(),
        };
        self.phase = SwitchPhase::ResolveTarget;
        self.pending = None;
        self.quiesce_deadline = None;
    }

    /// Append one printable path character while the path field owns input.
    pub fn append_path_input(&mut self, ch: char) {
        if let SwitchOverlay::OpenPath { input } = &mut self.overlay {
            input.push(ch);
        }
    }

    /// Remove the final path character from the active directory field.
    pub fn backspace_path_input(&mut self) {
        if let SwitchOverlay::OpenPath { input } = &mut self.overlay {
            input.pop();
        }
    }

    /// Return the directory draft while the Open path field is visible.
    pub fn path_input(&self) -> Option<&str> {
        match &self.overlay {
            SwitchOverlay::OpenPath { input } => Some(input.as_str()),
            _ => None,
        }
    }

    /// Start the bounded wait for project-owned tasks to exit.
    pub fn start_quiesce_wait(&mut self, timeout: Duration) {
        self.quiesce_deadline = Some(Instant::now() + timeout);
    }

    /// Return whether the current task-exit acknowledgement deadline has expired.
    pub fn quiesce_timed_out(&self) -> bool {
        self.quiesce_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    /// Move recent-project selection while clamping to visible entries.
    pub fn move_picker(&mut self, delta: isize) {
        let SwitchOverlay::Picker { selection } = &mut self.overlay else {
            return;
        };
        if self.recent.is_empty() {
            *selection = 0;
            return;
        }
        *selection = if delta.is_negative() {
            selection.saturating_sub(delta.unsigned_abs())
        } else {
            selection
                .saturating_add(delta as usize)
                .min(self.recent.len() - 1)
        };
    }

    /// Resolve the highlighted recent entry to a project identity.
    pub fn picker_target(&self) -> Option<ProjectIdentity> {
        let SwitchOverlay::Picker { selection } = self.overlay else {
            return None;
        };
        self.recent
            .get(selection)
            .map(|entry| ProjectIdentity::from_root(&entry.root))
    }

    /// Cancel the request and preserve the current project and generation.
    pub fn cancel(&mut self) {
        self.phase = SwitchPhase::Active;
        self.overlay = SwitchOverlay::None;
        self.pending = None;
        self.quiesce_deadline = None;
    }

    /// Enter detached preparation after dirty-state and task resolution.
    pub fn mark_preparing(&mut self) {
        self.phase = SwitchPhase::Prepare;
        self.overlay = SwitchOverlay::None;
        self.quiesce_deadline = None;
    }

    /// Mark the final context replacement boundary.
    pub fn mark_committing(&mut self) {
        self.phase = SwitchPhase::Commit;
        self.overlay = SwitchOverlay::None;
        self.quiesce_deadline = None;
    }

    /// Activate a prepared context and advance its event generation exactly once.
    pub fn mark_committed(&mut self, identity: ProjectIdentity) {
        self.active = ProjectStamp {
            identity,
            generation: self.active.generation.next(),
        };
        self.phase = SwitchPhase::Active;
        self.overlay = SwitchOverlay::None;
        self.pending = None;
        self.quiesce_deadline = None;
    }

    /// Accept only results owned by the currently active project generation.
    pub fn accepts(&self, stamp: &ProjectStamp) -> bool {
        self.active == *stamp && self.phase == SwitchPhase::Active
    }

    /// Capture the active project identity and generation for asynchronous work.
    pub fn current_stamp(&self) -> ProjectStamp {
        self.active.clone()
    }
}

/// Result of starting a switch request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchDecision {
    Noop,
    NeedsConfirmation,
    Ready,
}

/// Validate a target before it reaches the switch state machine.
///
/// # Errors
/// Returns path canonicalization or current-directory resolution errors.
pub fn resolve_project_target(path: impl Into<PathBuf>) -> Result<ProjectIdentity> {
    ProjectIdentity::new(path.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use tempfile::tempdir;

    fn recent_entry(root: &Path) -> RecentProjectEntry {
        RecentProjectEntry {
            root: root.to_string_lossy().into_owned(),
            display_name: root
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            last_opened_ms: 0,
        }
    }

    #[test]
    fn same_project_is_a_noop_without_generation_change() {
        let root = tempdir().unwrap();
        let identity = ProjectIdentity::new(root.path()).unwrap();
        let mut state = ProjectSwitchState::new(identity.clone());
        let before = state.current_stamp();
        assert_eq!(
            state.begin(
                identity,
                vec![DirtyProjectObject::Requirement("doc-1".to_string())],
                true,
            ),
            SwitchDecision::Noop
        );
        assert_eq!(state.current_stamp(), before);
        assert!(state.is_active());
        assert_eq!(state.overlay, SwitchOverlay::None);
        assert!(state.pending.is_none());
    }

    #[test]
    fn dirty_switch_stays_before_commit_until_cancel_or_resolution() {
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        let mut state = ProjectSwitchState::new(ProjectIdentity::new(a.path()).unwrap());
        let target = ProjectIdentity::new(b.path()).unwrap();
        let before = state.current_stamp();
        let dirty = vec![
            DirtyProjectObject::Feature(a.path().join("draft.feature")),
            DirtyProjectObject::Requirement("doc-2".to_string()),
            DirtyProjectObject::TestPoint("tp-1".to_string()),
            DirtyProjectObject::AgentProposal,
        ];
        assert_eq!(
            state.begin(target.clone(), dirty.clone(), false),
            SwitchDecision::NeedsConfirmation
        );
        assert_eq!(state.phase, SwitchPhase::ResolveDirty);
        assert_eq!(state.pending, Some(PendingProjectSwitch { target, dirty }));
        assert!(matches!(state.overlay, SwitchOverlay::Dirty { .. }));
        state.cancel();
        assert!(state.is_active());
        assert_eq!(state.current_stamp(), before);
        assert!(state.pending.is_none());
    }

    #[test]
    fn commit_enters_prepare_then_increments_generation_once() {
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        let a_id = ProjectIdentity::new(a.path()).unwrap();
        let b_id = ProjectIdentity::new(b.path()).unwrap();
        let mut state = ProjectSwitchState::new(a_id);
        let old = state.current_stamp();
        assert_eq!(
            state.begin(b_id.clone(), Vec::new(), false),
            SwitchDecision::Ready
        );
        assert_eq!(state.phase, SwitchPhase::QuiesceTasks);
        assert!(state.pending.is_some());
        state.mark_preparing();
        assert_eq!(state.phase, SwitchPhase::Prepare);
        assert_eq!(state.current_stamp(), old);
        state.mark_committed(b_id.clone());
        assert_eq!(state.phase, SwitchPhase::Active);
        assert_eq!(state.overlay, SwitchOverlay::None);
        assert!(state.pending.is_none());
        assert_eq!(state.active.identity, b_id);
        assert_eq!(state.active.generation, ProjectGeneration::new(1));
        assert!(!state.accepts(&old));
        assert!(state.accepts(&state.current_stamp()));

        let b_stamp = state.current_stamp();
        assert_eq!(
            state.begin(old.identity.clone(), Vec::new(), false),
            SwitchDecision::Ready
        );
        state.mark_preparing();
        state.mark_committed(old.identity.clone());
        assert_eq!(state.active.identity, old.identity);
        assert_eq!(state.active.generation, ProjectGeneration::new(2));
        assert!(!state.accepts(&old));
        assert!(!state.accepts(&b_stamp));
        assert!(state.accepts(&state.current_stamp()));
    }

    #[test]
    fn stale_project_events_are_rejected_by_identity_generation_and_phase() {
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        let c = tempdir().unwrap();
        let a_id = ProjectIdentity::new(a.path()).unwrap();
        let b_id = ProjectIdentity::new(b.path()).unwrap();
        let c_id = ProjectIdentity::new(c.path()).unwrap();
        let mut state = ProjectSwitchState::new(a_id.clone());
        let old = state.current_stamp();
        let wrong_identity = ProjectStamp {
            identity: c_id,
            generation: old.generation,
        };
        let wrong_generation = ProjectStamp {
            identity: a_id,
            generation: old.generation.next(),
        };

        assert!(!state.accepts(&wrong_identity));
        assert!(!state.accepts(&wrong_generation));
        assert!(state.accepts(&old));

        assert_eq!(
            state.begin(b_id.clone(), Vec::new(), false),
            SwitchDecision::Ready
        );
        assert!(!state.accepts(&old));
        state.mark_preparing();
        assert!(!state.accepts(&state.current_stamp()));
        state.mark_committed(b_id.clone());

        let current = state.current_stamp();
        let stale_b_generation = ProjectStamp {
            identity: b_id,
            generation: old.generation,
        };
        assert!(!state.accepts(&old));
        assert!(!state.accepts(&stale_b_generation));
        assert!(state.accepts(&current));
    }

    #[test]
    fn busy_switch_requires_confirmation_and_cancel_restores_active_state() {
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        let mut state = ProjectSwitchState::new(ProjectIdentity::new(a.path()).unwrap());
        let target = ProjectIdentity::new(b.path()).unwrap();
        let before = state.current_stamp();

        assert_eq!(
            state.begin(target.clone(), Vec::new(), true),
            SwitchDecision::NeedsConfirmation
        );
        assert_eq!(state.phase, SwitchPhase::QuiesceTasks);
        assert_eq!(state.overlay, SwitchOverlay::Busy { target });
        assert!(!state.accepts(&before));

        state.cancel();
        assert!(state.is_active());
        assert_eq!(state.current_stamp(), before);
        assert!(state.accepts(&before));
    }

    #[test]
    fn picker_navigation_clamps_and_returns_the_selected_identity() {
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        let c = tempdir().unwrap();
        let mut state = ProjectSwitchState::new(ProjectIdentity::new(a.path()).unwrap());
        state.begin_picker(vec![
            recent_entry(a.path()),
            recent_entry(b.path()),
            recent_entry(c.path()),
        ]);

        assert_eq!(state.phase, SwitchPhase::ResolveTarget);
        assert_eq!(state.overlay, SwitchOverlay::Picker { selection: 0 });
        assert_eq!(
            state.picker_target(),
            Some(ProjectIdentity::from_root(a.path()))
        );

        state.move_picker(1);
        assert_eq!(state.overlay, SwitchOverlay::Picker { selection: 1 });
        assert_eq!(
            state.picker_target(),
            Some(ProjectIdentity::from_root(b.path()))
        );
        state.move_picker(isize::MAX);
        assert_eq!(state.overlay, SwitchOverlay::Picker { selection: 2 });
        state.move_picker(isize::MAX);
        assert_eq!(state.overlay, SwitchOverlay::Picker { selection: 2 });
        state.move_picker(isize::MIN);
        assert_eq!(state.overlay, SwitchOverlay::Picker { selection: 0 });
        state.move_picker(isize::MIN);
        assert_eq!(state.overlay, SwitchOverlay::Picker { selection: 0 });

        state.cancel();
        state.move_picker(1);
        assert_eq!(state.overlay, SwitchOverlay::None);
        assert!(state.picker_target().is_none());
    }

    #[test]
    fn picker_with_empty_history_keeps_an_invalid_selection_safe() {
        let root = tempdir().unwrap();
        let mut state = ProjectSwitchState::new(ProjectIdentity::new(root.path()).unwrap());
        state.begin_picker(Vec::new());
        state.move_picker(isize::MAX);
        state.move_picker(isize::MIN);
        assert_eq!(state.overlay, SwitchOverlay::Picker { selection: 0 });
        assert!(state.picker_target().is_none());

        state.overlay = SwitchOverlay::Picker { selection: 99 };
        assert!(state.picker_target().is_none());
        state.cancel();
        assert!(state.is_active());
    }

    #[test]
    fn open_path_input_and_quiesce_timeout_are_bounded_state_transitions() {
        let root = tempdir().unwrap();
        let mut state = ProjectSwitchState::new(ProjectIdentity::new(root.path()).unwrap());
        state.begin_open_path();
        state.append_path_input('C');
        state.append_path_input(':');
        state.append_path_input('\\');
        assert_eq!(state.path_input(), Some("C:\\"));
        state.backspace_path_input();
        assert_eq!(state.path_input(), Some("C:"));
        state.start_quiesce_wait(Duration::ZERO);
        assert!(state.quiesce_timed_out());
        state.cancel();
        assert!(state.is_active());
    }

    #[test]
    fn target_path_aliases_resolve_to_one_identity_and_same_project_is_a_noop() {
        let root = tempdir().unwrap();
        let nested = root.path().join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        let canonical = resolve_project_target(&nested).unwrap();
        let alias = resolve_project_target(nested.join(".").join("..").join("nested")).unwrap();
        let trailing = resolve_project_target(PathBuf::from(format!(
            "{}{}",
            nested.display(),
            std::path::MAIN_SEPARATOR
        )))
        .unwrap();

        assert_eq!(canonical, alias);
        assert_eq!(canonical, trailing);

        let mut state = ProjectSwitchState::new(canonical.clone());
        let before = state.current_stamp();
        assert_eq!(state.begin(alias, Vec::new(), false), SwitchDecision::Noop);
        assert_eq!(state.current_stamp(), before);
        assert_eq!(state.active.identity, canonical);
    }

    #[test]
    fn missing_absolute_target_is_held_pending_until_the_app_prepares_it() {
        let root = tempdir().unwrap();
        let missing = root.path().join("does-not-exist");
        let target = resolve_project_target(&missing).unwrap();
        assert_eq!(target.root(), missing);

        let active = ProjectIdentity::new(root.path()).unwrap();
        let mut state = ProjectSwitchState::new(active.clone());
        assert_eq!(
            state.begin(target.clone(), Vec::new(), false),
            SwitchDecision::Ready
        );
        assert_eq!(state.active.identity, active);
        assert_eq!(state.phase, SwitchPhase::QuiesceTasks);
        assert_eq!(
            state.pending.as_ref().map(|request| &request.target),
            Some(&target)
        );
    }
}
