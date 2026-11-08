//! The shell's parts of casting (docs/chromecast.md, *UI*): the cast panel that hangs from the
//! top bar's cast button, and the now-casting bar that floats over the pages while the player is
//! closed and a cast goes on. The player's own casting view is `views::player`'s.

use egui::accesskit::Role;
use egui::{Id, Key, Modifiers, Rect, Sense, Ui, WidgetInfo, pos2, vec2};
use egui_material_icons::icons::{ICON_CAST_CONNECTED, ICON_PAUSE, ICON_PLAY_ARROW, ICON_SPEAKER_GROUP, ICON_TV};
use jellyfin::ImageRequest;
use jellyfin::models::{BaseItemDto, BaseItemKind, ImageType};

use crate::cast::{Cast, Casting};
use crate::playback::UiCommand;
use crate::services::Services;
use crate::theme::{self, text};
use crate::widgets::button::{self, Focus, State};
use crate::widgets::focus::{self, Place};
use crate::widgets::list_row::{self, Leading, ListRow};
use crate::widgets::round_button::{self, RoundButton};
use crate::widgets::text::timestamp;
use crate::widgets::{dialog, image, popup, state, surface};

/// The cast panel's width.
pub const PANEL_W: f32 = 340.;
/// The overline's height, as the SyncPlay panel's sections are.
const SECTION_H: f32 = 30.;

/// The now-casting bar on the desktop: at most this wide, this tall, this far up from the
/// window's bottom, with a popup's frame this round.
const BAR_MAX_W: f32 = 640.;
const BAR_H: f32 = 68.;
const BAR_LIFT: f32 = 24.;
const BAR_RADIUS: u8 = 18;
/// On a phone: the window's width less this on each side, and this tall.
const PHONE_SIDE: f32 = 8.;
const PHONE_BAR_H: f32 = 64.;
/// The progress along the bar's bottom edge.
const PROGRESS_H: f32 = 3.;

/// A choice in the cast panel, in the order it lists them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CastEntry {
    /// A device on the network, by id: casts to it (or, the one in use, does nothing).
    Device(String),
    Disconnect,
    Stop,
}

/// What the panel lists, in the order the keyboard walks it.
pub fn entries(cast: &Cast) -> Vec<CastEntry> {
    let mut entries: Vec<CastEntry> = cast
        .devices
        .iter()
        .map(|device| CastEntry::Device(device.id.clone()))
        .collect();
    if cast.is_casting() {
        entries.push(CastEntry::Disconnect);
        entries.push(CastEntry::Stop);
    }
    entries
}

/// Does what an entry says.
pub fn choose(entry: CastEntry, services: &mut Services) {
    match entry {
        CastEntry::Device(id) => services.start_cast(&id),
        CastEntry::Disconnect => services.disconnect_cast(),
        CastEntry::Stop => services.stop_cast(),
    }
}

