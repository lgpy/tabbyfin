use std::sync::Arc;
use std::time::Duration;

use reqwest::{RequestBuilder, Response, StatusCode, header};
use serde::Serialize;
use serde::de::DeserializeOwned;
use url::Url;

use crate::{Error, Result};

/// Identifies this app and device to the server. Sent with every request.
#[derive(Debug, Clone)]
pub struct ClientInfo {
    /// App name, e.g. `jelly-gpui`.
    pub client: String,
    /// Human-readable device name, e.g. the hostname.
    pub device: String,
    /// Stable per-install id. Jellyfin ties sessions to it, so generate it once and persist it.
    pub device_id: String,
    pub version: String,
}

/// A Jellyfin server connection. Cheap to clone.
///
/// Created without a token for public endpoints and login; use [`Client::with_token`] for the
/// authenticated client.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: Url,
    info: Arc<ClientInfo>,
    token: Option<Arc<str>>,
}

impl Client {
    pub fn new(server_url: &str, info: ClientInfo) -> Result<Self> {
        let base = parse_server_url(server_url)?;
        let http = reqwest::Client::builder()
            .user_agent(format!("{}/{}", info.client, info.version))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            http,
            base,
            info: Arc::new(info),
            token: None,
        })
    }

    /// Returns a client that authenticates with `token`. Shares the connection pool.
    pub fn with_token(&self, token: impl Into<Arc<str>>) -> Self {
        Self {
            token: Some(token.into()),
            ..self.clone()
        }
    }

    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    pub fn base_url(&self) -> &Url {
        &self.base
    }

    pub fn info(&self) -> &ClientInfo {
        &self.info
    }

    /// The `Authorization` header value. Also handed to mpv so stream requests authenticate
    /// without putting the token in URLs.
    pub fn authorization_header(&self) -> String {
        let info = &self.info;
        let mut value = format!(
            r#"MediaBrowser Client="{}", Device="{}", DeviceId="{}", Version="{}""#,
            encode(&info.client),
            encode(&info.device),
            encode(&info.device_id),
            encode(&info.version),
        );
        if let Some(token) = &self.token {
            value.push_str(&format!(r#", Token="{}""#, encode(token)));
        }
        value
    }

    pub(crate) fn url(&self, path: &str) -> Url {
        self.base
            .join(path.trim_start_matches('/'))
            .expect("API paths are valid relative URLs")
    }

    pub(crate) async fn get<T: DeserializeOwned>(&self, path: &str, query: &Query) -> Result<T> {
        let req = self.http.get(self.url(path)).query(&query.0);
        decode(self.send(req).await?).await
    }

    pub(crate) async fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        query: &Query,
        body: Option<&B>,
    ) -> Result<T> {
        decode(self.send(self.post_request(path, query, body)).await?).await
    }

    /// POST for endpoints that answer `204 No Content`.
    pub(crate) async fn post_no_content<B: Serialize>(
        &self,
        path: &str,
        query: &Query,
        body: Option<&B>,
    ) -> Result<()> {
        self.send(self.post_request(path, query, body)).await?;
        Ok(())
    }

    pub(crate) async fn delete(&self, path: &str) -> Result<()> {
        self.send(self.http.delete(self.url(path))).await?;
        Ok(())
    }

    pub(crate) async fn get_bytes(&self, url: Url) -> Result<Vec<u8>> {
        let resp = self.send(self.http.get(url)).await?;
        Ok(resp.bytes().await?.to_vec())
    }

    fn post_request<B: Serialize>(&self, path: &str, query: &Query, body: Option<&B>) -> RequestBuilder {
        let req = self.http.post(self.url(path)).query(&query.0);
        match body {
            Some(body) => req.json(body),
            None => req,
        }
    }

    async fn send(&self, req: RequestBuilder) -> Result<Response> {
        let resp = req
            .header(header::AUTHORIZATION, self.authorization_header())
            .send()
            .await?;
        let status = resp.status();
        tracing::debug!(url = %resp.url().path(), %status, "jellyfin request");
        match status {
            s if s.is_success() => Ok(resp),
            StatusCode::UNAUTHORIZED => Err(Error::Unauthorized),
            StatusCode::FORBIDDEN => Err(Error::Forbidden),
            StatusCode::NOT_FOUND => Err(Error::NotFound),
            // Returned with a plain-text "Jellyfin Server is loading" body during startup.
            StatusCode::SERVICE_UNAVAILABLE => Err(Error::ServerStarting),
            s => {
                let mut message = resp.text().await.unwrap_or_default();
                message.truncate(500);
                Err(Error::Status {
                    status: s.as_u16(),
                    message,
                })
            }
        }
    }
}

