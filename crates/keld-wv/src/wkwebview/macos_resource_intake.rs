//! GH473: bounded, authority-free accounting for a native `WebKit` resource task.
//! This module grants no `WebView` filesystem, manifest, CSP, or Bun authority.
//! Future native callbacks must reject methods and body presence before copying
//! native `NSString` values or opening an `HTTPBodyStream`.

use std::collections::BTreeMap;

/// Limits come from verified Host policy, never a page, Wry, or a global fallback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct IntakeLimits {
    pub max_url_utf16_units: usize,
    pub max_header_fields: usize,
    pub max_header_utf16_units: usize,
    pub task_slots: usize,
    pub max_retained_bytes: usize,
}

/// Metadata observed without turning the native request into an `http::Request`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct NativeRequestShape {
    pub is_get: bool,
    pub has_body: bool,
    pub has_body_stream: bool,
    pub url_utf16_units: usize,
    pub header_fields: usize,
    pub header_utf16_units: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum IntakeReject {
    WrongMethod,
    BodyOrStream,
    MissingUrl,
    MetadataLimit,
    NoTaskCapacity,
    RetainedByteLimit,
    DuplicateNativeTask,
    TaskIncarnationExhausted,
    StoppedOrFinished,
    WorkNotQuiescent,
}

/// Shape-only admission. Not permission to open or deliver source bytes.
pub(super) fn check_shape(
    shape: NativeRequestShape,
    limits: IntakeLimits,
) -> Result<(), IntakeReject> {
    // Order matters: an invalid method fails before header/body conversions.
    if !shape.is_get {
        return Err(IntakeReject::WrongMethod);
    }
    if shape.has_body || shape.has_body_stream {
        return Err(IntakeReject::BodyOrStream);
    }
    if shape.url_utf16_units == 0 {
        return Err(IntakeReject::MissingUrl);
    }
    if shape.url_utf16_units > limits.max_url_utf16_units
        || shape.header_fields > limits.max_header_fields
        || shape.header_utf16_units > limits.max_header_utf16_units
    {
        return Err(IntakeReject::MetadataLimit);
    }
    Ok(())
}

/// Task incarnation is NOT GH542's document generation. It prevents a native
/// task-pointer reuse from resurrecting cancellation or retained accounting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TaskTicket {
    native_identity: usize,
    incarnation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Live,
    Stopped,
    Terminal,
}

#[derive(Clone, Copy, Debug)]
struct Task {
    incarnation: u64,
    phase: Phase,
    retained_bytes: usize,
    worker_active: bool,
}

/// UI-executor-only ledger. Native task stop tombstones first; work and memory
/// reservations remain charged until the true worker/buffer owner is quiescent.
/// No Objective-C reference or filesystem handle is captured in this state.
#[derive(Debug)]
pub(super) struct TaskLedger {
    limits: IntakeLimits,
    next_incarnation: u64,
    tasks: BTreeMap<usize, Task>,
    retained_bytes: usize,
}

impl TaskLedger {
    pub(super) fn new(limits: IntakeLimits) -> Self {
        Self {
            limits,
            next_incarnation: 0,
            tasks: BTreeMap::new(),
            retained_bytes: 0,
        }
    }

    pub(super) fn begin(&mut self, native_identity: usize) -> Result<TaskTicket, IntakeReject> {
        if self.tasks.contains_key(&native_identity) {
            return Err(IntakeReject::DuplicateNativeTask);
        }
        if self.tasks.len() >= self.limits.task_slots {
            return Err(IntakeReject::NoTaskCapacity);
        }
        let incarnation = self
            .next_incarnation
            .checked_add(1)
            .ok_or(IntakeReject::TaskIncarnationExhausted)?;
        self.next_incarnation = incarnation;
        self.tasks.insert(
            native_identity,
            Task {
                incarnation,
                phase: Phase::Live,
                retained_bytes: 0,
                worker_active: false,
            },
        );
        Ok(TaskTicket {
            native_identity,
            incarnation,
        })
    }

    fn current(&self, ticket: TaskTicket) -> Result<&Task, IntakeReject> {
        self.tasks
            .get(&ticket.native_identity)
            .filter(|task| task.incarnation == ticket.incarnation)
            .ok_or(IntakeReject::StoppedOrFinished)
    }

    fn current_mut(&mut self, ticket: TaskTicket) -> Result<&mut Task, IntakeReject> {
        self.tasks
            .get_mut(&ticket.native_identity)
            .filter(|task| task.incarnation == ticket.incarnation)
            .ok_or(IntakeReject::StoppedOrFinished)
    }

