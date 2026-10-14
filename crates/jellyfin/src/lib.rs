//! Jellyfin API client. Kept free of GPUI and mpv so it can be built and tested on its own.
//!
//! See `docs/PLAN.md` §4. Only the `Authorization: MediaBrowser ...` header (or the `ApiKey`
//! query parameter) is used; legacy auth methods are disabled by default since Jellyfin 12.0.
