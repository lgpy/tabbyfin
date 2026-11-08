//! Downloading an item's original file: the URL, range requests and the body stream.
//!
//! The shared client's 30 s timeout is meant for API calls, where a slow answer means the
//! server is in trouble. A download can take hours, so [`Client::download`] overrides it and
//! fails on a stall instead: no bytes for [`STALL_TIMEOUT`] is a broken connection, whatever
//! the file's size.

use std::time::Duration;

use reqwest::{Response, StatusCode, header};
use url::Url;

use crate::{Client, Error, Result};

/// How long a download may go without a byte before it counts as stalled.
pub const STALL_TIMEOUT: Duration = Duration::from_secs(30);

/// The whole request's limit, as a backstop to [`STALL_TIMEOUT`] for a server that trickles a
/// byte just often enough. It isn't a speed limit: 12 hours is plenty for a 4K remux on a slow
/// line.
const TOTAL_TIMEOUT: Duration = Duration::from_secs(12 * 60 * 60);

/// A download under way: where its body starts in the file, and how long the file is.
pub struct Download {
    response: Response,
    /// The server answered the `Range` request with `206 Partial Content`, so the body starts at
    /// the offset that was asked for. `false` means it sent the whole file from the start.
    pub partial: bool,
    /// The file's whole length, from `Content-Range` (206) or `Content-Length` (200), when the
    /// server says. Without it, a download only ends when the connection does.
    pub total: Option<u64>,
}

impl Download {
    /// The next piece of the body, `None` once all of it has arrived. Fails with
    /// [`Error::Stalled`] when nothing arrives for [`STALL_TIMEOUT`].
    pub async fn chunk(&mut self) -> Result<Option<Vec<u8>>> {
        match tokio::time::timeout(STALL_TIMEOUT, self.response.chunk()).await {
            Ok(Ok(Some(bytes))) => Ok(Some(bytes.to_vec())),
            Ok(Ok(None)) => Ok(None),
            Ok(Err(err)) => Err(Error::Http(err)),
            Err(_) => Err(Error::Stalled),
        }
    }
}

impl Client {
    /// The item's original file as the server stores it (`GET /Items/{id}/Download`), in the
    /// default version. Like [`Client::stream_url`], the URL carries no token: authenticate with
    /// [`Client::authorization_header`], which a download's request does.
    pub fn download_url(&self, item_id: &str) -> Url {
        self.url(&format!("/Items/{item_id}/Download"))
    }

    /// Starts downloading `url` from `offset`: a `Range: bytes={offset}-` asks the server for
    /// the rest of the file, which it may or may not honour (see [`Download::partial`]). Read
    /// the body with [`Download::chunk`].
    pub async fn download(&self, url: Url, offset: u64) -> Result<Download> {
        let mut request = self.http().get(url).timeout(TOTAL_TIMEOUT);
        if offset > 0 {
            request = request.header(header::RANGE, format!("bytes={offset}-"));
        }
        let response = self.send(request).await?;
        let partial = response.status() == StatusCode::PARTIAL_CONTENT;
        let total = if partial {
            content_range_total(&response)
        } else {
            response.content_length()
        };
        Ok(Download {
            response,
            partial,
            total,
        })
    }
}

/// The file's length in a `Content-Range: bytes 100-599/600` header. The server may send `*`
/// instead when it doesn't know, and some proxies leave the header out of a 206.
fn content_range_total(response: &Response) -> Option<u64> {
    let value = response.headers().get(header::CONTENT_RANGE)?.to_str().ok()?;
    value.rsplit('/').next()?.trim().parse().ok()
}