    pub(super) fn reserve_bytes(
        &mut self,
        ticket: TaskTicket,
        additional: usize,
    ) -> Result<(), IntakeReject> {
        if self.current(ticket)?.phase != Phase::Live {
            return Err(IntakeReject::StoppedOrFinished);
        }
        let next = self
            .retained_bytes
            .checked_add(additional)
            .ok_or(IntakeReject::RetainedByteLimit)?;
        if next > self.limits.max_retained_bytes {
            return Err(IntakeReject::RetainedByteLimit);
        }
        let entry = self.current_mut(ticket)?;
        entry.retained_bytes = entry
            .retained_bytes
            .checked_add(additional)
            .ok_or(IntakeReject::RetainedByteLimit)?;
        self.retained_bytes = next;
        Ok(())
    }

    // There is no async producer in the diagnostic path. Exercise actual
    // worker quiescence semantics only in the scoped ledger tests until the
    // GH473 source and GH542 document owners supply a permitted worker.
    #[cfg(test)]
    pub(super) fn worker_started(&mut self, ticket: TaskTicket) -> Result<(), IntakeReject> {
        let entry = self.current_mut(ticket)?;
        if entry.phase != Phase::Live {
            return Err(IntakeReject::StoppedOrFinished);
        }
        entry.worker_active = true;
        Ok(())
    }

    /// The native stopURLSchemeTask callback tombstones the associated task.
    pub(super) fn stop(&mut self, native_identity: usize) {
        if let Some(entry) = self.tasks.get_mut(&native_identity) {
            entry.phase = Phase::Stopped;
        }
    }

    /// Take the terminal callback lease before reentering Objective-C. Do not
    /// retain a `RefCell` borrow or Rust lock while invoking a `WebKit` callback.
    pub(super) fn begin_terminal_callback(&mut self, ticket: TaskTicket) -> bool {
        let Ok(entry) = self.current_mut(ticket) else {
            return false;
        };
        if entry.phase != Phase::Live {
            return false;
        }
        entry.phase = Phase::Terminal;
        true
    }

    #[cfg(test)]
    pub(super) fn worker_quiesced(&mut self, ticket: TaskTicket) -> Result<(), IntakeReject> {
        self.current_mut(ticket)?.worker_active = false;
        Ok(())
    }

    /// Release a terminal/stopped incarnation only after work is quiescent.
    pub(super) fn release(&mut self, ticket: TaskTicket) -> Result<(), IntakeReject> {
        let entry = self.current(ticket)?;
        if entry.phase == Phase::Live || entry.worker_active {
            return Err(IntakeReject::WorkNotQuiescent);
        }
        let charged = entry.retained_bytes;
        self.retained_bytes -= charged;
        self.tasks.remove(&ticket.native_identity);
        Ok(())
    }

    #[cfg(test)]
    fn live_count(&self) -> usize {
        self.tasks.len()
    }

