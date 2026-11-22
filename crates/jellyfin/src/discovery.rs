//! Finding Jellyfin servers on the local network: a server answers a UDP broadcast of
//! `Who is JellyfinServer?` to port 7359 with a small JSON description of itself (on by default;
//! an admin can turn *Auto-discovery* off).
//!
//! The standard library can't list the machine's network interfaces, and a new dependency to do
//! it is more than this is worth, so the broadcast goes out the way the system routes it: from
//! the unbound socket, from the address the default route leaves by, and to that network's
//! `/24` directed broadcast too. A machine on several networks (a VPN beside the LAN) finds the
//! servers of its default route's network.

use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket as StdUdpSocket};
use std::time::Duration;

use futures_util::future::join_all;
use serde::Deserialize;
use tokio::net::UdpSocket;
use tokio::time::{Instant, timeout_at};

/// The port servers listen on for discovery.
const PORT: u16 = 7359;
const QUERY: &[u8] = b"Who is JellyfinServer?";
/// Anyone on the network can answer, so what's kept is bounded: the datagrams read from one
/// socket, the servers returned, and the characters of a name.
const MAX_DATAGRAMS: usize = 128;
const MAX_SERVERS: usize = 32;
const MAX_NAME_CHARS: usize = 80;
/// Longer than any real address; an address is rejected rather than cut, since a cut one points
/// somewhere else.
const MAX_ADDRESS_CHARS: usize = 200;

/// A server that answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredServer {
    /// Where to reach it, as the server itself says (`http://192.168.1.5:8096`).
    pub address: String,
    pub id: String,
    pub name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Reply {
    address: String,
    id: String,
    name: String,
}

/// Broadcasts the query and collects answers for `wait`: one entry per server and address (a
/// host can answer with another's `Id`, and both rows should show), by name, without the ones that only name this machine (a loopback address is no use to
/// anyone choosing from a list). Empty when nothing answers or the network can't broadcast.
pub async fn discover_servers(wait: Duration) -> Vec<DiscoveredServer> {
    let mut locals = vec![Ipv4Addr::UNSPECIFIED];
    let mut targets = vec![SocketAddr::from((Ipv4Addr::BROADCAST, PORT))];
    if let Some(local) = default_local_address() {
        let [a, b, c, _] = local.octets();
        locals.push(local);
        targets.push(SocketAddr::from((Ipv4Addr::new(a, b, c, 255), PORT)));
    }
    discover_with(&locals, &targets, wait).await
}

/// [`discover_servers`] from each of `locals` to each of `targets`.
async fn discover_with(locals: &[Ipv4Addr], targets: &[SocketAddr], wait: Duration) -> Vec<DiscoveredServer> {
    let deadline = Instant::now() + wait;
    let mut sockets = Vec::new();
    for &local in locals {
        let Ok(socket) = UdpSocket::bind((local, 0)).await else {
            continue;
        };
        if socket.set_broadcast(true).is_err() {
            continue;
        }
        for &target in targets {
            // A network that refuses (no route, no broadcast) just finds nothing there.
            let _ = socket.send_to(QUERY, target).await;
        }
        sockets.push(socket);
    }
    let heard = join_all(sockets.iter().map(|socket| listen(socket, deadline))).await;
    let mut seen = HashSet::new();
    let mut found: Vec<DiscoveredServer> = heard
        .into_iter()
        .flatten()
        .filter_map(|datagram| parse_reply(&datagram))
        .filter(|server| seen.insert((server.id.clone(), server.address.clone())))
        .take(MAX_SERVERS)
        .collect();
    found.sort_by_key(|server| server.name.to_lowercase());
    found
}

/// What `socket` receives until `deadline`, or until `MAX_DATAGRAMS` reads. A read that fails
/// (on Windows a datagram larger than the buffer, or an ICMP unreachable for the query) says
/// nothing about the next one, so it's skipped, and counts towards the limit.
async fn listen(socket: &UdpSocket, deadline: Instant) -> Vec<Vec<u8>> {
    let mut datagrams = Vec::new();
    let mut buffer = vec![0; 2048];
    for _ in 0..MAX_DATAGRAMS {
        match timeout_at(deadline, socket.recv_from(&mut buffer)).await {
            Ok(Ok((len, _))) => datagrams.push(buffer[..len].to_vec()),
            Ok(Err(_)) => {}
            Err(_) => break,
        }
    }
    datagrams
}

/// A server's answer, unless it isn't one or only names this machine.
fn parse_reply(datagram: &[u8]) -> Option<DiscoveredServer> {
    let reply: Reply = serde_json::from_slice(datagram).ok()?;
    if reply.address.chars().count() > MAX_ADDRESS_CHARS {
        return None;
    }
    let url = url::Url::parse(&reply.address).ok()?;
    if !matches!(url.scheme(), "http" | "https") || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    let local = match url.host()? {
        url::Host::Domain(name) => name.eq_ignore_ascii_case("localhost"),
        url::Host::Ipv4(ip) => ip.is_loopback() || ip.is_unspecified(),
        url::Host::Ipv6(ip) => ip.is_loopback() || ip.is_unspecified(),
    };
    if local || reply.id.is_empty() {
        return None;
    }
    Some(DiscoveredServer {
        address: reply.address.trim_end_matches('/').to_owned(),
        id: reply.id,
        name: reply.name.chars().take(MAX_NAME_CHARS).collect(),
    })
}