/// The panel's body: its heading, the devices and, while casting, *Disconnect* and *Stop
/// casting*. Returns the entry a click picked. `highlight` is the keyboard's.
pub fn panel_content(ui: &mut Ui, services: &Services, highlight: Option<&CastEntry>) -> Option<CastEntry> {
    let cast = &services.cast;
    let tokens = theme::tokens_of(ui);
    let width = ui.available_width();
    let radius = tokens.radius_control as f32;
    let casting_on = cast.session.as_ref().map(|session| session.device.id.as_str());
    let reconnecting = cast
        .session
        .as_ref()
        .is_some_and(|session| session.reconnecting.is_some());
    let status = match casting_on {
        Some(_) if reconnecting => Some(("Reconnecting", tokens.warning)),
        Some(_) => Some(("Casting", tokens.success)),
        None => None,
    };
    popup::panel_heading(
        ui,
        "Cast to a TV",
        status,
        "Play on a Chromecast or Google TV on this network. Your TV plays it; this app is the remote.",
    );
    ui.add_space(surface::GAP_GROUP);
    let mut chosen = None;
    if cast.devices.is_empty() {
        state::icon_message(ui, ICON_TV, "Looking for TVs on this network…");
    } else {
        popup::panel_section(ui, "On this network", SECTION_H, |_| {});
        list_row::block(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.;
            for device in &cast.devices {
                let entry = CastEntry::Device(device.id.clone());
                let in_use = casting_on == Some(device.id.as_str());
                let leading = if in_use {
                    Leading::Icon(ICON_CAST_CONNECTED, tokens.brand)
                } else if device.is_group() {
                    Leading::Icon(ICON_SPEAKER_GROUP, tokens.ink_muted)
                } else {
                    Leading::Icon(ICON_TV, tokens.ink_muted)
                };
                // mDNS doesn't say how many TVs a group has.
                let detail = if in_use && reconnecting {
                    "Reconnecting"
                } else if in_use {
                    "Casting"
                } else if device.runs_receiver() {
                    // Its own ad says the receiver is up: what it plays can be picked up again.
                    "Jellyfin is running"
                } else if device.is_group() {
                    "Group"
                } else {
                    device.model.as_str()
                };
                let label = if in_use {
                    format!("{}, casting", device.name)
                } else if device.runs_receiver() {
                    format!("Reconnect to {}", device.name)
                } else {
                    format!("Cast to {}", device.name)
                };
                let ringed = highlight == Some(&entry);
                let row = ListRow::new(&device.name)
                    .leading(leading)
                    .detail(Some(detail))
                    .highlighted(in_use || ringed)
                    .ringed(ringed)
                    .role(Role::MenuItem)
                    .label(&label)
                    .show(ui);
                if row.row.clicked() && !in_use {
                    chosen = Some(entry);
                }
            }
        });
    }
    if cast.is_casting() {
        ui.add_space(surface::GAP_GROUP);
        let disconnect = button::quiet(ui, "Disconnect", width, State::Ready, Focus::Skip);
        if highlight == Some(&CastEntry::Disconnect) {
            focus::ring(ui, disconnect.rect, radius, Place::Outside);
        }
        if disconnect.clicked() {
            chosen = Some(CastEntry::Disconnect);
        }
        ui.add_space(surface::GAP_ITEM);
        let stop = button::quiet_danger(ui, "Stop casting", width, State::Ready, Focus::Skip);
        if highlight == Some(&CastEntry::Stop) {
            focus::ring(ui, stop.rect, radius, Place::Outside);
        }
        if stop.clicked() {
            chosen = Some(CastEntry::Stop);
        }
    }
    chosen
}

/// Whether the now-casting bar shows: something casts and the player is closed (the caller's
/// part).
pub fn has_bar(services: &Services) -> bool {
    services
        .cast
        .casting()
        .is_some_and(|casting| casting.item_id.is_some() || casting.item.is_some())
}

/// What a SyncPlay choice waits on while a cast is going: casting and SyncPlay don't mix.
#[derive(Clone, Debug)]
pub enum GroupAction {
    Join(String),
    /// A new group, by its name.
    Create(String),
}

/// The question before joining or starting a SyncPlay group while casting: the cast stops first.
pub struct StopCastFirst {
    action: GroupAction,
}

impl StopCastFirst {
    pub fn new(action: GroupAction) -> Self {
        Self { action }
    }

    /// Draws the question: `false` once it should be dropped (answered, or closed).
    pub fn show(&mut self, services: &mut Services, ctx: &egui::Context) -> bool {
        let mut confirmed = false;
        let device = services
            .cast
            .session
            .as_ref()
            .map_or_else(String::new, |session| session.device.name.clone());
        let title = match &self.action {
            GroupAction::Join(_) => "Stop casting to join?",
            GroupAction::Create(_) => "Stop casting to start a group?",
        };
        let note = format!(
            "A SyncPlay group plays here, in step with the others, so casting to {device} stops. \
             What it plays stops too."
        );
        let modal = dialog::show(ctx, Id::unique("stop_cast_first"), 460., title, None, |ui| {
            ui.label(note);
            dialog::buttons(ui, |ui| {
                if dialog::danger(ui, "Stop casting", State::Ready).clicked() {
                    confirmed = true;
                }
                dialog::cancel(ui);
            });
            if ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter)) {
                confirmed = true;
            }
        });
        if confirmed {
            services.stop_cast();
            match self.action.clone() {
                GroupAction::Join(id) => services.syncplay.join(id),
                GroupAction::Create(name) => services.syncplay.create(name),
            }
            return false;
        }
        !modal.should_close()
    }
}

