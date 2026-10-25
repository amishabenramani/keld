//! GH473 native scheme ingress probe.
//!
//! This private, debug-only handler deliberately serves NO application resource
//! bytes. It establishes the `WebKit` callback/stop/constructor wiring behind
//! existing test hooks without minting a document, filesystem or IPC grant.
//! The real source + CSP + GH542 lease are separate implementation gates.

#![deny(unsafe_op_in_unsafe_fn)]

use std::cell::{Cell, RefCell};
use std::panic::AssertUnwindSafe;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSError, NSObject, NSObjectProtocol, ns_string};
use objc2_web_kit::{WKURLSchemeHandler, WKURLSchemeTask, WKWebView, WKWebViewConfiguration};

use super::macos_resource_intake::{
    IntakeLimits, IntakeReject, NativeRequestShape, TaskLedger, TaskTicket, check_shape,
};
use crate::error::WvError;

const TEST_LIMITS: IntakeLimits = IntakeLimits {
    // Diagnostic limits only. Production resource bounds require verified
    // source closure and observed peak-retention evidence.
    max_url_utf16_units: 2_048,
    max_header_fields: 32,
    max_header_utf16_units: 4_096,
    task_slots: 16,
    max_retained_bytes: 64 * 1024,
};

pub(super) struct SchemeIvars {
    task_ledger: RefCell<TaskLedger>,
    native_starts: Cell<u64>,
    native_stops: Cell<u64>,
}

define_class!(
    // SAFETY: NSObject's initialization invariant is fulfilled by the owned
    // ivars. MainThreadOnly matches WebKit's UI callback contract; the class
    // carries no worker pointer, file descriptor, or app authorization.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = SchemeIvars]
    pub(super) struct KeldNativeResourceProbe;

    // SAFETY: NSObject supplies this inherited Objective-C protocol.
    unsafe impl NSObjectProtocol for KeldNativeResourceProbe {}

    // SAFETY: selectors and native argument types are the exact public
    // WKURLSchemeHandler signature in objc2-web-kit 0.3.2.
    unsafe impl WKURLSchemeHandler for KeldNativeResourceProbe {
        #[unsafe(method(webView:startURLSchemeTask:))]
        fn start(&self, _view: &WKWebView, task: &ProtocolObject<dyn WKURLSchemeTask>) {
            self.native_start(task);
        }

        #[unsafe(method(webView:stopURLSchemeTask:))]
        fn stop(&self, _view: &WKWebView, task: &ProtocolObject<dyn WKURLSchemeTask>) {
            self.native_stop(task);
        }
    }
);

impl KeldNativeResourceProbe {
    /// Register against the selected verified per-window configuration BEFORE
    /// Wry constructs `WKWebView`. Never registered in shipping builds.
    #[allow(unsafe_code)] // GH473 WK config, objc2 alloc/init and NSException catch
    pub(super) fn install(config: &WKWebViewConfiguration) -> Result<Retained<Self>, WvError> {
        let mtm = MainThreadMarker::new().ok_or_else(|| {
            WvError::Webview(String::from("GH473 native scheme needs AppKit main thread"))
        })?;
        let instance = mtm.alloc::<Self>().set_ivars(SchemeIvars {
            task_ledger: RefCell::new(TaskLedger::new(TEST_LIMITS)),
            native_starts: Cell::new(0),
            native_stops: Cell::new(0),
        });
        // SAFETY: freshly allocated NSObject subclass with initialized ivars.
        let handler: Retained<Self> = unsafe { msg_send![super(instance), init] };
        let protocol = ProtocolObject::from_ref(&*handler);
        // SAFETY: one non-shipping Keld scheme is registered on a valid
        // main-thread configuration before the native WebView is constructed.
        // Catch an Objective-C exception (including accidental duplicate
        // registration) rather than continuing with an unguarded config.
        let installed = objc2::exception::catch(AssertUnwindSafe(|| unsafe {
            config.setURLSchemeHandler_forURLScheme(Some(protocol), ns_string!("keld-r13-test"));
        }));
        if installed.is_err() {
            return Err(WvError::Webview(String::from(
                "GH473 native diagnostic scheme registration rejected",
            )));
        }
        Ok(handler)
    }

    fn identity(task: &ProtocolObject<dyn WKURLSchemeTask>) -> usize {
        std::ptr::from_ref(task).cast::<()>() as usize
    }

