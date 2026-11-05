//! The account's downloads, one row each in the order they were asked for: the item's picture,
//! its title, where the download stands (waiting, how far along with a bar, its size once it's
//! here, or why it failed), and round buttons at the right end for what can be done about it.
//! Clicking a row opens the item's page, while online. Offline (`Services::online`) it's the
//! shell's only page.
//!
//! It reads [`Downloads`] each frame and asks the server for nothing, so it shows the same
//! whatever the connection is doing.

use egui::accesskit::Role;
use egui::{Rect, RichText, Sense, TextWrapMode, Ui, WidgetInfo, WidgetText, pos2, vec2};
use egui_material_icons::MaterialIcon;
use egui_material_icons::icons::{ICON_CLOSE, ICON_DELETE, ICON_DOWNLOAD, ICON_PLAY_ARROW, ICON_REFRESH};
use jellyfin::models::{BaseItemDto, BaseItemKind};

use crate::downloads::{self, DownloadState, Offer};
use crate::item_menu::resume_point;
use crate::layout;
use crate::nav::{NavRequest, Route};
use crate::playback;
use crate::theme::{self, text};
use crate::views::{Page, PageCx};
use crate::widgets::menu::MenuAction;
use crate::widgets::round_button::RoundButton;
use crate::widgets::{backdrop, image, state};

/// A row's height, and the room round its picture.
const ROW_H: f32 = 88.;
const PAD: f32 = 8.;
const ROW_GAP: f32 = 8.;
/// The picture's height; an episode's still is 16:9, anything else a 2:3 poster.
const IMAGE_H: f32 = ROW_H - 2. * PAD;
/// The round buttons at a row's right end.
const BUTTON: f32 = 36.;
const BUTTON_ICON: f32 = 20.;
const BUTTON_GAP: f32 = 4.;
/// The widest the list gets, so a row's buttons stay near its title on a wide window.
const MAX_W: f32 = 880.;

#[derive(Default)]
pub struct DownloadsPage;

/// What a row asked for this frame.
enum Asked {
    Open(BaseItemDto),
    Play(BaseItemDto),
    Download(Offer, BaseItemDto),
}

impl Page for DownloadsPage {
    fn title(&self) -> &str {
        "Downloads"
    }

    fn show(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        let entries: Vec<(BaseItemDto, DownloadState)> = cx
            .services
            .downloads
            .entries()
            .map(|(item, state)| (item.clone(), state.clone()))
            .collect();
        if entries.is_empty() {
            return state::icon_message(
                ui,
                ICON_DOWNLOAD,
                "Nothing downloaded yet. Download a movie or an episode from its page or its menu.",
            );
        }
        let side = layout::side(ui.ctx());
        let mut asked = None;
        let area = egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui::Frame::NONE
                .inner_margin(egui::Margin {
                    left: side as i8,
                    right: side as i8,
                    top: 8,
                    bottom: 16,
                })
                .show(ui, |ui| {
                    let width = ui.available_width().min(MAX_W);
                    ui.spacing_mut().item_spacing.y = ROW_GAP;
                    ui.label(
                        RichText::new(summary(&entries))
                            .font(text::font(ui, "sm"))
                            .color(theme::tokens_of(ui).ink_muted),
                    );
                    for (item, state) in &entries {
                        let poster = cx.services.downloads.primary_image_path(&item.id);
                        let images = &mut cx.services.images;
                        if let Some(row_asked) = row(ui, images, item, poster.as_deref(), state, width) {
                            asked = Some(row_asked);
                        }
                    }
                });
        });
        backdrop::scroll_edge(ui, &area);
        match asked {
            // An item's page needs the server.
            Some(Asked::Open(_)) if !cx.services.online => {}
            Some(Asked::Open(item)) => cx.nav.push(NavRequest::Push(Route::for_item(&item))),
            Some(Asked::Play(item)) => {
                cx.services.carry_out(
                    MenuAction::Play {
                        start: resume_point(&item),
                    },
                    &item,
                );
            }
            Some(Asked::Download(offer, item)) => cx.services.download(offer, &item),
            None => {}
        }
    }
}

/// "3 downloaded, 12.4 GB · 1 downloading": what's here and what's still to come.
fn summary(entries: &[(BaseItemDto, DownloadState)]) -> String {
    let (mut done, mut size, mut coming, mut failed) = (0, 0, 0, 0);
    for (_, state) in entries {
        match state {
            DownloadState::Done { size: bytes } => {
                done += 1;
                size += bytes;
            }
            DownloadState::Failed(_) => failed += 1,
            _ => coming += 1,
        }
    }
    let mut parts = Vec::new();
    if done > 0 {
        parts.push(format!("{done} downloaded, {}", downloads::bytes(size)));
    }
    if coming > 0 {
        parts.push(format!("{coming} to come"));
    }
    if failed > 0 {
        parts.push(format!("{failed} failed"));
    }
    parts.join(" · ")
}

/// Where a download stands, for the line under its title, and how far along its bar is.
fn status(state: &DownloadState) -> (String, Option<f32>) {
    match state {
        DownloadState::Queued => ("Waiting to download".to_owned(), Some(0.)),
        DownloadState::Paused => ("Paused".to_owned(), None),
        DownloadState::Downloading { done, total } => match total.filter(|&total| total > 0) {
            Some(total) => {
                let share = *done as f32 / total as f32;
                (
                    format!(
                        "{} of {} · {:.0}%",
                        downloads::bytes(*done),
                        downloads::bytes(total),
                        share * 100.
                    ),
                    Some(share),
                )
            }
            None => (format!("{} so far", downloads::bytes(*done)), Some(0.)),
        },
        DownloadState::Done { size } => (downloads::bytes(*size), None),
        DownloadState::Failed(message) => (format!("Failed: {message}"), None),
    }
}

