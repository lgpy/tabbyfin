//! Spike: video inside a GPUI window through libmpv's software renderer (PLAN §5.4 option C).
//! Findings are in `docs/spike-inline-video.md`.
//!
//! ```sh
//! cargo run --release -p jelly-gpui --example inline_video -- <url> [header] [WxH]
//! ```
//!
//! Shows the video with a GPUI overlay (stats and controls) and prints stats every 2 s:
//! frames mpv rendered, frames GPUI presented, frame copy cost, and the process's CPU use.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;
use player::software::{RenderStats, SoftwarePlayer, VideoFrame};
use tokio::sync::watch;

struct InlineVideo {
    player: SoftwarePlayer,
    image: Option<Arc<RenderImage>>,
    presented: u64,
    paused: bool,
    stats: SharedString,
    _tasks: Vec<Task<()>>,
}

impl InlineVideo {
    fn new(
        player: SoftwarePlayer,
        mut frames: watch::Receiver<Option<Arc<VideoFrame>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let present = cx.spawn_in(window, async move |this, cx| {
            while frames.changed().await.is_ok() {
                let Some(frame) = frames.borrow_and_update().clone() else { continue };
                let Some(buffer) = image::RgbaImage::from_raw(frame.width, frame.height, frame.pixels.clone()) else {
                    continue;
                };
                let image = Arc::new(RenderImage::new(smallvec::smallvec![image::Frame::new(buffer)]));
                let updated = this.update_in(cx, |this, window, cx| {
                    // Each frame is a new texture; free the previous one from the atlas.
                    if let Some(old) = this.image.replace(image) {
                        let _ = window.drop_image(old);
                    }
                    this.presented += 1;
                    cx.notify();
                });
                if updated.is_err() {
                    break;
                }
            }
        });

        let report = cx.spawn(async move |this, cx| {
            let mut sampler = Sampler::new();
            loop {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                let Ok(()) = this.update(cx, |this, cx| {
                    this.stats = sampler.sample(this.player.stats(), this.presented).into();
                    println!("{}", this.stats);
                    cx.notify();
                }) else {
                    break;
                };
            }
        });

        Self {
            player,
            image: None,
            presented: 0,
            paused: false,
            stats: "measuring…".into(),
            _tasks: vec![present, report],
        }
    }

    fn command(&self, args: &[&str]) {
        if let Err(err) = self.player.command(args) {
            eprintln!("mpv: {err}");
        }
    }
}

impl Render for InlineVideo {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            .bg(black())
            .children(
                self.image
                    .clone()
                    .map(|image| img(ImageSource::Render(image)).size_full().object_fit(ObjectFit::Contain)),
            )
            .child(
                div()
                    .absolute()
                    .top_2()
                    .left_2()
                    .p_2()
                    .rounded_md()
                    .bg(black().opacity(0.6))
                    .text_color(white())
                    .text_sm()
                    .child(self.stats.clone()),
            )
            .child(
                v_flex().absolute().bottom_4().left_0().right_0().items_center().child(
                    h_flex()
                        .gap_2()
                        .p_2()
                        .rounded_md()
                        .bg(black().opacity(0.6))
                        .child(Button::new("back").ghost().label("-10s").on_click(
                            cx.listener(|this, _, _, _| this.command(&["seek", "-10", "relative"])),
                        ))
                        .child(
                            Button::new("pause")
                                .primary()
                                .label(if self.paused { "Play" } else { "Pause" })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.paused = !this.paused;
                                    let value = if this.paused { "yes" } else { "no" };
                                    if let Err(err) = this.player.set_property("pause", value) {
                                        eprintln!("mpv: {err}");
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(Button::new("forward").ghost().label("+10s").on_click(
                            cx.listener(|this, _, _, _| this.command(&["seek", "10", "relative"])),
                        )),
                ),
            )
    }
}

/// Rates over the last interval, plus process CPU time from `/proc` on Linux.
struct Sampler {
    started: Instant,
    at: Instant,
    rendered: u64,
    presented: u64,
    copy_time: Duration,
    cpu: Option<Duration>,
}

impl Sampler {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            at: Instant::now(),
            rendered: 0,
            presented: 0,
            copy_time: Duration::ZERO,
            cpu: process_cpu_time(),
        }
    }

    fn sample(&mut self, stats: RenderStats, presented: u64) -> String {
        let elapsed = self.at.elapsed().as_secs_f64();
        let rendered = stats.frames - self.rendered;
        let copy_ms = (stats.copy_time - self.copy_time).as_secs_f64() * 1000. / rendered.max(1) as f64;
        let cpu = process_cpu_time();
        let cpu_pct = match (cpu, self.cpu) {
            (Some(now), Some(before)) => format!("{:.0}%", (now - before).as_secs_f64() / elapsed * 100.),
            _ => "n/a".into(),
        };
        let line = format!(
            "mpv rendered {:.1} fps (copy {copy_ms:.1} ms/frame) · GPUI presented {:.1} fps · CPU {cpu_pct} of one core",
            rendered as f64 / elapsed,
            (presented - self.presented) as f64 / elapsed,
        );
        if std::env::var_os("SPIKE_THREADS").is_some() {
            let groups = cpu_by_group();
            let breakdown: Vec<String> = groups
                .iter()
                .map(|(group, time)| format!("{group} {:.0}%", time.as_secs_f64() / self.started.elapsed().as_secs_f64() * 100.))
                .collect();
            println!("  since start: {}", breakdown.join(", "));
        }
        *self = Self {
            started: self.started,
            at: Instant::now(),
            rendered: stats.frames,
            presented,
            copy_time: stats.copy_time,
            cpu,
        };
        line
    }
}

