//! `Error::is_certificate`: the login refuses to fall back from `https://` to `http://` after a
//! refused certificate, but must still fall back when the port just speaks plain HTTP.

use std::error::Error as _;
use std::sync::Arc;

use jellyfin::{Client, ClientInfo, Error};
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(server_url: &str) -> Client {
    let info = ClientInfo {
        client: "tabbyfin-tests".into(),
        device: "tests".into(),
        device_id: "test-device".into(),
        version: "0.1.0".into(),
    };
    Client::new(server_url, info).expect("server URL")
}

/// The error and its causes, for a failing assertion.
fn chain(error: &Error) -> String {
    let mut text = format!("{error:?}");
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(&format!("\n  caused by: {cause:?}"));
        source = cause.source();
    }
    text
}

/// A TLS server on a free port whose certificate no client trusts (self-signed). Returns its port.
async fn self_signed_server() -> u16 {
    let certified = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string()]).expect("a certificate");
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der()));
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("protocol versions")
        .with_no_client_auth()
        .with_single_cert(vec![certified.cert.der().clone()], key)
        .expect("a server config");
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("address").port();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            // The client walks away mid-handshake; that's the point.
            tokio::spawn(async move {
                let _ = acceptor.accept(stream).await;
            });
        }
    });
    port
}

#[tokio::test]
async fn a_refused_certificate_is_recognised() {
    let port = self_signed_server().await;
    let error = client(&format!("https://127.0.0.1:{port}"))
        .public_info()
        .await
        .expect_err("an untrusted certificate");
    assert!(error.is_certificate(), "not recognised: {}", chain(&error));
}

/// `host:8096` is tried as `https://` first: a plain-HTTP server fails that handshake, and the
/// login must go on to `http://`.
#[tokio::test]
async fn plain_http_behind_https_is_not_a_certificate_problem() {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let url = server.uri().replacen("http://", "https://", 1);
    let error = client(&url).public_info().await.expect_err("no TLS there");
    assert!(matches!(error, Error::Http(_)), "{}", chain(&error));
    assert!(!error.is_certificate(), "taken for a certificate: {}", chain(&error));
}

#[tokio::test]
async fn a_closed_port_is_not_a_certificate_problem() {
    // Bind, note the port, and let go of it: nothing listens there now.
    let port = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind")
        .local_addr()
        .expect("address")
        .port();
    let error = client(&format!("https://127.0.0.1:{port}"))
        .public_info()
        .await
        .expect_err("nothing listens");
    assert!(matches!(error, Error::Http(_)), "{}", chain(&error));
    assert!(!error.is_certificate(), "{}", chain(&error));
}

#[tokio::test]
async fn the_servers_own_answers_are_not_certificate_problems() {
    assert!(!Error::NotFound.is_certificate());
    assert!(!Error::Unauthorized.is_certificate());
    assert!(!Error::Forbidden.is_certificate());
    assert!(!Error::ServerStarting.is_certificate());
}