    #[cfg(test)]
    fn retained(&self) -> usize {
        self.retained_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> IntakeLimits {
        // Fixture inputs only, not production-approved limits.
        IntakeLimits {
            max_url_utf16_units: 32,
            max_header_fields: 2,
            max_header_utf16_units: 12,
            task_slots: 2,
            max_retained_bytes: 100,
        }
    }

    fn get() -> NativeRequestShape {
        NativeRequestShape {
            is_get: true,
            has_body: false,
            has_body_stream: false,
            url_utf16_units: 8,
            header_fields: 1,
            header_utf16_units: 6,
        }
    }

    #[test]
    fn rejects_post_before_header_and_body_accounting() {
        let mut req = get();
        req.is_get = false;
        req.has_body = true;
        req.has_body_stream = true;
        req.url_utf16_units = usize::MAX;
        assert_eq!(check_shape(req, limits()), Err(IntakeReject::WrongMethod));
        req.is_get = true;
        assert_eq!(check_shape(req, limits()), Err(IntakeReject::BodyOrStream));
        req.has_body = false;
        assert_eq!(check_shape(req, limits()), Err(IntakeReject::BodyOrStream));
    }

    #[test]
    fn detects_exact_metadata_max_plus_one() {
        assert_eq!(check_shape(get(), limits()), Ok(()));
        for (mut req, code) in [
            (
                NativeRequestShape {
                    url_utf16_units: 0,
                    ..get()
                },
                IntakeReject::MissingUrl,
            ),
            (
                NativeRequestShape {
                    url_utf16_units: 33,
                    ..get()
                },
                IntakeReject::MetadataLimit,
            ),
            (
                NativeRequestShape {
                    header_fields: 3,
                    ..get()
                },
                IntakeReject::MetadataLimit,
            ),
            (
                NativeRequestShape {
                    header_utf16_units: 13,
                    ..get()
                },
                IntakeReject::MetadataLimit,
            ),
        ] {
            assert_eq!(check_shape(req, limits()), Err(code));
            req.url_utf16_units = 32;
            req.header_fields = 2;
            req.header_utf16_units = 12;
            assert_eq!(check_shape(req, limits()), Ok(()));
        }
    }

    #[test]
    fn stopped_work_keeps_reservation_charged_until_quiescent() {
        let mut ledger = TaskLedger::new(limits());
        let a = ledger.begin(1).unwrap();
        ledger.reserve_bytes(a, 90).unwrap();
        ledger.worker_started(a).unwrap();
        ledger.stop(1);
        assert!(!ledger.begin_terminal_callback(a));
        assert_eq!(
            ledger.reserve_bytes(a, 1),
            Err(IntakeReject::StoppedOrFinished)
        );
        assert_eq!(ledger.release(a), Err(IntakeReject::WorkNotQuiescent));
        assert_eq!(ledger.retained(), 90);
        let b = ledger.begin(2).unwrap();
        assert_eq!(
            ledger.reserve_bytes(b, 11),
            Err(IntakeReject::RetainedByteLimit)
        );
        ledger.worker_quiesced(a).unwrap();
        ledger.release(a).unwrap();
        assert_eq!(ledger.retained(), 0);
        ledger.reserve_bytes(b, 11).unwrap();
        assert!(!ledger.begin_terminal_callback(a));
        assert!(ledger.begin_terminal_callback(b));
        ledger.release(b).unwrap();
        assert_eq!(ledger.release(b), Err(IntakeReject::StoppedOrFinished));
    }

    #[test]
    fn pointer_reuse_and_incarnation_cannot_resurrect_old_task() {
        let mut ledger = TaskLedger::new(limits());
        let old = ledger.begin(7).unwrap();
        assert_eq!(ledger.begin(7), Err(IntakeReject::DuplicateNativeTask));
        assert!(ledger.begin_terminal_callback(old));
        assert!(!ledger.begin_terminal_callback(old));
        ledger.release(old).unwrap();
        let new = ledger.begin(7).unwrap();
        assert_ne!(new, old);
        ledger.stop(7);
        assert!(!ledger.begin_terminal_callback(new));
        assert!(!ledger.begin_terminal_callback(old));
        ledger.release(new).unwrap();
        assert_eq!(ledger.live_count(), 0);
    }

    #[test]
    fn active_work_cannot_release_or_overbook_slot_and_bytes() {
        let mut ledger = TaskLedger::new(limits());
        let a = ledger.begin(9).unwrap();
        let b = ledger.begin(10).unwrap();
        assert_eq!(ledger.begin(11), Err(IntakeReject::NoTaskCapacity));
        assert_eq!(ledger.release(a), Err(IntakeReject::WorkNotQuiescent));
        ledger.reserve_bytes(a, 100).unwrap();
        assert_eq!(
            ledger.reserve_bytes(b, 1),
            Err(IntakeReject::RetainedByteLimit)
        );
        assert!(ledger.begin_terminal_callback(a));
        ledger.release(a).unwrap();
        ledger.reserve_bytes(b, 100).unwrap();
        assert!(ledger.begin_terminal_callback(b));
        ledger.release(b).unwrap();
    }

    #[test]
    fn checked_arithmetic_refuses_overflow() {
        let mut ledger = TaskLedger::new(IntakeLimits {
            max_retained_bytes: usize::MAX,
            ..limits()
        });
        let ticket = ledger.begin(1).unwrap();
        ledger.reserve_bytes(ticket, usize::MAX).unwrap();
        assert_eq!(
            ledger.reserve_bytes(ticket, 1),
            Err(IntakeReject::RetainedByteLimit)
        );
        assert!(ledger.begin_terminal_callback(ticket));
        ledger.release(ticket).unwrap();
        assert_eq!(ledger.retained(), 0);

        ledger.next_incarnation = u64::MAX;
        assert_eq!(ledger.begin(2), Err(IntakeReject::TaskIncarnationExhausted));
    }
}