    #[allow(unsafe_code)] // GH473 public task.request and didFailWithError only
    fn native_start(&self, task: &ProtocolObject<dyn WKURLSchemeTask>) {
        self.ivars()
            .native_starts
            .set(self.ivars().native_starts.get().saturating_add(1));

        let key = Self::identity(task);
        let start_result = { self.ivars().task_ledger.borrow_mut().begin(key) };
        let ticket = match start_result {
            Ok(ticket) => ticket,
            Err(IntakeReject::DuplicateNativeTask) => {
                // Do not call WebKit twice for the same active task object.
                return;
            }
            Err(_) => {
                // There is no owned work/slot to retire. Still fail this
                // newly presented WebKit task, never leave it hanging.
                Self::reject(task);
                return;
            }
        };
        // SAFETY: task is the live object WebKit passed to this callback;
        // no cross-thread object use and no retained task after callback.
        let request = unsafe { task.request() };
        let method_get = request
            .HTTPMethod()
            .as_ref()
            .is_some_and(|method| method.isEqualToString(ns_string!("GET")));
        // Invalid methods must not even access WebKit's possibly-materialized
        // HTTPBody/HTTPBodyStream or convert URL/header metadata. The method
        // getter itself is the only native input used before this rejection.
        if !method_get {
            self.finish_ticket(task, ticket);
            return;
        }
        // WebKit may materialize NSData while answering these getters. Check
        // presence before Keld-owned read/copy/stream operations; no body may
        // be admitted, and no positive resource response exists in this hook.
        let body_present = request.HTTPBody().is_some();
        let stream_present = request.HTTPBodyStream().is_some();
        if body_present || stream_present {
            self.finish_ticket(task, ticket);
            return;
        }
        let url_utf16_units = request
            .URL()
            .and_then(|url| url.absoluteString())
            .map_or(0, |text| text.length());
        let header_fields = request
            .allHTTPHeaderFields()
            .map_or(0, |fields| fields.count());
        // Do not let even diagnostic metadata retention exceed the scoped
        // budget. Checked arithmetic precedes all Keld-owned byte copies.
        let approximate_metadata_bytes = url_utf16_units.saturating_mul(2);
        let reserved = {
            self.ivars()
                .task_ledger
                .borrow_mut()
                .reserve_bytes(ticket, approximate_metadata_bytes)
        };
        if reserved.is_err() {
            self.finish_ticket(task, ticket);
            return;
        }
        let checked = check_shape(
            NativeRequestShape {
                is_get: method_get,
                has_body: body_present,
                has_body_stream: stream_present,
                url_utf16_units,
                header_fields,
                // This diagnostic never authorizes a positive response;
                // a full header-byte budget is not yet implemented.
                header_utf16_units: usize::MAX,
            },
            TEST_LIMITS,
        );
        // Full native per-header byte enumeration is not yet implemented:
        // header_utf16_units=MAX fails closed even for GET. The checked shape
        // must not be treated as a source, CSP, document or IPC permit.
        if checked.is_err() {
            self.finish_ticket(task, ticket);
            return;
        }
        // If future policy admits a shape, no verified source has yet been
        // attached; deny rather than granting resource access implicitly.
        self.finish_ticket(task, ticket);
    }

    fn finish_ticket(&self, task: &ProtocolObject<dyn WKURLSchemeTask>, ticket: TaskTicket) {
        let may_callback = {
            self.ivars()
                .task_ledger
                .borrow_mut()
                .begin_terminal_callback(ticket)
        };
        if may_callback {
            Self::reject(task);
        }
        let _ = self.ivars().task_ledger.borrow_mut().release(ticket);
    }

    #[allow(unsafe_code)] // Only a fixed terminal NSError to a live native task.
    fn reject(task: &ProtocolObject<dyn WKURLSchemeTask>) {
        // SAFETY: this is the active WebKit start callback on the UI thread.
        // There is no async work, and neither data nor response is delivered.
        let error = unsafe {
            NSError::errorWithDomain_code_userInfo(
                ns_string!("KELD-GH473-resource-not-admitted"),
                403,
                None,
            )
        };
        // SAFETY: caller either owns the terminal lease or has no recorded
        // task at all. stopURLSchemeTask does not call back through here.
        unsafe { task.didFailWithError(&error) };
    }

    fn native_stop(&self, task: &ProtocolObject<dyn WKURLSchemeTask>) {
        self.ivars()
            .native_stops
            .set(self.ivars().native_stops.get().saturating_add(1));
        // STOP never sends didFail/didReceive/didFinish: WebKit forbids any
        // callback after this point, including an error callback.
        self.ivars()
            .task_ledger
            .borrow_mut()
            .stop(Self::identity(task));
    }
}
