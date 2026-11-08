//! The live check: find a device, launch Jellyfin's receiver on it, and print what it reports.
//!
//! ```sh
//! mise run cast                                  # discover, launch, Identify
//! mise run cast -- --ip 192.168.1.20             # skip discovery
//! mise run cast -- --device "living room"        # pick a device by name
//! mise run cast -- --app 6F511C87                # the unstable receiver
//! mise run cast -- --play <item id>              # cast a dev-server item
//! ```
//!
//! `--play` signs in to the dev server (`JELLYFIN_URL`, default `localhost:8096`, with
//! `JELLYFIN_USER` / `JELLYFIN_PASS`, default `dev` / `dev`, as `dev/setup-server.py` sets up) and
//! sends its real credentials, so the TV can fetch the item. Without `--play` the same sign-in is
//! tried for `Identify`'s credentials, and skipped with a note when the server isn't running.
//!
//! The TV must be able to reach the server address that's printed: a server on `localhost` is
//! replaced with its LAN address, and an HTTPS server with a self-signed certificate won't work.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use anyhow::{Context, bail, ensure};
use cast::jellyfin::{self as jf, Command, Credentials, Envelope, ItemStub, MediaStatus, PlayRequest, ReceiverEvent};
use cast::{Channel, Device, Discovery, DiscoveryEvent, Message, receiver};
use jellyfin::Client;
use jellyfin::models::BaseItemDto;

/// The port a single device listens on.
const PORT: u16 = 8009;
/// How long discovery runs, which is how long a device takes to answer.
const DISCOVERY: Duration = Duration::from_secs(5);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let args = Args::parse()?;

    let device = match args.ip {
        Some(address) => Device {
            id: String::new(),
            name: address.ip().to_string(),
            model: "by address".into(),
            address: address.ip(),
            port: address.port(),
            running_app: None,
        },
        None => discover(&args).await?,
    };
    println!("device: {} at {}", device.name, device.socket_addr());

    let (channel, mut messages) = Channel::connect(device.socket_addr())
        .await
        .with_context(|| format!("connecting to {}", device.socket_addr()))?;
    let app = receiver::launch(&channel, &args.app)
        .await
        .with_context(|| format!("launching {}", args.app))?;
    ensure!(
        app.has_namespace(jf::NAMESPACE),
        "the launched app ({}) doesn't listen on {}, so it isn't Jellyfin's receiver",
        app.app_id,
        jf::NAMESPACE
    );
    channel.connect_app(&app.transport_id).await?;
    println!(
        "launched {} ({}), transport id {}, session {}",
        app.display_name.as_deref().unwrap_or("the app"),
        app.app_id,
        app.transport_id,
        app.session_id
    );

    // Identify needs the same credentials as a play command, even though it plays nothing.
    let signed_in = match dev_server().await {
        Ok(pair) => Some(pair),
        Err(error) if args.play.is_none() => {
            println!("not signing in, so Identify is sent without credentials: {error:#}");
            None
        }
        Err(error) => return Err(error),
    };
    let account = signed_in
        .as_ref()
        .map(|(_, account)| account.clone())
        .unwrap_or_default();
    let credentials = Credentials {
        server_address: account.server_address.clone(),
        access_token: account.access_token.clone(),
        user_id: account.user_id.clone(),
        device_id: account.device_id.clone(),
        server_id: account.server_id.clone(),
        server_version: account.server_version.clone(),
        receiver_name: device.name.clone(),
        max_bitrate: None,
        subtitle_appearance: None,
    };
    if !credentials.server_address.is_empty() {
        println!(
            "the TV will reach the server at {} as {}",
            credentials.server_address, credentials.user_id
        );
    }

    channel
        .send(
            jf::NAMESPACE,
            &app.transport_id,
            Envelope::new(&credentials, Command::Identify).to_value(),
        )
        .await?;
    println!("sent Identify");

    if let Some(item_id) = &args.play {
        let (client, _) = signed_in.as_ref().expect("--play signs in or returns the error");
        let item = client
            .item(item_id, &[])
            .await
            .with_context(|| format!("fetching item {item_id}"))?;
        println!("playing {} ({})", item.name, item.id);
        let request = PlayRequest {
            items: vec![stub(&item, &credentials.server_id)],
            ..PlayRequest::default()
        };
        channel
            .send(
                jf::NAMESPACE,
                &app.transport_id,
                Envelope::new(&credentials, Command::PlayNow(request)).to_value(),
            )
            .await?;
    }

    println!("waiting for events (Ctrl-C to stop)");
    while let Some(message) = messages.recv().await {
        print_message(&message);
    }
    println!("the device closed the connection");
    Ok(())
}