/// What the bar did.
pub enum BarClick {
    /// The bar itself: back to the player.
    Open,
}

/// The now-casting bar, floating over the bottom of the page: on the desktop at most
/// [`BAR_MAX_W`] wide, [`BAR_LIFT`] up from the bottom; on a phone (`compact`) across the window
/// less [`PHONE_SIDE`] a side, its bottom at `floor` (the top of the bottom bar). Pause works in
/// place; anywhere else opens the player.
pub fn bar(ui: &mut Ui, services: &mut Services, compact: bool, floor: f32) -> Option<BarClick> {
    let session = services.cast.session.as_ref()?;
    // Connected with nothing playing on the TV: there's nothing to show or go back to.
    if session.casting.item_id.is_none() && session.casting.item.is_none() {
        return None;
    }
    let casting = session.casting.clone();
    let device = session.device.name.clone();
    let reconnecting = session.reconnecting.is_some();
    let window = ui.ctx().content_rect();
    let rect = if compact {
        Rect::from_min_max(
            pos2(window.left() + PHONE_SIDE, floor - PHONE_SIDE - PHONE_BAR_H),
            pos2(window.right() - PHONE_SIDE, floor - PHONE_SIDE),
        )
    } else {
        let width = BAR_MAX_W.min(window.width() - 2. * BAR_LIFT);
        Rect::from_center_size(
            pos2(window.center().x, window.bottom() - BAR_LIFT - BAR_H / 2.),
            vec2(width, BAR_H),
        )
    };
    let mut clicked = None;
    egui::Area::new(Id::unique("now_casting_bar"))
        .order(egui::Order::Middle)
        .fixed_pos(rect.min)
        .show(ui.ctx(), |ui| {
            clicked = draw_bar(ui, services, rect, &casting, &device, reconnecting, compact);
        });
    clicked
}

