//! Refuse untrusted HTTP route data without echoing it into test diagnostics.
//! The first test uses a real owned TCP connection, not a mock stream.
use super::RecoveryObserver;
use std::net::{TcpListener, TcpStream};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc;

#[test]
fn native_unexpected_recovery_route_does_not_echo_private_request_path() {
    const PRIVATE: &str = "KELD_SYNTHETIC_PRIVATE_ROUTE_PATH_TOKEN_SENTINEL";
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("owned loopback listener");
    let client = TcpStream::connect(listener.local_addr().expect("local bound address"))
        .expect("real native recovery fixture client");
    let (accepted, _) = listener.accept().expect("accept native HTTP peer");

    let (_commands, inbox) = mpsc::channel();
    let mut observer = RecoveryObserver::new(inbox);
    let route = format!("/unexpected/{PRIVATE}?token={PRIVATE}");
    let rejected = catch_unwind(AssertUnwindSafe(|| observer.accept_wait(&route, accepted)));
    let payload = rejected.expect_err("unexpected route must still fail closed");
    let fixed = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&'static str>().copied());
    assert!(
        fixed.is_some_and(|diagnostic| diagnostic == "unexpected recovery fixture route"),
        "untrusted route must not be interpolated into diagnostic panic"
    );
    drop(client);
}

#[test]
fn public_observer_fallback_has_same_fixed_route_panic() {
    // Source-binding the other dispatch branch prevents a future raw route
    // interpolation from bypassing the native recovery-path regression.
    let source = include_str!("../server.rs");
    assert!(
        source.contains("None => panic!(\"unexpected public FS observation route\"),"),
        "observer's non-recovery dispatch must use a fixed route diagnostic"
    );
    assert!(
        !source.contains("unexpected public FS observation route {path}"),
        "untrusted public route must not be interpolated into panic"
    );
}
