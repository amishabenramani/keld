//! Existing macOS descriptor census binds one connected application socket pair.
use crate::support::unix_descriptors::{UnixDescriptor, unix_descriptors};
use std::collections::BTreeSet;

// Diagnostic counts are not socket authority. Never print the inspected PID,
// private endpoint, descriptor ID or kernel socket address to test stderr.
fn safe_socket_census_marker(
    host_rows: &[UnixDescriptor],
    bun_rows: &[UnixDescriptor],
    pairs: &BTreeSet<(String, String)>,
) -> String {
    format!(
        "KELD_KEL140_APPLICATION_SOCKET_CENSUS host_fd_count={} bun_fd_count={} connected_pairs={}",
        host_rows.len(),
        bun_rows.len(),
        pairs.len()
    )
}

pub(super) fn assert_single_application_link(host: u32, bun: u32, endpoint: &str) {
    let host_rows = unix_descriptors(host);
    let bun_rows = unix_descriptors(bun);
    // Bound listeners and accepted FD clones can share a pathname. A Bun
    // connected-peer identity selects the actual host socket object instead.
    let pairs = application_socket_pairs(&host_rows, &bun_rows, endpoint);
    eprintln!(
        "{}",
        safe_socket_census_marker(&host_rows, &bun_rows, &pairs)
    );
    assert_eq!(pairs.len(), 1, "one connected application socket pair");
    let (server_socket, client_socket) = pairs.first().expect("one actual connected pair");
    let client_peer = format!("->{server_socket}");
    for row in bun_rows.iter().filter(|row| &row.socket == client_socket) {
        assert!(
            row.identity == client_peer,
            "client FD clone changed its connected peer"
        );
    }
    for row in host_rows.iter().filter(|row| &row.socket == server_socket) {
        if let Some(peer) = row.identity.strip_prefix("->") {
            assert!(
                peer == client_socket.as_str(),
                "host connected-peer symmetry"
            );
        } else {
            assert!(
                row.identity == endpoint,
                "accepted host socket lost the declared endpoint"
            );
        }
    }
}

fn application_socket_pairs(
    host_rows: &[UnixDescriptor],
    bun_rows: &[UnixDescriptor],
    endpoint: &str,
) -> BTreeSet<(String, String)> {
    let mut pairs = BTreeSet::new();
    for client in bun_rows {
        if let Some(peer) = client.identity.strip_prefix("->")
            && host_rows
                .iter()
                .any(|server| server.socket == peer && server.identity == endpoint)
        {
            pairs.insert((peer.to_owned(), client.socket.clone()));
        }
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_link_census_preserves_private_pairing_without_leaking_identity() {
        const PRIVATE: &str = "KELD_TEST_PRIVATE_ENDPOINT_TOKEN_PATH";
        let host_rows = [UnixDescriptor {
            descriptor: PRIVATE.to_owned(),
            socket: "0xHOST".to_owned(),
            identity: PRIVATE.to_owned(),
        }];
        let bun_rows = [UnixDescriptor {
            descriptor: PRIVATE.to_owned(),
            socket: "0xBUN".to_owned(),
            identity: "->0xHOST".to_owned(),
        }];
        let pairs = application_socket_pairs(&host_rows, &bun_rows, PRIVATE);
        assert!(
            pairs.len() == 1,
            "actual connected peer relation must still match"
        );
        let diagnostic = safe_socket_census_marker(&host_rows, &bun_rows, &pairs);
        assert!(
            diagnostic
                == "KELD_KEL140_APPLICATION_SOCKET_CENSUS host_fd_count=1 bun_fd_count=1 connected_pairs=1",
            "diagnostic must contain counts only"
        );
        assert!(
            !diagnostic.contains(PRIVATE),
            "private endpoint leaked into diagnostic"
        );
        assert!(
            !diagnostic.contains("0xHOST"),
            "kernel socket identity leaked"
        );
        assert!(
            !diagnostic.contains("0xBUN"),
            "kernel socket identity leaked"
        );
    }

    #[test]
    fn application_link_stderr_callsite_must_remain_count_only() {
        let (implementation, _) = include_str!("application_link.rs")
            .split_once("#[cfg(test)]\nmod tests")
            .expect("source-binding regression owns adjacent test boundary");
        let compact: String = implementation
            .chars()
            .filter(|char| !char.is_whitespace())
            .collect();
        assert!(
            compact.contains(
                "eprintln!(\"{}\",safe_socket_census_marker(&host_rows,&bun_rows,&pairs));"
            ),
            "actual socket census stderr callsite bypassed safe counter formatter"
        );
        assert!(
            implementation.matches("eprintln!(").count() == 1,
            "socket census must not add a second unreviewed diagnostic sink"
        );
        assert!(
            !implementation.contains("endpoint={endpoint}")
                && !implementation.contains("host={host}")
                && !implementation.contains("{pairs:?}"),
            "raw socket census interpolation must not return"
        );
    }
}