struct Args {
    /// An address to skip discovery with, port optional.
    ip: Option<SocketAddr>,
    /// A substring of the device's name.
    device: Option<String>,
    app: String,
    play: Option<String>,
}

impl Args {
    fn parse() -> anyhow::Result<Self> {
        let mut args = Self {
            ip: None,
            device: None,
            app: jf::STABLE_APP_ID.to_string(),
            play: None,
        };
        let mut argv = std::env::args().skip(1);
        while let Some(flag) = argv.next() {
            let mut value = || argv.next().with_context(|| format!("{flag} needs a value"));
            match flag.as_str() {
                "--ip" => args.ip = Some(parse_address(&value()?)?),
                "--device" => args.device = Some(value()?),
                "--app" => args.app = value()?,
                "--play" => args.play = Some(value()?),
                "--help" | "-h" => {
                    println!(
                        concat!(
                            "usage: cast [--ip ADDRESS[:PORT]] [--device NAME] [--app APP_ID] [--play ITEM_ID]\n",
                            "\n",
                            "  --ip      connect straight to a device, skipping discovery\n",
                            "  --device  pick the first found device whose name contains this\n",
                            "  --app     the receiver app id (default {})\n",
                            "  --play    cast an item from the dev server",
                        ),
                        jf::STABLE_APP_ID
                    );
                    std::process::exit(0);
                }
                other => bail!("unknown argument {other:?}; try --help"),
            }
        }
        Ok(args)
    }
}

fn parse_address(value: &str) -> anyhow::Result<SocketAddr> {
    if let Ok(address) = value.parse::<SocketAddr>() {
        return Ok(address);
    }
    let ip: IpAddr = value
        .parse()
        .with_context(|| format!("{value:?} is not an IP address (use [::1]:{PORT} for IPv6)"))?;
    Ok(SocketAddr::new(ip, PORT))
}

async fn discover(args: &Args) -> anyhow::Result<Device> {
    let (discovery, mut events) = Discovery::start().context("starting mDNS discovery")?;
    println!("looking for devices for {} s...", DISCOVERY.as_secs());
    let mut devices: Vec<Device> = Vec::new();
    let deadline = tokio::time::Instant::now() + DISCOVERY;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, events.recv()).await {
            Ok(Some(DiscoveryEvent::Found(device) | DiscoveryEvent::Updated(device))) => {
                println!("  {} ({}) at {}", device.name, device.model, device.socket_addr());
                match devices.iter_mut().find(|known| known.id == device.id) {
                    Some(known) => *known = device,
                    None => devices.push(device),
                }
            }
            Ok(Some(DiscoveryEvent::Lost(id))) => devices.retain(|known| known.id != id),
            Ok(None) | Err(_) => break,
        }
    }
    drop(discovery);

    let wanted = args.device.as_deref().map(str::to_lowercase);
    let named = |device: &Device| match &wanted {
        Some(wanted) => device.name.to_lowercase().contains(wanted),
        None => true,
    };
    if devices.is_empty() {
        bail!("no devices found; try --ip <address>");
    }
    match devices.into_iter().find(named) {
        Some(device) => Ok(device),
        None => bail!(
            "no device name contains {:?}",
            args.device.as_deref().unwrap_or_default()
        ),
    }
}

/// The credentials for a dev server, filled in by signing in.
#[derive(Clone, Default)]
struct Account {
    server_address: String,
    access_token: String,
    user_id: String,
    device_id: String,
    server_id: String,
    server_version: String,
}

