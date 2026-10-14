//! Actionable notifications (PRD §5.7).
//!
//! Toolkit-free state so tests run headless: [`NotificationsCenter`] records
//! one [`Notification`] per event (approval needed, run finished, agent
//! question, error). Clicking a notification resolves to a [`NotificationTarget`]
//! (project + destination) that the GPUI shell opens; while the main window
//! is focused the same event is delivered as an in-app toast instead of an
//! OS notification (see [`delivery_for`]).

use crate::lifecycle::WindowVisibility;

/// What happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationKind {
    ApprovalNeeded,
    RunCompleted,
    AgentQuestion,
    RunError,
}

impl NotificationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ApprovalNeeded => "approval",
            Self::RunCompleted => "run_done",
            Self::AgentQuestion => "question",
            Self::RunError => "error",
        }
    }
}

/// Where a click (or toast tap) navigates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationDestination {
    /// Open the approvals inbox; `card_id` focuses one card when present.
    Approvals { card_id: Option<String> },
    /// Open the run / activity view for this run.
    Run { run_id: String },
    /// Open the team room scrolled to the agent's question message.
    Question { message_id: String },
    /// Open the run / activity view showing the error.
    ErrorDetail { run_id: Option<String> },
}

/// Click target: the project to open plus the destination inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationTarget {
    pub project_id: String,
    pub destination: NotificationDestination,
}

impl NotificationTarget {
    pub fn describe(&self) -> String {
        match &self.destination {
            NotificationDestination::Approvals { card_id } => match card_id {
                Some(c) => format!("project:{} approvals:{}", self.project_id, c),
                None => format!("project:{} approvals", self.project_id),
            },
            NotificationDestination::Run { run_id } => {
                format!("project:{} run:{run_id}", self.project_id)
            }
            NotificationDestination::Question { message_id } => {
                format!("project:{} message:{message_id}", self.project_id)
            }
            NotificationDestination::ErrorDetail { run_id } => match run_id {
                Some(r) => format!("project:{} error:{r}", self.project_id, r = r),
                None => format!("project:{} error", self.project_id),
            },
        }
    }
}

/// One notification row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub id: String,
    pub kind: NotificationKind,
    pub project_id: String,
    pub title: String,
    pub body: String,
    pub target: NotificationTarget,
    pub created_at: u64,
    pub read: bool,
}

impl Notification {
    pub fn is_unread(&self) -> bool {
        !self.read
    }
}

/// How the shell should surface a notification right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationDelivery {
    /// Window is focused: show an in-app toast, no OS notification.
    Toast(String),
    /// Window hidden/destroyed: show an OS notification.
    Os { title: String, body: String },
}

/// Delivery rule: focused window → in-app toast, otherwise OS notification.
pub fn delivery_for(
    notification: &Notification,
    visibility: WindowVisibility,
) -> NotificationDelivery {
    if visibility == WindowVisibility::Visible {
        NotificationDelivery::Toast(toast_text(notification))
    } else {
        NotificationDelivery::Os {
            title: notification.title.clone(),
            body: notification.body.clone(),
        }
    }
}

/// One-line toast text: "Title — body" (body truncated).
pub fn toast_text(notification: &Notification) -> String {
    let body = truncate(&notification.body, 140);
    if body.is_empty() {
        notification.title.clone()
    } else {
        format!("{} — {body}", notification.title)
    }
}

/// In-app toast: transient, tappable, expires after a few seconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    pub notification_id: String,
    pub text: String,
    pub target: NotificationTarget,
    pub expires_at: u64,
}

impl Toast {
    pub fn expired(&self, now: u64) -> bool {
        now >= self.expires_at
    }
}

pub const TOAST_TTL_SECS: u64 = 8;
pub const MAX_NOTIFICATIONS: usize = 200;

/// Actionable notification inbox + toast queue.
#[derive(Debug, Default)]
pub struct NotificationsCenter {
    items: Vec<Notification>,
    toasts: Vec<Toast>,
    next_id: u64,
    now: Option<u64>,
    pending_visibility: Option<WindowVisibility>,
}