/// The round buttons a download has, rightmost last: playing it once it's here, and stopping,
/// retrying or deleting it.
fn buttons(state: &DownloadState) -> Vec<(MaterialIcon, &'static str, Option<Offer>)> {
    match state {
        DownloadState::Done { .. } => vec![
            (ICON_PLAY_ARROW, "Play", None),
            (ICON_DELETE.outlined(), "Remove download", Some(Offer::Remove)),
        ],
        DownloadState::Failed(_) => vec![
            (ICON_REFRESH, "Retry download", Some(Offer::Retry)),
            (ICON_CLOSE, "Remove from downloads", Some(Offer::Cancel)),
        ],
        _ => vec![(ICON_CLOSE, "Cancel download", Some(Offer::Cancel))],
    }
}

/// One download as a sunk row `width` wide.
fn row(
    ui: &mut Ui,
    images: &mut crate::images::ImageCache,
    item: &BaseItemDto,
    poster: Option<&std::path::Path>,
    state: &DownloadState,
    width: f32,
) -> Option<Asked> {
    let tokens = theme::tokens_of(ui);
    let title = playback::title(item);
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, &title));
    let hovered = response.hovered();
    ui.painter().rect_filled(
        rect,
        tokens.radius_control,
        if hovered { tokens.inset_hover } else { tokens.inset },
    );

    let image_w = if item.kind == BaseItemKind::Episode {
        IMAGE_H * 16. / 9.
    } else {
        IMAGE_H * 2. / 3.
    };
    let picture = Rect::from_min_size(rect.min + vec2(PAD, PAD), vec2(image_w, IMAGE_H));
    // The poster saved with the download, which needs no server; the server's while it's on
    // its way.
    match poster.and_then(|path| images.get_file(path)) {
        Some(texture) => image::texture(ui, picture, &texture, tokens.radius_control),
        None => {
            let request = item.primary_image().map(|req| req.fill_width((2. * image_w) as u32));
            image::show(ui, images, picture, request.as_ref(), Some(item), tokens.radius_control);
        }
    }

    let buttons = buttons(state);
    let buttons_w = buttons.len() as f32 * (BUTTON + BUTTON_GAP);
    let text_left = picture.right() + 12.;
    let text_w = (rect.right() - PAD - buttons_w - 8. - text_left).max(0.);
    let line = |text: String, size: &str, color| {
        WidgetText::from(RichText::new(text).font(text::font(ui, size)).color(color)).into_galley(
            ui,
            Some(TextWrapMode::Truncate),
            text_w,
            egui::TextStyle::Body,
        )
    };
    let (status, progress) = status(state);
    let failed = matches!(state, DownloadState::Failed(_));
    let heading = line(title.clone(), "base", tokens.ink);
    let detail = line(status, "sm", if failed { tokens.danger } else { tokens.ink_muted });
    let block_h = heading.size().y + 4. + detail.size().y + if progress.is_some() { 10. } else { 0. };
    let mut y = rect.center().y - block_h / 2.;
    ui.painter().galley(pos2(text_left, y), heading.clone(), tokens.ink);
    y += heading.size().y + 4.;
    ui.painter()
        .galley(pos2(text_left, y), detail.clone(), tokens.ink_muted);
    y += detail.size().y + 6.;
    if let Some(done) = progress {
        let track = Rect::from_min_size(pos2(text_left, y), vec2(text_w, 4.));
        ui.painter().rect_filled(track, 2., tokens.surface_border);
        let filled = Rect::from_min_size(track.min, vec2(track.width() * done.clamp(0., 1.), 4.));
        ui.painter().rect_filled(filled, 2., tokens.brand);
    }

    let mut asked = None;
    let mut center = pos2(rect.right() - PAD - BUTTON / 2., rect.center().y);
    for (icon, label, offer) in buttons.into_iter().rev() {
        let clicked = RoundButton::icon(icon, label)
            .size(BUTTON, BUTTON_ICON)
            .tooltip(label)
            .show_at(ui, center, response.id.with(label))
            .clicked();
        if clicked {
            asked = Some(match offer {
                Some(offer) => Asked::Download(offer, item.clone()),
                None => Asked::Play(item.clone()),
            });
        }
        center.x -= BUTTON + BUTTON_GAP;
    }
    if asked.is_none() && response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        asked = Some(Asked::Open(item.clone()));
    }
    asked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(state: DownloadState) -> (BaseItemDto, DownloadState) {
        (BaseItemDto::default(), state)
    }

    #[test]
    fn the_summary_counts_what_is_here_and_what_is_to_come() {
        let entries = [
            entry(DownloadState::Done { size: 4_000_000_000 }),
            entry(DownloadState::Done { size: 300_000_000 }),
            entry(DownloadState::Downloading {
                done: 1,
                total: Some(2),
            }),
            entry(DownloadState::Queued),
            entry(DownloadState::Failed("No room.".to_owned())),
        ];
        assert_eq!(summary(&entries), "2 downloaded, 4.3 GB · 2 to come · 1 failed");
        assert_eq!(summary(&entries[2..4]), "2 to come");
    }

    #[test]
    fn a_transfer_shows_how_far_along_it_is() {
        let (line, bar) = status(&DownloadState::Downloading {
            done: 500_000_000,
            total: Some(2_000_000_000),
        });
        assert_eq!(line, "500.0 MB of 2.0 GB · 25%");
        assert_eq!(bar, Some(0.25));
    }
}