async fn decode<T: DeserializeOwned>(resp: Response) -> Result<T> {
    let text = resp.text().await?;
    let de = &mut serde_json::Deserializer::from_str(&text);
    serde_path_to_error::deserialize(de).map_err(|err| Error::Decode {
        path: err.path().to_string(),
        source: err.into_inner(),
    })
}

/// The server URL-decodes auth header values, so encode anything that could break the
/// `key="value", ...` syntax (quotes, commas, non-ASCII device names).
fn encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

/// Normalizes user input like `jf.example.com`, `http://host:8096/` or
/// `https://example.com/jellyfin` into a base URL that relative API paths can be joined to.
/// Assumes `http://` when no scheme is given.
pub fn parse_server_url(input: &str) -> Result<Url> {
    let input = input.trim();
    let with_scheme = if input.contains("://") {
        input.to_owned()
    } else {
        format!("http://{input}")
    };
    let mut url = Url::parse(&with_scheme).map_err(|e| Error::InvalidUrl(format!("{input}: {e}")))?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return Err(Error::InvalidUrl(input.to_owned()));
    }
    url.set_query(None);
    url.set_fragment(None);
    // A trailing slash makes `join` keep a reverse-proxy subpath like `/jellyfin`.
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

/// Query string builder. Jellyfin takes lists as comma-separated values.
#[derive(Default)]
pub(crate) struct Query(pub(crate) Vec<(&'static str, String)>);

impl Query {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, key: &'static str, value: impl ToString) -> &mut Self {
        self.0.push((key, value.to_string()));
        self
    }

    pub fn opt(&mut self, key: &'static str, value: Option<impl ToString>) -> &mut Self {
        if let Some(value) = value {
            self.push(key, value);
        }
        self
    }

    pub fn list<T: AsRef<str>>(&mut self, key: &'static str, values: &[T]) -> &mut Self {
        if !values.is_empty() {
            let joined = values.iter().map(AsRef::as_ref).collect::<Vec<_>>().join(",");
            self.push(key, joined);
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_url_normalization() {
        let cases = [
            ("localhost:8096", "http://localhost:8096/"),
            ("http://localhost:8096/", "http://localhost:8096/"),
            ("https://example.com/jellyfin", "https://example.com/jellyfin/"),
            (
                "  https://example.com/jellyfin/?x=1#y ",
                "https://example.com/jellyfin/",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(parse_server_url(input).unwrap().as_str(), expected, "{input}");
        }
        assert!(parse_server_url("ftp://example.com").is_err());
        assert!(parse_server_url("").is_err());
    }

    #[test]
    fn join_keeps_subpath() {
        let info = ClientInfo {
            client: "c".into(),
            device: "d".into(),
            device_id: "id".into(),
            version: "1".into(),
        };
        let client = Client::new("https://example.com/jellyfin", info).unwrap();
        assert_eq!(
            client.url("/Users/Me").as_str(),
            "https://example.com/jellyfin/Users/Me"
        );
    }

    #[test]
    fn auth_header_encodes_values() {
        let info = ClientInfo {
            client: "jelly-gpui".into(),
            device: r#"Lu's "PC", home"#.into(),
            device_id: "abc".into(),
            version: "0.1.0".into(),
        };
        let client = Client::new("localhost", info).unwrap().with_token("tok");
        assert_eq!(
            client.authorization_header(),
            r#"MediaBrowser Client="jelly-gpui", Device="Lu%27s+%22PC%22%2C+home", DeviceId="abc", Version="0.1.0", Token="tok""#
        );
    }
}
