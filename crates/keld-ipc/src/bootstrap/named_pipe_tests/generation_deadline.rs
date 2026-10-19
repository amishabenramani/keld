//! Generation deadline lifecycle scenarios for Windows named pipes.
//! Native observers verify no late authorization or reaccept after expiry.

use super::{
    AppLinkDeadlines, Arc, BlockingObserver, BootstrapRejection, Duration, Instant, Mutex,
    TestConsumeGate, WindowsNamedPipeBootstrapAdmission, WindowsNamedPipeBootstrapListener,
    WindowsNamedPipeServer, client, handshake_client, parse_app_link,
};
use std::io::Write as _;
use std::sync::mpsc;
use std::thread;

#[test]
fn generation_deadline_is_terminal_without_a_client() {
    let listener = WindowsNamedPipeBootstrapListener::bind().expect("bind");
    let outcome = listener
        .accept_authenticated_until(
            Instant::now() + Duration::from_millis(30),
            &super::NoopRejectionObserver,
        )
        .expect("deadline admission");
    assert!(matches!(
        outcome,
        WindowsNamedPipeBootstrapAdmission::DeadlineElapsed
    ));
    let error = WindowsNamedPipeServer::connect_client(listener.endpoint())
        .expect_err("terminal generation must not accept a late connector");
    assert!(matches!(error.raw_os_error(), Some(2 | 231)));
}

#[test]
fn already_expired_generation_never_enters_pipe_accept() {
    let listener = WindowsNamedPipeBootstrapListener::bind().expect("bind");
    let deadline = Instant::now()
        .checked_sub(Duration::from_millis(1))
        .expect("representable expired deadline");
    let outcome = listener
        .accept_authenticated_until(deadline, &super::NoopRejectionObserver)
        .expect("expired admission");
    assert!(matches!(
        outcome,
        WindowsNamedPipeBootstrapAdmission::DeadlineElapsed
    ));
    let stale = WindowsNamedPipeServer::connect_client(listener.endpoint())
        .expect_err("expired generation must close before accepting");
    assert_eq!(stale.raw_os_error(), Some(2));
}

#[test]
fn generation_expiry_during_rejection_blocks_reaccept() {
    let listener = Arc::new(WindowsNamedPipeBootstrapListener::bind().expect("bind"));
    let endpoint = listener.endpoint().to_owned();
    let cancellation = listener.cancellation();
    let deadline = Instant::now() + Duration::from_millis(100);
    let (observed_tx, observed_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let observer = BlockingObserver {
        observed: observed_tx,
        release: Mutex::new(release_rx),
    };
    let worker_listener = Arc::clone(&listener);
    let (result_tx, result_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        let _ = result_tx.send(worker_listener.accept_authenticated_until(deadline, &observer));
    });

    let mut hostile = client(&endpoint).expect("open hostile client");
    hostile.write_all(&[0_u8; 16]).expect("bad header");
    assert_eq!(
        observed_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("header rejection"),
        BootstrapRejection::Header
    );
    while Instant::now() < deadline {
        thread::yield_now();
    }
    release_tx.send(()).expect("release observer after expiry");

    let outcome = match result_rx.recv_timeout(Duration::from_secs(1)) {
        Ok(result) => result.expect("admission result"),
        Err(error) => {
            cancellation.cancel().expect("cancel wedged reaccept");
            worker.join().expect("join cancelled reaccept");
            panic!("expired rejection path re-entered an unbounded accept: {error}");
        }
    };
    assert!(matches!(
        outcome,
        WindowsNamedPipeBootstrapAdmission::DeadlineElapsed
    ));
    drop(hostile);
    worker.join().expect("join expired rejection worker");
}

#[test]
fn generation_expiry_at_final_auth_boundary_is_not_consumed() {
    let listener = Arc::new(WindowsNamedPipeBootstrapListener::bind().expect("bind"));
    let link = listener.app_link();
    let (endpoint, token) = parse_app_link(&link).expect("parse link");
    let endpoint = endpoint.to_owned();
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::channel();
    listener.install_before_consume_gate(TestConsumeGate {
        entered: entered_tx,
        release: release_rx,
    });
    let deadline = Instant::now() + Duration::from_millis(100);
    let worker_listener = Arc::clone(&listener);
    let worker = thread::spawn(move || {
        worker_listener.accept_authenticated_until(deadline, &super::NoopRejectionObserver)
    });
    let mut role = client(&endpoint).expect("open role");
    role.set_app_link_deadlines(Some(Duration::from_millis(500)))
        .expect("role deadlines");
    handshake_client(&mut role, &token).expect("HELLO reaches final boundary");
    entered_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("final authentication boundary");
    while Instant::now() < deadline {
        thread::yield_now();
    }
    release_tx.send(()).expect("release final boundary");
    let outcome = worker.join().expect("join admission").expect("admission");
    assert!(matches!(
        outcome,
        WindowsNamedPipeBootstrapAdmission::DeadlineElapsed
    ));
    let stale = WindowsNamedPipeServer::connect_client(&endpoint)
        .expect_err("expired final authentication must not consume a session");
    assert_eq!(stale.raw_os_error(), Some(2));
    drop(role);
}

/// The server's own consume gate is *after* the listener's final generation
/// clock check. Scheduling or lifecycle contention inside that gate must not
/// turn a now-expired generation into an authenticated app link. The accepted
/// HELLO and consumed instance are real native Windows named-pipe objects.
#[test]
fn generation_expiry_inside_server_consume_never_authenticates() {
    let listener = Arc::new(WindowsNamedPipeBootstrapListener::bind().expect("bind"));
    let link = listener.app_link();
    let (endpoint, token) = parse_app_link(&link).expect("parse app link");
    let endpoint = endpoint.to_owned();
    let server = listener
        .server
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .cloned()
        .expect("unconsumed server instance");
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::channel();
    server.install_before_consume_gate(TestConsumeGate {
        entered: entered_tx,
        release: release_rx,
    });

    // Preserve the existing native final-boundary deadline and bounded peer
    // handshake, rather than increasing a timeout to make this case pass.
    let deadline = Instant::now() + Duration::from_millis(100);
    let accepting = Arc::clone(&listener);
    let worker = thread::spawn(move || {
        accepting.accept_authenticated_until(deadline, &super::NoopRejectionObserver)
    });
    let mut role = client(&endpoint).expect("connect original test-owned endpoint");
    role.set_app_link_deadlines(Some(Duration::from_millis(500)))
        .expect("original bounded peer clock");
    handshake_client(&mut role, &token).expect("valid HELLO reaches server consume");
    entered_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("server.consume is blocked after the last generation deadline check");
    while Instant::now() < deadline {
        thread::yield_now();
    }
    release_tx
        .send(())
        .expect("release only the test-owned consume gate");
    let outcome = worker
        .join()
        .expect("join bounded admission")
        .expect("admission result");
    assert!(
        !matches!(
            &outcome,
            WindowsNamedPipeBootstrapAdmission::Authenticated(_)
        ),
        "a consumed generation authenticated a peer after its original deadline"
    );
    assert!(
        matches!(outcome, WindowsNamedPipeBootstrapAdmission::DeadlineElapsed),
        "expired generation must report its original deadline, not another cause"
    );
    let stale = WindowsNamedPipeServer::connect_client(&endpoint)
        .expect_err("expired consumed generation must close its named-pipe endpoint");
    assert_eq!(stale.raw_os_error(), Some(2));
    drop(role);
}