impl NotificationsCenter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the clock (tests only); otherwise wall-clock time is used.
    pub fn set_now(&mut self, now: u64) {
        self.now = Some(now);
    }

    fn now_secs(&self) -> u64 {
        self.now.unwrap_or_else(now_secs)
    }

    fn alloc_id(&mut self) -> String {
        self.next_id += 1;
        format!("n_{}", self.next_id)
    }

    pub fn all(&self) -> &[Notification] {
        &self.items
    }

    pub fn unread_count(&self) -> usize {
        self.items.iter().filter(|n| n.is_unread()).count()
    }

    pub fn unread(&self) -> Vec<&Notification> {
        self.items.iter().filter(|n| n.is_unread()).collect()
    }

    pub fn toasts(&self) -> &[Toast] {
        &self.toasts
    }

    fn push(
        &mut self,
        kind: NotificationKind,
        project_id: &str,
        title: String,
        body: String,
        target: NotificationTarget,
    ) -> String {
        let id = self.alloc_id();
        let created_at = self.now_secs();
        self.items.push(Notification {
            id: id.clone(),
            kind,
            project_id: project_id.into(),
            title,
            body,
            target: target.clone(),
            created_at,
            read: false,
        });
        if self.items.len() > MAX_NOTIFICATIONS {
            let overflow = self.items.len() - MAX_NOTIFICATIONS;
            self.items.drain(..overflow);
        }
        id
    }

    /// Approval needed: opens the approvals inbox focused on `card_id`.
    pub fn notify_approval(
        &mut self,
        project_id: &str,
        card_id: &str,
        agent_id: &str,
        preview: &str,
    ) -> String {
        let title = "Approval needed".to_string();
        let body = format!("{agent_id} wants approval: {}", truncate(preview, 140));
        let target = NotificationTarget {
            project_id: project_id.into(),
            destination: NotificationDestination::Approvals {
                card_id: Some(card_id.into()),
            },
        };
        self.push(
            NotificationKind::ApprovalNeeded,
            project_id,
            title,
            body,
            target,
        )
    }

    /// Run finished: opens the run view.
    pub fn notify_run_completed(
        &mut self,
        project_id: &str,
        run_id: &str,
        run_title: &str,
    ) -> String {
        let title = "Run finished".to_string();
        let body = format!("{run_title} completed");
        let target = NotificationTarget {
            project_id: project_id.into(),
            destination: NotificationDestination::Run {
                run_id: run_id.into(),
            },
        };
        self.push(
            NotificationKind::RunCompleted,
            project_id,
            title,
            body,
            target,
        )
    }

    /// Agent asked the user a question: opens the message in the team room.
    pub fn notify_agent_question(
        &mut self,
        project_id: &str,
        agent_id: &str,
        message_id: &str,
        question: &str,
    ) -> String {
        let title = format!("{agent_id} has a question");
        let body = truncate(question, 200);
        let target = NotificationTarget {
            project_id: project_id.into(),
            destination: NotificationDestination::Question {
                message_id: message_id.into(),
            },
        };
        self.push(
            NotificationKind::AgentQuestion,
            project_id,
            title,
            body,
            target,
        )
    }

    /// Run/agent error: opens the error detail (run when known).
    pub fn notify_error(&mut self, project_id: &str, run_id: Option<&str>, error: &str) -> String {
        let title = "Something went wrong".to_string();
        let body = truncate(error, 200);
        let target = NotificationTarget {
            project_id: project_id.into(),
            destination: NotificationDestination::ErrorDetail {
                run_id: run_id.map(|s| s.to_string()),
            },
        };
        self.push(NotificationKind::RunError, project_id, title, body, target)
    }

    /// Emit + surface: stores the notification and, when the window is
    /// focused, queues an in-app toast instead of an OS notification.
    /// Returns the delivery so the shell knows which to show.
    pub fn emit(
        &mut self,
        kind: NotificationKind,
        project_id: &str,
        title: &str,
        body: &str,
        target: NotificationTarget,
    ) -> (String, NotificationDelivery) {
        let id = self.push(kind, project_id, title.into(), body.into(), target);
        let notification = self.items.iter().find(|n| n.id == id).expect("just pushed");
        // Focused window → toast (prune expired first so the queue stays short).
        let visibility = self.last_visibility();
        let delivery = delivery_for(notification, visibility);
        if let NotificationDelivery::Toast(ref text) = delivery {
            let expires_at = notification.created_at.saturating_add(TOAST_TTL_SECS);
            self.toasts.push(Toast {
                notification_id: id.clone(),
                text: text.clone(),
                target: notification.target.clone(),
                expires_at,
            });
        }
        (id, delivery)
    }

    fn last_visibility(&self) -> WindowVisibility {
        self.pending_visibility.unwrap_or(WindowVisibility::Hidden)
    }

    /// Visibility used by [`Self::emit`] (tests/headless set it; the GPUI
    /// shell passes the live visibility on every event). Defaults to hidden
    /// so background events surface as OS notifications.
    pub fn set_visibility(&mut self, visibility: WindowVisibility) {
        self.pending_visibility = Some(visibility);
    }

    /// Surface an existing notification for the current visibility without
    /// duplicating it (used when visibility changes or on re-render).
    pub fn surface(
        &mut self,
        id: &str,
        visibility: WindowVisibility,
    ) -> Option<NotificationDelivery> {
        let notification = self.items.iter().find(|n| n.id == id)?.clone();
        let delivery = delivery_for(&notification, visibility);
        if let NotificationDelivery::Toast(ref text) = delivery {
            if !self.toasts.iter().any(|t| t.notification_id == id) {
                self.toasts.push(Toast {
                    notification_id: id.into(),
                    text: text.clone(),
                    target: notification.target.clone(),
                    expires_at: notification.created_at.saturating_add(TOAST_TTL_SECS),
                });
            }
        }
        Some(delivery)
    }

    /// Click (or toast tap): marks read and returns where to navigate.
    /// Returns `None` for unknown ids.
    pub fn click(&mut self, id: &str) -> Option<NotificationTarget> {
        let notification = self.items.iter_mut().find(|n| n.id == id)?;
        notification.read = true;
        self.toasts.retain(|t| t.notification_id != id);
        Some(notification.target.clone())
    }

    pub fn mark_read(&mut self, id: &str) -> bool {
        if let Some(n) = self.items.iter_mut().find(|n| n.id == id) {
            n.read = true;
            return true;
        }
        false
    }

    pub fn mark_all_read(&mut self) {
        for n in &mut self.items {
            n.read = true;
        }
    }

    pub fn dismiss_toast(&mut self, notification_id: &str) -> bool {
        let before = self.toasts.len();
        self.toasts.retain(|t| t.notification_id != notification_id);
        before != self.toasts.len()
    }

    /// Drop expired toasts. Returns how many were removed.
    pub fn prune_toasts(&mut self) -> usize {
        let now = self.now_secs();
        let before = self.toasts.len();
        self.toasts.retain(|t| !t.expired(now));
        before - self.toasts.len()
    }

    /// Badge text for the inbox ("3" or "" when none unread).
    pub fn badge(&self) -> String {
        let n = self.unread_count();
        if n == 0 {
            String::new()
        } else {
            n.to_string()
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    format!("{}…", &s[..max])
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod notifications_tests {
    use super::*;

    fn center() -> NotificationsCenter {
        let mut c = NotificationsCenter::new();
        c.set_visibility(WindowVisibility::Hidden);
        c.set_now(1_000);
        c
    }

    #[test]
    fn notifications_approval_click_opens_card() {
        let mut c = center();
        let id = c.notify_approval("p1", "tc_1", "ada", "cargo test");
        assert_eq!(c.unread_count(), 1);
        let n = c.all().iter().find(|n| n.id == id).unwrap();
        assert_eq!(n.kind, NotificationKind::ApprovalNeeded);
        // Clicking opens the relevant project/action and marks read.
        let target = c.click(&id).expect("target");
        assert_eq!(target.project_id, "p1");
        assert_eq!(
            target.destination,
            NotificationDestination::Approvals {
                card_id: Some("tc_1".into())
            }
        );
        assert_eq!(target.describe(), "project:p1 approvals:tc_1");
        assert_eq!(c.unread_count(), 0);
        assert!(c.click("missing").is_none());
    }

    #[test]
    fn notifications_run_completed_question_and_error_targets() {
        let mut c = center();
        let done = c.notify_run_completed("p1", "r1", "Market study");
        let q = c.notify_agent_question("p1", "mira", "m9", "Which source should I trust?");
        let e = c.notify_error("p2", Some("r2"), "provider timeout");
        assert_eq!(c.unread_count(), 3);
        assert_eq!(
            c.click(&done).unwrap().destination,
            NotificationDestination::Run {
                run_id: "r1".into()
            }
        );
        assert_eq!(
            c.click(&q).unwrap().destination,
            NotificationDestination::Question {
                message_id: "m9".into()
            }
        );
        let target = c.click(&e).unwrap();
        assert_eq!(target.project_id, "p2");
        assert_eq!(
            target.destination,
            NotificationDestination::ErrorDetail {
                run_id: Some("r2".into())
            }
        );
        // Error without a run still opens the project.
        let e2 = c.notify_error("p3", None, "boom");
        assert_eq!(c.click(&e2).unwrap().describe(), "project:p3 error");
    }

    #[test]
    fn notifications_focused_window_shows_toast_not_os() {
        let mut c = center();
        let id = c.notify_run_completed("p1", "r1", "Market study");
        // Hidden window → OS notification, no toast.
        let hidden = c.surface(&id, WindowVisibility::Hidden).expect("delivery");
        assert!(matches!(hidden, NotificationDelivery::Os { .. }));
        assert!(c.toasts().is_empty());
        let destroyed = c
            .surface(&id, WindowVisibility::Destroyed)
            .expect("delivery");
        assert!(matches!(destroyed, NotificationDelivery::Os { .. }));
        // Focused window → in-app toast, tappable to the run.
        let toast = c.surface(&id, WindowVisibility::Visible).expect("delivery");
        let NotificationDelivery::Toast(text) = toast else {
            panic!("expected toast");
        };
        assert!(text.contains("Run finished"), "{text}");
        assert_eq!(c.toasts().len(), 1);
        assert_eq!(c.toasts()[0].notification_id, id);
        // Tapping the toast opens the run and clears the toast.
        let target = c.click(&id).expect("target");
        assert_eq!(target.describe(), "project:p1 run:r1");
        assert!(c.toasts().is_empty());
        assert!(c.surface("missing", WindowVisibility::Visible).is_none());
    }

    #[test]
    fn notifications_toasts_expire_and_badges_clear() {
        let mut c = center();
        c.set_visibility(WindowVisibility::Visible);
        let (id, delivery) = c.emit(
            NotificationKind::AgentQuestion,
            "p1",
            "mira has a question",
            "Which angle?",
            NotificationTarget {
                project_id: "p1".into(),
                destination: NotificationDestination::Question {
                    message_id: "m1".into(),
                },
            },
        );
        assert!(matches!(delivery, NotificationDelivery::Toast(_)));
        assert_eq!(c.toasts().len(), 1);
        assert_eq!(c.badge(), "1");
        // Still live before the TTL, expired after.
        c.set_now(1_000 + TOAST_TTL_SECS - 1);
        assert_eq!(c.prune_toasts(), 0);
        c.set_now(1_000 + TOAST_TTL_SECS + 1);
        assert_eq!(c.prune_toasts(), 1);
        assert!(c.toasts().is_empty());
        // Dismiss + mark-all-read clear the inbox badge.
        let (id2, _) = c.emit(
            NotificationKind::RunError,
            "p1",
            "Something went wrong",
            "boom",
            NotificationTarget {
                project_id: "p1".into(),
                destination: NotificationDestination::ErrorDetail { run_id: None },
            },
        );
        assert!(c.dismiss_toast(&id2));
        assert!(!c.dismiss_toast(&id2));
        assert_eq!(c.badge(), "2");
        c.mark_all_read();
        assert_eq!(c.badge(), "");
        assert_eq!(c.unread_count(), 0);
        assert!(c.mark_read(&id));
        assert!(!c.mark_read("missing"));
    }
}