fn draw_bar(
    ui: &mut Ui,
    services: &mut Services,
    rect: Rect,
    casting: &Casting,
    device: &str,
    reconnecting: bool,
    compact: bool,
) -> Option<BarClick> {
    let tokens = theme::tokens_of(ui);
    ui.set_min_size(rect.size());
    let painter = ui.painter().clone();
    let frame = popup::frame(tokens).corner_radius(BAR_RADIUS);
    painter.add(frame.paint(rect));
    let response = ui.interact(rect, Id::unique("now_casting_bar_open"), Sense::CLICK);
    let name = bar_title(casting);
    let label = if reconnecting {
        format!("{name}, reconnecting to {device}")
    } else {
        format!("{name}, on {device}")
    };
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, label.clone()));
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    // The thumbnail, 16:9, inside the bar's padding.
    let pad = 10.;
    let thumb_h = rect.height() - 2. * pad;
    let thumb = Rect::from_min_size(rect.min + vec2(pad, pad), vec2(thumb_h * 16. / 9., thumb_h));
    let request = casting.item.as_ref().and_then(thumbnail);
    image::show(
        ui,
        &mut services.images,
        thumb,
        request.as_ref(),
        casting.item.as_ref(),
        8,
    );

    // The pause button at the right end, and the time before it on the desktop.
    let button_size = 40.;
    let button_center = pos2(rect.right() - pad - button_size / 2., rect.center().y);
    let (icon, tip) = if casting.paused {
        (ICON_PLAY_ARROW, "Play")
    } else {
        (ICON_PAUSE, "Pause")
    };
    let pause = RoundButton::icon(icon, tip)
        .size(button_size, 24.)
        .focus(Focus::Skip)
        .show_at(ui, button_center, Id::unique("now_casting_pause"));
    let mut right = button_center.x - button_size / 2. - 8.;
    if !compact && let Some(duration) = casting.duration {
        let time = format!("{} / {}", timestamp(casting.position()), timestamp(duration));
        let galley = painter.layout_no_wrap(time, text::font(ui, "sm"), tokens.ink_muted);
        let pos = pos2(right - galley.size().x, rect.center().y - galley.size().y / 2.);
        right = pos.x - 12.;
        painter.galley(pos, galley, tokens.ink_muted);
    }

    // The name over the cast mark and where it plays.
    let left = thumb.right() + 12.;
    let text_w = (right - left).max(0.);
    let name_galley = painter.layout(name, text::font(ui, "sm"), tokens.ink, text_w);
    let name_pos = pos2(left, rect.center().y - name_galley.size().y - 1.);
    painter
        .with_clip_rect(Rect::from_min_max(pos2(left, rect.top()), pos2(right, rect.bottom())))
        .galley(name_pos, name_galley, tokens.ink);
    let mark = 14.;
    round_button::glyph(
        ui,
        pos2(left + mark / 2., rect.center().y + 10.),
        ICON_CAST_CONNECTED,
        mark,
        tokens.brand,
    );
    let place = if reconnecting {
        format!("Reconnecting to {device}…")
    } else {
        match (&casting.series_name, compact) {
            (Some(series), false) => format!("{series} · on {device}"),
            _ => format!("on {device}"),
        }
    };
    let place_galley = painter.layout_no_wrap(place, text::font(ui, "xs"), tokens.ink_muted);
    painter
        .with_clip_rect(Rect::from_min_max(pos2(left, rect.top()), pos2(right, rect.bottom())))
        .galley(
            pos2(left + mark + 4., rect.center().y + 10. - place_galley.size().y / 2.),
            place_galley,
            tokens.ink_muted,
        );

    // The progress along the bottom edge, inside the rounded corners.
    if let Some(duration) = casting.duration.filter(|duration| !duration.is_zero()) {
        let track = Rect::from_min_max(
            pos2(rect.left() + BAR_RADIUS as f32, rect.bottom() - PROGRESS_H - 1.),
            pos2(rect.right() - BAR_RADIUS as f32, rect.bottom() - 1.),
        );
        painter.rect_filled(track, 2, tokens.inset);
        let done = (casting.position().as_secs_f32() / duration.as_secs_f32()).clamp(0., 1.);
        let filled = Rect::from_min_size(track.min, vec2(track.width() * done, track.height()));
        painter.rect_filled(filled, 2, tokens.brand);
        if casting.playing && !casting.paused {
            // The position moves between the receiver's reports.
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    if pause.clicked() {
        services.control(UiCommand::TogglePause);
        return None;
    }
    response.clicked().then_some(BarClick::Open)
}

/// The bar's title: the episode's or movie's name.
fn bar_title(casting: &Casting) -> String {
    casting
        .item
        .as_ref()
        .map_or_else(|| casting.name.clone(), |item| item.name.clone())
}

/// A 16:9 picture of the item: an episode's own still, otherwise a thumb or the backdrop.
fn thumbnail(item: &BaseItemDto) -> Option<ImageRequest> {
    if item.kind == BaseItemKind::Episode
        && let Some(still) = item.image(ImageType::Primary)
    {
        return Some(still);
    }
    item.image(ImageType::Thumb)
        .or_else(|| {
            let parent = item.parent_thumb_item_id.as_ref()?;
            let tag = item.parent_thumb_image_tag.clone()?;
            Some(ImageRequest::new(parent, ImageType::Thumb, Some(tag)))
        })
        .or_else(|| item.backdrop_image())
}

/// What the cast button says, as its tooltip and to a screen reader: in a SyncPlay group, why
/// it's off.
pub fn button_label(services: &Services) -> &'static str {
    if services.is_casting() {
        "Casting (cast to a TV)"
    } else if services.in_group() {
        "Cast to a TV (off in a SyncPlay group)"
    } else {
        "Cast to a TV"
    }
}