/// CPU time per thread group, from `/proc/self/task` on Linux: where the time goes.
fn cpu_by_group() -> std::collections::BTreeMap<&'static str, Duration> {
    let mut groups = std::collections::BTreeMap::new();
    let Ok(tasks) = std::fs::read_dir("/proc/self/task") else { return groups };
    for task in tasks.flatten() {
        let Ok(stat) = std::fs::read_to_string(task.path().join("stat")) else { continue };
        let Some((head, rest)) = stat.rsplit_once(')') else { continue };
        let name = head.split_once('(').map(|(_, n)| n).unwrap_or("");
        let fields: Vec<&str> = rest.split_whitespace().collect();
        let ticks: u64 = fields.get(11).and_then(|f| f.parse().ok()).unwrap_or(0)
            + fields.get(12).and_then(|f| f.parse().ok()).unwrap_or(0);
        let group = if name.starts_with("inline_video") {
            "GPUI main thread"
        } else if name.starts_with("llvmpipe") {
            "llvmpipe (software GPU)"
        } else if name == "mpv-render" {
            "mpv render + copy"
        } else if name.starts_with("mpv") || name.starts_with("av:") || name.starts_with("ffmpeg") || name.starts_with("vo") || name.starts_with("ao") || name.starts_with("demux") {
            "mpv decode/audio/demux"
        } else {
            "other"
        };
        *groups.entry(group).or_default() += Duration::from_millis(ticks * 10);
    }
    groups
}

/// User + system CPU time of this process (all threads, including mpv's).
fn process_cpu_time() -> Option<Duration> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    // Fields after the parenthesised command name; utime and stime are fields 14 and 15.
    let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    let ticks: u64 = fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?;
    // Linux reports in clock ticks, which are 1/100 s on all mainstream configurations.
    Some(Duration::from_millis(ticks * 10))
}

fn main() {
    let mut args = std::env::args().skip(1);
    let url = args.next().expect("usage: inline_video <url> [header] [WxH]");
    let mut headers = Vec::new();
    let mut max_size = (1280, 720);
    for arg in args {
        match arg.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))) {
            Some(size) => max_size = size,
            None => headers.push(arg),
        }
    }

    let (player, frames) = match SoftwarePlayer::new(&url, &headers, max_size) {
        Ok(started) => started,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };

    gpui_kit::application().with_assets(gpui_kit::assets::Assets).run(move |cx| {
        gpui_kit::init(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1280.), px(720.)), cx)),
            titlebar: Some(TitlebarOptions {
                title: Some("inline video spike".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| InlineVideo::new(player, frames, window, cx))
        })
        .expect("failed to open window");
    });
}
