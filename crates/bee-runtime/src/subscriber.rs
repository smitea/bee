//! S17 §5: StreamSubscriber state machine.
//!
//! Watches the ControlPlane for the upstream Producer's lifecycle
//! and re-establishes the BRP subscription when the Producer comes
//! back. The state machine is pure: given (current state, upstream
//! lifecycle, upstream presence), it produces the next state + an
//! action the runtime should take. The watcher / BRP wire is
//! layered on top; for S17 the wire is mocked and left as a
//! follow-up issue.

use crate::JobLifecycleState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriberState {
    Connecting,
    Active,
    WaitingForUpstream,
    Resubscribing,
}

pub enum SubscriberAction {
    None,
    OpenSubscription,
    CloseSubscription,
    ReopenSubscriptionFrom { from_offset: u64 },
}

pub struct SubscriberTick {
    pub next: SubscriberState,
    pub action: SubscriberAction,
}

pub struct StreamSubscriber {
    pub upstream_job: u32,
    pub stream_sig: String,
    pub last_consumed_offset: u64,
    pub state: SubscriberState,
}

impl StreamSubscriber {
    pub fn new(upstream_job: u32, stream_sig: String) -> Self {
        Self {
            upstream_job,
            stream_sig,
            last_consumed_offset: 0,
            state: SubscriberState::Connecting,
        }
    }

    /// Drive one state-machine tick. Pure function — no I/O.
    pub fn tick(
        &mut self,
        upstream_lifecycle: JobLifecycleState,
        upstream_present: bool,
    ) -> SubscriberTick {
        use SubscriberAction::*;
        use SubscriberState::*;

        let (next, action) = match (
            self.state,
            upstream_present,
            upstream_lifecycle,
        ) {
            (Connecting, true, JobLifecycleState::Running) => (Active, OpenSubscription),
            (Connecting, _, _) => (Connecting, None),

            (Active, true, JobLifecycleState::Running) => (Active, None),
            (
                Active,
                true,
                JobLifecycleState::Pending
                | JobLifecycleState::Scheduled
                | JobLifecycleState::WaitingForUpstream,
            ) => (Active, None),
            (Active, true, JobLifecycleState::Failed | JobLifecycleState::Completed) => {
                (WaitingForUpstream, CloseSubscription)
            }
            (Active, false, _) => (WaitingForUpstream, CloseSubscription),

            (WaitingForUpstream, true, JobLifecycleState::Running) => (
                Resubscribing,
                ReopenSubscriptionFrom {
                    from_offset: self.last_consumed_offset,
                },
            ),
            (WaitingForUpstream, _, _) => (WaitingForUpstream, None),

            (Resubscribing, true, JobLifecycleState::Running) => (Active, None),
            (Resubscribing, _, _) => (WaitingForUpstream, CloseSubscription),
        };

        self.state = next;
        SubscriberTick { next, action }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::JobLifecycleState::*;

    fn sub() -> StreamSubscriber {
        StreamSubscriber::new(1, "sig".into())
    }

    #[test]
    fn connecting_to_active_when_upstream_running() {
        let mut s = sub();
        let t = s.tick(Running, true);
        assert_eq!(t.next, SubscriberState::Active);
        assert!(matches!(t.action, SubscriberAction::OpenSubscription));
    }

    #[test]
    fn connecting_stays_connecting_when_upstream_not_running() {
        let mut s = sub();
        let t = s.tick(Scheduled, true);
        assert_eq!(t.next, SubscriberState::Connecting);
        assert!(matches!(t.action, SubscriberAction::None));
    }

    #[test]
    fn active_to_waiting_when_upstream_fails() {
        let mut s = sub();
        s.tick(Running, true); // -> Active
        let t = s.tick(Failed, true);
        assert_eq!(t.next, SubscriberState::WaitingForUpstream);
        assert!(matches!(t.action, SubscriberAction::CloseSubscription));
    }

    #[test]
    fn active_to_waiting_when_upstream_disappears() {
        let mut s = sub();
        s.tick(Running, true); // -> Active
        let t = s.tick(Running, false);
        assert_eq!(t.next, SubscriberState::WaitingForUpstream);
        assert!(matches!(t.action, SubscriberAction::CloseSubscription));
    }

    #[test]
    fn waiting_to_resubscribing_when_upstream_running_again() {
        let mut s = sub();
        s.tick(Running, true);
        s.tick(Failed, true);
        let t = s.tick(Running, true);
        assert_eq!(t.next, SubscriberState::Resubscribing);
        assert!(matches!(t.action, SubscriberAction::ReopenSubscriptionFrom { .. }));
    }

    #[test]
    fn resubscribing_to_active_after_reopen() {
        let mut s = sub();
        s.last_consumed_offset = 42;
        s.tick(Running, true);
        s.tick(Failed, true);
        s.tick(Running, true); // -> Resubscribing
        let t = s.tick(Running, true); // re-subscribe complete
        assert_eq!(t.next, SubscriberState::Active);
        s.last_consumed_offset = 100;
        let t = s.tick(Failed, true); // upstream dies mid-resub
        assert_eq!(t.next, SubscriberState::WaitingForUpstream);
    }

    #[test]
    fn waiting_stays_waiting_while_upstream_absent() {
        let mut s = sub();
        s.tick(Running, true);
        s.tick(Failed, true);
        let t = s.tick(Failed, false);
        assert_eq!(t.next, SubscriberState::WaitingForUpstream);
        assert!(matches!(t.action, SubscriberAction::None));
    }

    #[test]
    fn active_stays_active_on_lifecycle_pause() {
        let mut s = sub();
        s.tick(Running, true);
        let t = s.tick(WaitingForUpstream, true);
        assert_eq!(t.next, SubscriberState::Active,
            "a transient upstream pause must not sever the subscription");
    }
}