/// The address this machine reaches the network from, by asking the system which one it would
/// use towards an outside address (a UDP `connect` sends nothing).
fn default_local_address() -> Option<Ipv4Addr> {
    let socket = StdUdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(address: &str, id: &str, name: &str) -> Vec<u8> {
        serde_json::json!({ "Address": address, "Id": id, "Name": name, "EndpointAddress": null })
            .to_string()
            .into_bytes()
    }

    #[test]
    fn a_servers_answer_is_read_and_loopback_ones_are_not() {
        let server = parse_reply(&reply("http://192.168.1.5:8096/", "abc", "Home")).expect("a server");
        assert_eq!(
            server,
            DiscoveredServer {
                address: "http://192.168.1.5:8096".into(),
                id: "abc".into(),
                name: "Home".into(),
            }
        );
        for address in [
            "http://127.0.0.1:8096",
            "http://localhost:8096",
            "http://[::1]:8096",
            "http://0.0.0.0:8096",
        ] {
            assert_eq!(parse_reply(&reply(address, "abc", "Home")), None, "{address}");
        }
        assert_eq!(parse_reply(b"Who is JellyfinServer?"), None);
        assert_eq!(parse_reply(&reply("not a url", "abc", "Home")), None);
    }

    #[test]
    fn only_plain_http_addresses_are_kept_and_names_are_cut() {
        for address in [
            "ftp://192.168.1.5/",
            "file:///etc/passwd",
            "http://user:pw@192.168.1.5:8096",
            "http://u@192.168.1.5",
        ] {
            assert_eq!(parse_reply(&reply(address, "abc", "Home")), None, "{address}");
        }
        assert!(parse_reply(&reply("https://192.168.1.5", "abc", "Home")).is_some());
        let long = format!("http://192.168.1.5/{}", "a".repeat(300));
        assert_eq!(parse_reply(&reply(&long, "abc", "Home")), None);
        let named = parse_reply(&reply("http://192.168.1.5", "abc", &"n".repeat(500))).expect("a server");
        assert_eq!(named.name.chars().count(), MAX_NAME_CHARS);
    }

    #[tokio::test]
    async fn the_list_of_servers_is_capped() {
        let mut answers = Vec::new();
        for n in 0..100 {
            answers.push(reply("http://192.168.1.7:8096", &format!("id{n}"), "Many"));
        }
        let found = discover_with(
            &[Ipv4Addr::LOCALHOST],
            &[responder(answers).await],
            Duration::from_millis(400),
        )
        .await;
        assert_eq!(found.len(), MAX_SERVERS);
    }

    /// A host answering with the real server's id and name but its own address is another row,
    /// not a replacement.
    #[tokio::test]
    async fn the_same_id_at_two_addresses_shows_both() {
        let answers = vec![
            reply("http://192.168.1.5:8096", "real", "Home"),
            reply("http://192.168.1.66:8096", "real", "Home"),
        ];
        let found = discover_with(
            &[Ipv4Addr::LOCALHOST],
            &[responder(answers).await],
            Duration::from_millis(300),
        )
        .await;
        let addresses: Vec<&str> = found.iter().map(|server| server.address.as_str()).collect();
        assert_eq!(found.len(), 2, "{addresses:?}");
    }

    /// A server on a socket of its own: answers the query with each of `answers`.
    async fn responder(answers: Vec<Vec<u8>>) -> SocketAddr {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.expect("a socket");
        let address = socket.local_addr().expect("an address");
        tokio::spawn(async move {
            let mut buffer = [0; 64];
            let (len, from) = socket.recv_from(&mut buffer).await.expect("the query");
            assert_eq!(&buffer[..len], QUERY);
            for answer in answers {
                socket.send_to(&answer, from).await.expect("an answer");
            }
        });
        address
    }

    /// Answers from two servers, one twice and one only naming the machine, come back as the
    /// one server each of the first two is, by name.
    #[tokio::test]
    async fn answers_are_collected_deduplicated_and_sorted() {
        let first = responder(vec![
            reply("http://192.168.1.9:8096", "b", "zebra"),
            reply("http://192.168.1.9:8096", "b", "zebra"),
            reply("http://127.0.0.1:8096", "c", "mine"),
        ])
        .await;
        let second = responder(vec![reply("http://192.168.1.4:8096", "a", "Attic")]).await;
        let found = discover_with(&[Ipv4Addr::LOCALHOST], &[first, second], Duration::from_millis(400)).await;
        let names: Vec<&str> = found.iter().map(|server| server.name.as_str()).collect();
        assert_eq!(names, ["Attic", "zebra"]);
    }

    #[tokio::test]
    async fn nothing_answering_is_an_empty_list() {
        let silent = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.expect("a socket");
        let target = silent.local_addr().expect("an address");
        let found = discover_with(&[Ipv4Addr::LOCALHOST], &[target], Duration::from_millis(150)).await;
        assert!(found.is_empty());
    }
}
