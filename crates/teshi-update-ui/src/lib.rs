//! Native GPUI adapter for the UI-independent application updater.

use gpui::{
    App, Context, InteractiveElement, IntoElement, MouseButton, ParentElement, Render, Styled,
    Window, div, px, rgb,
};
use std::{
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};
use teshi_core::version::{ReleaseChannel, build_identity};
use teshi_update::{
    UpdateEvent,
    github::{Clock, SystemClock},
    install::Installation,
    lifecycle::{self, DesktopHandoff, UNSAVED_INSTALL_MESSAGE, WAITING_FOR_EXIT_MESSAGE},
    manager::{self, CheckResult},
    policy::{self, UpdateSettings},
    storage::state_directory,
    transaction::{Restart, TransactionResult},
};

enum Event {
    Checked(Box<teshi_update::Result<CheckResult>>),
    Progress(UpdateEvent),
    Installed(teshi_update::Result<TransactionResult>),
}

/// Native update bar. Blocking network/filesystem work runs on worker threads.
pub struct UpdateView {
    state: PathBuf,
    settings: UpdateSettings,
    channel: Option<ReleaseChannel>,
    installation: Option<Installation>,
    status: String,
    report: Option<CheckResult>,
    busy: bool,
    cancel: Arc<AtomicBool>,
    sender: mpsc::Sender<Event>,
    receiver: mpsc::Receiver<Event>,
    can_exit: Rc<dyn Fn(&App) -> bool>,
}

impl UpdateView {
    /// Creates a view and schedules check-and-notify without installing anything.
    /// `can_exit` must reject closing when the host has unsaved work.
    pub fn new(can_exit: Rc<dyn Fn(&App) -> bool>, cx: &mut Context<Self>) -> Self {
        let state_result = state_directory();
        let state = state_result.as_ref().cloned().unwrap_or_default();
        let settings_result = state_result.and_then(|_| policy::load_settings(&state));
        let settings = settings_result.as_ref().cloned().unwrap_or(UpdateSettings {
            auto_check: false,
            channel: None,
        });
        let installation = std::env::current_exe()
            .ok()
            .and_then(|p| Installation::detect(&p, &build_identity()).ok());
        let status = settings_result
            .err()
            .map(|e| e.to_string())
            .or_else(|| {
                installation
                    .as_ref()
                    .and_then(|i| i.root.as_deref())
                    .and_then(teshi_update::transaction::last_result)
                    .map(|r| r.detail)
            })
            .unwrap_or_else(|| "Updates".into());
        let (sender, receiver) = mpsc::channel();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                if this.update(cx, |view, cx| view.tick(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            state,
            settings,
            channel: None,
            installation,
            status,
            report: None,
            busy: false,
            cancel: Arc::new(AtomicBool::new(false)),
            sender,
            receiver,
            can_exit,
        }
    }

    fn check(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.report = None;
        self.status = "Checking for updates…".into();
        let sender = self.sender.clone();
        let channel = self.channel;
        std::thread::spawn(move || {
            let result = std::env::current_exe()
                .map_err(teshi_update::UpdateError::from)
                .and_then(|p| manager::check(&p, build_identity(), channel));
            let _ = sender.send(Event::Checked(Box::new(result)));
        });
        cx.notify();
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        while let Ok(event) = self.receiver.try_recv() {
            match event {
                Event::Checked(result) => {
                    self.busy = false;
                    match *result {
                        Ok(report) => {
                            self.status = report
                                .candidate
                                .as_ref()
                                .map(|c| format!("{} available", c.manifest.tag))
                                .unwrap_or_else(|| "Teshi is up to date".into());
                            if let Some(reason) = &report.installation.explanation {
                                self.status.push_str(&format!(" — {reason}"));
                            }
                            self.report = Some(report);
                        }
                        Err(error) => self.status = error.to_string(),
                    }
                }
                Event::Progress(event) => {
                    self.status = format!(
                        "{:?}{}",
                        event.status,
                        event
                            .progress
                            .map(|p| format!(" {}%", (p * 100.0) as u32))
                            .unwrap_or_default()
                    );
                }
                Event::Installed(result) => {
                    self.busy = false;
                    match result {
                        Ok(result) => {
                            match lifecycle::desktop_handoff(&result, (self.can_exit)(cx)) {
                                DesktopHandoff::QuitForHelper => {
                                    self.status = result.detail;
                                    cx.quit();
                                }
                                DesktopHandoff::KeepOpen => {
                                    self.status = WAITING_FOR_EXIT_MESSAGE.into();
                                }
                                DesktopHandoff::Installed | DesktopHandoff::Report => {
                                    self.status = result.detail;
                                }
                            }
                        }
                        Err(error) => self.status = error.to_string(),
                    }
                }
            }
            cx.notify();
        }
        if !self.busy
            && let Some(installation) = &self.installation
            && policy::claim_check(&self.state, installation, &self.settings, SystemClock.now())
                .unwrap_or(false)
        {
            self.check(cx);
        }
    }

    fn install(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if !lifecycle::may_start_install(!(self.can_exit)(cx)) {
            self.status = UNSAVED_INSTALL_MESSAGE.into();
            cx.notify();
            return;
        }
        let Some(report) = self.report.clone() else {
            return;
        };
        self.busy = true;
        self.cancel.store(false, Ordering::Relaxed);
        let cancel = self.cancel.clone();
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let result = std::env::current_dir()
                .map_err(teshi_update::UpdateError::from)
                .and_then(|working_directory| {
                    let restart = Restart {
                        arguments: std::env::args().skip(1).collect(),
                        working_directory,
                    };
                    manager::install_with_restart(
                        &report,
                        &cancel,
                        &mut |event| {
                            let _ = sender.send(Event::Progress(event));
                        },
                        Some(restart),
                    )
                });
            let _ = sender.send(Event::Installed(result));
        });
        cx.notify();
    }

    fn toggle_auto(&mut self, cx: &mut Context<Self>) {
        let mut settings = self.settings.clone();
        settings.auto_check = !settings.auto_check;
        match policy::save_settings(&self.state, &settings) {
            Ok(()) => self.settings = settings,
            Err(error) => self.status = error.to_string(),
        }
        cx.notify();
    }
}

