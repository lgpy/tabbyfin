//! Searching the server's subtitle providers (the Open Subtitles plugin and others) and
//! having it download a result into the library. Both need the user's *subtitle management*
//! permission ([`crate::models::UserPolicy::enable_subtitle_management`]); administrators
//! don't pass it automatically.

use crate::client::Query;
use crate::models::RemoteSubtitleInfo;
use crate::{Client, Result};

impl Client {
    /// Subtitles the providers have for a movie or episode in one language, a three-letter
    /// ISO 639-2 code (`eng`, `por`). `perfect_match_only` keeps the ones matched by the
    /// file's hash. The list is empty when nothing is found, and also when no provider is
    /// enabled for the item's library: the server doesn't tell the two apart, and has no
    /// endpoint that lists the providers.
    ///
    /// Available since 10.8; `404` for an item that isn't a video.
    pub async fn search_subtitles(
        &self,
        item_id: &str,
        language: &str,
        perfect_match_only: bool,
    ) -> Result<Vec<RemoteSubtitleInfo>> {
        let mut q = Query::new();
        if perfect_match_only {
            q.push("isPerfectMatch", true);
        }
        self.get(
            &format!("/Items/{item_id}/RemoteSearch/Subtitles/{}", path_segment(language)),
            &q,
        )
        .await
    }

    /// Has the server download a search result (a [`RemoteSubtitleInfo::id`]) and add it to
    /// the item. The server answers `204` even when the download fails (it only logs it), and
    /// refreshes the item's metadata in the background, so the new stream shows up in the
    /// item's `MediaStreams` a moment later, not when this returns.
    pub async fn download_subtitle(&self, item_id: &str, subtitle_id: &str) -> Result<()> {
        self.post_no_content::<()>(
            &format!("/Items/{item_id}/RemoteSearch/Subtitles/{}", path_segment(subtitle_id)),
            &Query::new(),
            None,
        )
        .await
    }
}

/// Percent-encodes a value for one path segment: provider ids can hold `/`, `+` and `=`.
fn path_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::path_segment;

    #[test]
    fn path_segments_are_encoded() {
        assert_eq!(path_segment("eng"), "eng");
        assert_eq!(path_segment("os_a/b+c=="), "os_a%2Fb%2Bc%3D%3D");
        assert_eq!(path_segment("é"), "%C3%A9");
    }
}
