//! End-to-end check of the API client against a real server.
//!
//! ```sh
//! cargo run -p jellyfin --example smoke
//! ```
//!
//! Env: `JELLYFIN_URL` (default `localhost:8096`), `JELLYFIN_USER` / `JELLYFIN_PASS`
//! (default `dev` / `dev`, matching `dev/setup-server.py`).

use std::time::Duration;

use anyhow::{Context, bail, ensure};
use jellyfin::models::{BaseItemKind, CollectionType, ItemField, ItemSortBy, MediaStreamType};
use jellyfin::{Client, ClientInfo, Error, ItemsQuery};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let url = env("JELLYFIN_URL", "localhost:8096");
    let user = env("JELLYFIN_USER", "dev");
    let pass = env("JELLYFIN_PASS", "dev");

    let anon = Client::new(
        &url,
        ClientInfo {
            client: "jelly-gpui-smoke".into(),
            device: "smoke-test".into(),
            device_id: "jelly-gpui-smoke-test".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
    )?;

    // Server probe, retrying while it boots.
    let info = {
        let mut attempts = 0;
        loop {
            match anon.public_info().await {
                Err(Error::ServerStarting) if attempts < 30 => {
                    attempts += 1;
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                other => break other?,
            }
        }
    };
    let version = info.server_version().context("server reported no version")?;
    ok(format!(
        "server {:?} at {} is version {version} (supported: {})",
        info.server_name.unwrap_or_default(),
        anon.base_url(),
        version.is_supported()
    ));

    // Auth.
    match anon.current_user().await {
        Err(Error::Unauthorized) => ok("unauthenticated /Users/Me is rejected"),
        other => bail!("expected Unauthorized, got {other:?}"),
    }
    match anon.authenticate_by_name(&user, "definitely-wrong").await {
        Err(Error::Unauthorized) => ok("wrong password is rejected"),
        other => bail!("expected Unauthorized, got {other:?}"),
    }
    let auth = anon.authenticate_by_name(&user, &pass).await?;
    let client = anon.with_token(auth.access_token.as_str());
    let me = client.current_user().await?;
    ensure!(me.id == auth.user.id);
    ok(format!("logged in as {} ({})", me.name, me.id));

    // Libraries.
    let fields = [ItemField::PrimaryImageAspectRatio, ItemField::Overview];
    let views = client.user_views().await?.items;
    ok(format!(
        "views: {}",
        views.iter().map(|v| v.name.as_str()).collect::<Vec<_>>().join(", ")
    ));
    for view in &views {
        let kinds = match view.collection_type {
            Some(CollectionType::Movies) => vec![BaseItemKind::Movie],
            Some(CollectionType::TvShows) => vec![BaseItemKind::Series],
            _ => continue,
        };
        let page = client
            .items(&ItemsQuery {
                parent_id: Some(view.id.clone()),
                include_item_types: kinds,
                recursive: true,
                sort_by: vec![ItemSortBy::SortName],
                fields: fields.to_vec(),
                start_index: Some(0),
                limit: Some(50),
                ..Default::default()
            })
            .await?;
        ok(format!(
            "{}: {} of {} items: {}",
            view.name,
            page.items.len(),
            page.total_record_count,
            page.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join(", ")
        ));
        let latest = client.latest(&view.id, 10, &fields).await?;
        ok(format!("{}: {} latest", view.name, latest.len()));
    }

    let resume = client.resume(10, &fields).await?;
    ok(format!("continue watching: {} items", resume.total_record_count));
    let next_up = client.next_up(10, &fields).await?;
    ok(format!("next up: {} items", next_up.total_record_count));

    // Series -> seasons -> episodes.
    let series = first_of(&client, BaseItemKind::Series).await?;
    let seasons = client.seasons(&series.id, &fields).await?.items;
    let season = seasons.first().context("series has no seasons")?;
    let episodes = client.episodes(&series.id, Some(&season.id), &fields).await?.items;
    ok(format!(
        "{}: {} seasons; {} has episodes {}",
        series.name,
        seasons.len(),
        season.name,
        episodes
            .iter()
            .map(|e| format!("S{:02}E{:02}", e.parent_index_number.unwrap_or(0), e.index_number.unwrap_or(0)))
            .collect::<Vec<_>>()
            .join(", ")
    ));

    // Versions and tracks for a few movies (MediaSources is only returned when requested).
    let movies = client
        .items(&ItemsQuery {
            include_item_types: vec![BaseItemKind::Movie],
            recursive: true,
            sort_by: vec![ItemSortBy::SortName],
            fields: vec![ItemField::MediaSources],
            limit: Some(5),
            ..Default::default()
        })
        .await?
        .items;
    for movie in &movies {
        for source in &movie.media_sources {
            let streams = |kind| {
                source
                    .media_streams
                    .iter()
                    .filter(|s| s.kind == kind)
                    .map(|s| {
                        let lang = s.language.as_deref().unwrap_or("und");
                        if s.is_external { format!("{lang} (external)") } else { lang.to_string() }
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            ok(format!(
                "{} version {:?}: audio [{}], subtitles [{}]",
                movie.name,
                source.name.as_deref().unwrap_or("-"),
                streams(MediaStreamType::Audio),
                streams(MediaStreamType::Subtitle),
            ));
        }
    }
    let movie = client
        .item(&movies.first().context("no movies on the server")?.id, &[ItemField::Overview])
        .await?;
    let resume_at = movie.user_data.as_ref().map(|d| d.playback_position_ticks).unwrap_or_default();
    ok(format!("{} detail loaded, resume position {:.1}s", movie.name, resume_at.as_secs_f64()));

    // Images.
    let req = movie.primary_image().context("movie has no primary image")?.fill_width(300);
    let bytes = client.image(&req).await?;
    ensure!(bytes.starts_with(&[0xFF, 0xD8]) || bytes.starts_with(b"RIFF"), "not a JPEG/WebP image");
    ok(format!("primary image: {} bytes from {}", bytes.len(), client.image_url(&req).path()));

    // Quick Connect.
    if anon.quick_connect_enabled().await? {
        let qc = anon.quick_connect_initiate().await?;
        let state = anon.quick_connect_state(&qc.secret).await?;
        ensure!(!state.authenticated);
        ok(format!("quick connect initiated, code {}", qc.code));
    } else {
        ok("quick connect is disabled on this server");
    }

    // Logout revokes the token.
    client.logout().await?;
    match client.current_user().await {
        Err(Error::Unauthorized) => ok("logout revoked the token"),
        other => bail!("expected Unauthorized after logout, got {other:?}"),
    }

    println!("\nall checks passed");
    Ok(())
}

async fn first_of(client: &Client, kind: BaseItemKind) -> anyhow::Result<jellyfin::models::BaseItemDto> {
    client
        .items(&ItemsQuery {
            include_item_types: vec![kind],
            recursive: true,
            sort_by: vec![ItemSortBy::SortName],
            limit: Some(1),
            ..Default::default()
        })
        .await?
        .items
        .into_iter()
        .next()
        .with_context(|| format!("no {kind:?} on the server"))
}

fn env(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}

fn ok(msg: impl AsRef<str>) {
    println!("ok  {}", msg.as_ref());
}