impl Render for UpdateView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut controls = div()
            .flex()
            .gap(px(12.))
            .text_sm()
            .child(
                div()
                    .id("update-check")
                    .cursor_pointer()
                    .child("Check for Updates")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.check(cx)),
                    ),
            )
            .child(
                div()
                    .id("update-auto")
                    .cursor_pointer()
                    .child(if self.settings.auto_check {
                        "Automatic checks: On"
                    } else {
                        "Automatic checks: Off"
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.toggle_auto(cx)),
                    ),
            )
            .child(
                div()
                    .id("update-channel")
                    .cursor_pointer()
                    .child(format!(
                        "Channel: {:?}",
                        self.channel.unwrap_or(build_identity().channel)
                    ))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            if !this.busy {
                                this.channel = Some(
                                    if this.channel.unwrap_or(build_identity().channel)
                                        == ReleaseChannel::Nightly
                                    {
                                        ReleaseChannel::Stable
                                    } else {
                                        ReleaseChannel::Nightly
                                    },
                                );
                                this.report = None;
                                this.status =
                                    "Channel selected; check for updates before installing.".into();
                                cx.notify();
                            }
                        }),
                    ),
            );
        if let Some(candidate) = self.report.as_ref().and_then(|r| r.candidate.as_ref()) {
            let url = candidate.release_url.clone();
            controls = controls.child(
                div()
                    .id("update-notes")
                    .cursor_pointer()
                    .child("Release notes")
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| cx.open_url(&url)),
            );
        }
        if !self.busy
            && self
                .report
                .as_ref()
                .is_some_and(|r| r.candidate.is_some() && r.installation.can_install())
        {
            controls = controls.child(
                div()
                    .id("update-install")
                    .cursor_pointer()
                    .child("Download and Install")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.install(cx)),
                    ),
            );
        }
        if self.busy {
            controls = controls.child(
                div()
                    .id("update-cancel")
                    .cursor_pointer()
                    .child("Cancel")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.cancel.store(true, Ordering::Relaxed);
                            cx.notify();
                        }),
                    ),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .p(px(10.))
            .bg(rgb(0x181825))
            .text_color(rgb(0xcdd6f4))
            .child(self.status.clone())
            .child(controls)
    }
}

impl Drop for UpdateView {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