async fn dev_server() -> anyhow::Result<(Client, Account)> {
    let url = env("JELLYFIN_URL", "localhost:8096");
    let user = env("JELLYFIN_USER", "dev");
    let password = env("JELLYFIN_PASS", "dev");
    let anonymous = Client::new(
        &url,
        jellyfin::ClientInfo {
            client: "tabbyfin-cast".into(),
            device: "cast example".into(),
            device_id: "tabbyfin-cast-example".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
    )?;
    let info = anonymous
        .public_info()
        .await
        .with_context(|| format!("asking {url} who it is"))?;
    let auth = anonymous
        .authenticate_by_name(&user, &password)
        .await
        .with_context(|| format!("signing in to {url} as {user}"))?;
    let client = anonymous.with_token(auth.access_token.as_str());
    println!("signed in to {url} as {} ({})", auth.user.name, auth.user.id);

    Ok((
        client.clone(),
        Account {
            server_address: server_address(&client, info.local_address.as_deref()),
            access_token: auth.access_token,
            user_id: auth.user.id,
            device_id: "tabbyfin-cast-example".into(),
            server_id: auth.server_id.or(info.id).unwrap_or_default(),
            server_version: info.version.unwrap_or_default(),
        },
    ))
}

/// The address to hand the TV: a loopback server is replaced with its LAN address, because
/// `localhost` on the TV is the TV (docs/chromecast.md, "The Jellyfin receiver").
fn server_address(client: &Client, local_address: Option<&str>) -> String {
    let url = client.base_url();
    let loopback = match url.host_str() {
        Some("localhost") => true,
        Some(host) => host
            .trim_matches(['[', ']'])
            .parse::<IpAddr>()
            .is_ok_and(|ip| ip.is_loopback()),
        None => true,
    };
    let address = if loopback {
        local_address
            .filter(|address| !address.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| url.to_string())
    } else {
        url.to_string()
    };
    address.trim_end_matches('/').to_string()
}

/// An item as the receiver fetches it: only these fields, with the server's own key names.
fn stub(item: &BaseItemDto, server_id: &str) -> ItemStub {
    ItemStub {
        id: item.id.clone(),
        server_id: item.server_id.clone().unwrap_or_else(|| server_id.to_string()),
        name: item.name.clone(),
        kind: item.kind.as_str().map(str::to_string).unwrap_or_default(),
        // The API item has no media type, and everything this example casts is video.
        media_type: "Video".into(),
        is_folder: item.is_folder,
    }
}

fn print_message(message: &Message) {
    match message.namespace.as_str() {
        jf::NAMESPACE => match ReceiverEvent::from_value(&message.payload) {
            Ok(event) => println!("{}", describe(&event)),
            Err(error) => println!("unreadable event ({error}): {}", message.payload),
        },
        cast::NS_MEDIA => match MediaStatus::from_value(&message.payload) {
            Ok(status) => match status.entry() {
                Some(entry) => println!(
                    "media: {} {} at {:.1}s{}",
                    entry.item_id().unwrap_or("?"),
                    entry.player_state.as_deref().unwrap_or("?"),
                    entry.position_seconds().unwrap_or(0.0),
                    entry
                        .media
                        .as_ref()
                        .and_then(|media| media.duration)
                        .map(|duration| format!(" of {duration:.0}s"))
                        .unwrap_or_default(),
                ),
                None => println!("media: nothing playing"),
            },
            Err(error) => println!("unreadable media status ({error}): {}", message.payload),
        },
        cast::NS_RECEIVER => println!("receiver: {}", message.payload),
        other => println!("{other}: {}", message.payload),
    }
}

fn describe(event: &ReceiverEvent) -> String {
    let kind = match event {
        ReceiverEvent::PlaybackStart(_) => "playbackstart",
        ReceiverEvent::PlaybackProgress(_) => "playbackprogress",
        ReceiverEvent::PlayStateChange(_) => "playstatechange",
        ReceiverEvent::VolumeChange(_) => "volumechange",
        ReceiverEvent::PlaybackStop(_) => "playbackstop",
        ReceiverEvent::PlaybackError(_) => "playbackerror",
        ReceiverEvent::ConnectionError(_) => "connectionerror",
        ReceiverEvent::Error(_) => "error",
        ReceiverEvent::RepeatModeChange(_) => "repeatmodechange",
        ReceiverEvent::Other(kind, _) => kind,
    };
    let event = event.event();
    let mut line = kind.to_string();
    if let Some(message) = &event.message {
        line.push_str(&format!(" {message}"));
    }
    if let Some(item) = &event.data.now_playing_item {
        if let Some(series) = &item.series_name {
            line.push_str(&format!(" {series}"));
        }
        line.push_str(&format!(" {}", item.name.as_deref().unwrap_or("?")));
    } else if let Some(item_id) = &event.data.item_id {
        line.push_str(&format!(" {item_id}"));
    }
    if let Some(state) = &event.data.play_state {
        if let Some(ticks) = state.position_ticks {
            line.push_str(&format!(" at {:.1}s", ticks as f64 / jf::TICKS_PER_SECOND));
        }
        match state.is_paused {
            Some(true) => line.push_str(" paused"),
            Some(false) => line.push_str(" playing"),
            None => {}
        }
        if let Some(volume) = state.volume_level {
            line.push_str(&format!(" volume {volume:.0}%"));
        }
    }
    line
}

fn env(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}
