// The component gallery: every shared widget in `crates/app/src/widgets/`, in each of its states,
// in both themes, on the window's backdrop the way a page draws it. One section per family of
// widgets, one mockup per part of a family, and a caption naming the call each widget comes from.
//
// It's the catalogue `docs/DESIGN.md` points to: a new widget, or a new state of one, is added
// here, so a change that breaks one of them fails the mockup build. Render it with the `ui-mockup`
// skill's `render.py`, passing this file and `--no-open`; the command is at the top of the source,
// under the constants.
//
// The modules that don't draw (`menu`, `text`, `image`, `video`) aren't here, and neither are the
// states only a live pointer shows (a card's play button) or the ones that exist inside a popup or
// a modal: those are drawn as their parts (`popup::frame` and `popup::row`, a dialog's frame).

use egui::{Align, Color32, CornerRadius, Layout, Margin, Sense, Ui, UiBuilder, pos2, vec2};
use egui_material_icons::icons::{
    ICON_ARROW_BACK, ICON_AUDIOTRACK, ICON_CLOSED_CAPTION, ICON_DELETE, ICON_FAVORITE, ICON_MORE_VERT, ICON_PLAY_ARROW,
    ICON_SEARCH, ICON_SKIP_NEXT, ICON_SUBTITLES, ICON_TUNE, ICON_VISIBILITY, ICON_WATCH_LATER,
};
use jellyfin::Client;
use jellyfin::models::{BaseItemDto, BaseItemPerson, ChapterInfo, QueryResult};

use crate::images::ImageCache;
use crate::session;
use crate::theme::{self, over_video};
use crate::ui_tests::FixtureServer;
use crate::ui_tests::fixture_json;
use crate::ui_tests::mockups::{Mockup, Themes};
use crate::views::{Page, PageCx};
use crate::widgets::{
    avatar, avatar_row, backdrop, badge, button, card, code, countdown_pill, dialog, field, focus, image, label,
    library_tile, list_row, mascot, media_row, pill, popup, prompt, round_button, segmented, select, shelf, state,
    stepper, surface, tabs, toggle,
};

/// The width every mockup is drawn at, so the gallery reads as one page.
const WIDTH: f32 = 1000.;

/// The inset the gallery's content keeps from the mockup's edges, as a page keeps its margins.
const MARGIN: i8 = 24;

// Rendering it, from the repository root: the skill's script, this file, and `--no-open` so it
// only writes the pictures.
//
//     python3 -I .agents/skills/ui-mockup/render.py crates/app/src/ui_tests/gallery.rs --no-open

// ## Text
// The text styles beyond plain body text, and the label a form puts above its field.

/// The text styles
/// `label::heading`, `title`, `muted`, `note` and `overline` over the body text the pages draw
/// themselves, and `field::label`.
#[test]
fn mockup_text_styles() {
    Mockup::new("text.styles")
        .themes(Themes::Both)
        .size(vec2(WIDTH, 390.))
        .show(
            || (),
            |ui, _| {
                gallery(ui, |ui| {
                    named(ui, "body (the default text)", |ui| {
                        ui.label("Plain body text, which the page draws itself.");
                    });
                    named(ui, "label::heading", |ui| {
                        ui.label(label::heading(ui, "Continue watching"));
                    });
                    named(ui, "label::title", |ui| {
                        ui.label(label::title(ui, "Audio"));
                    });
                    named(ui, "label::muted", |ui| {
                        ui.label(label::muted(ui, "Waiting for the server to answer…"));
                    });
                    named(ui, "label::note", |ui| {
                        ui.label(label::note(ui, "2 items, 1 watched"));
                    });
                    named(ui, "label::overline", |ui| {
                        ui.label(label::overline(ui, "Playback"));
                    });
                    named(ui, "field::label", |ui| field::label(ui, "Server address"));
                });
            },
        );
}

// ## Buttons
// The cozy look's buttons in every state they have, the small actions that aren't boxes, the
// round icon buttons of the top bar, and a dialog's buttons.

/// `button::brand`, `quiet`, `quiet_danger` and `danger`
/// Each in its three states: ready, disabled, and loading, which is disabled with a spinner at
/// its right end and its label where it was.
#[test]
fn mockup_buttons_actions() {
    Mockup::new("buttons.actions")
        .themes(Themes::Both)
        .size(vec2(WIDTH, 250.))
        .show(
            || (),
            |ui, _| {
                gallery(ui, |ui| {
                    type Add = fn(&mut Ui, &str, f32, button::State, button::Focus) -> egui::Response;
                    let variants: [(&str, Add); 4] = [
                        ("button::brand", button::brand),
                        ("button::quiet", button::quiet),
                        ("button::quiet_danger", button::quiet_danger),
                        ("button::danger", button::danger),
                    ];
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 30.;
                        for (caption, add) in variants {
                            column(ui, 190., |ui| {
                                ui.label(call(ui, caption));
                                ui.add_space(4.);
                                shown(ui, "", |ui| {
                                    add(ui, "Save changes", 180., button::State::Ready, button::Focus::Skip)
                                });
                                shown(ui, "disabled", |ui| {
                                    add(ui, "Save changes", 180., button::State::Disabled, button::Focus::Skip);
                                });
                                shown(ui, "loading", |ui| {
                                    add(ui, "Save changes", 180., button::State::Loading, button::Focus::Skip);
                                });
                            });
                        }
                    });
                });
            },
        );
}

/// The buttons that aren't boxes
/// `subtle` and `link` in the text flow, `text` enabled and disabled, and the page's icon
/// actions: `brand_icon` with the keyboard's ring, and `quiet_icon` off, on and ringed.
#[test]
fn mockup_buttons_misc() {
    Mockup::new("buttons.misc")
        .themes(Themes::Both)
        .size(vec2(WIDTH, 225.))
        .show(
            || (),
            |ui, _| {
                gallery(ui, |ui| {
                    states(ui, "button::subtle, button::link and button::text", |ui| {
                        shown(ui, "subtle: an aside under a panel", |ui| {
                            button::subtle(ui, "Use another server");
                        });
                        shown(ui, "link: inside a line of text", |ui| {
                            button::link(ui, "All movies", "base");
                        });
                        shown(ui, "text", |ui| button::text(ui, "Clear filters", true));
                        shown(ui, "disabled", |ui| button::text(ui, "Clear filters", false));
                    });
                    states(ui, "button::brand_icon and button::quiet_icon", |ui| {
                        shown(ui, "", |ui| button::brand_icon(ui, ICON_PLAY_ARROW, "Play", false));
                        shown(ui, "ringed", |ui| {
                            button::brand_icon(ui, ICON_PLAY_ARROW, "Play", true);
                        });
                        shown(ui, "", |ui| {
                            button::quiet_icon(ui, ICON_FAVORITE, "Favorite", None, 1., false);
                        });
                        shown(ui, "on", |ui| {
                            button::quiet_icon(ui, ICON_FAVORITE, "Favorite", Some(true), 1., false);
                        });
                        shown(ui, "ringed", |ui| {
                            button::quiet_icon(ui, ICON_MORE_VERT, "More", None, 1., true);
                        });
                    });
                });
            },
        );
}

/// The round button
/// `RoundButton` with an icon, as a toggle that's on, ringed, disabled, and showing a person's
/// avatar.
#[test]
fn mockup_buttons_round() {
    Mockup::new("buttons.round")
        .themes(Themes::Both)
        .size(vec2(WIDTH, 135.))
        .show(
            || (),
            |ui, _| {
                gallery(ui, |ui| {
                    states(ui, "RoundButton", |ui| {
                        shown(ui, "an icon", |ui| {
                            round_button::RoundButton::icon(ICON_PLAY_ARROW, "Play").show(ui);
                        });
                        shown(ui, "on", |ui| {
                            round_button::RoundButton::icon(ICON_FAVORITE, "Favorite")
                                .on(true)
                                .show(ui);
                        });
                        shown(ui, "ringed", |ui| {
                            round_button::RoundButton::icon(ICON_MORE_VERT, "More")
                                .ringed(true)
                                .show(ui);
                        });
                        shown(ui, "disabled", |ui| {
                            round_button::RoundButton::icon(ICON_WATCH_LATER, "Watch later")
                                .enabled(false)
                                .show(ui);
                        });
                        shown(ui, "an avatar", |ui| {
                            round_button::RoundButton::avatar("dev", "dev").show(ui);
                        });
                    });
                });
            },
        );
}

/// The dialog's buttons
/// `dialog::main` with *Cancel*, and `dialog::danger` loading, in the row `dialog::buttons` lays
/// out from the right.
#[test]
fn mockup_buttons_dialog() {
    Mockup::new("buttons.dialog")
        .themes(Themes::Both)
        .size(vec2(WIDTH, 200.))
        .show(
            || (),
            |ui, _| {
                gallery(ui, |ui| {
                    named(ui, "dialog::buttons, dialog::main, dialog::cancel", |ui| {
                        column(ui, 460., |ui| {
                            dialog::buttons(ui, |ui| {
                                dialog::cancel(ui);
                                dialog::main(ui, "Save", button::State::Ready);
                            });
                            ui.add_space(theme::SPACE.sm);
                            dialog::buttons(ui, |ui| {
                                dialog::cancel(ui);
                                dialog::danger(ui, "Delete", button::State::Loading);
                            });
                        });
                    });
                });
            },
        );
}

// ## Fields and controls
// The text fields, the select, the switch and the keyboard's ring.

/// The text fields
/// `field::single_line` empty (with its hint) and filled, `with_trailing` with a password's eye,
/// and `with_leading` with a search box's magnifier, empty and filled.
#[test]
fn mockup_fields_text() {
    Mockup::new("fields.text")
        .themes(Themes::Both)
        .size(vec2(WIDTH, 370.))
        .show(Fields::default, |ui, state| {
            gallery(ui, |ui| {
                states(ui, "field::single_line and field::label", |ui| {
                    shown(ui, "empty, with a hint", |ui| {
                        column(ui, 280., |ui| {
                            field::label(ui, "Where to?");
                            ui.add(field::single_line(
                                ui,
                                ui.scope_id().with("empty"),
                                &mut state.empty,
                                "The server's address",
                            ));
                        });
                    });
                    shown(ui, "filled", |ui| {
                        column(ui, 280., |ui| {
                            field::label(ui, "Username");
                            ui.add(field::single_line(
                                ui,
                                ui.scope_id().with("filled"),
                                &mut state.filled,
                                "",
                            ));
                        });
                    });
                });
                states(ui, "field::with_trailing and field::trailing", |ui| {
                    shown(ui, "a password's eye", |ui| {
                        column(ui, 280., |ui| {
                            field::label(ui, "Password");
                            let edit = field::with_trailing(
                                ui,
                                ui.scope_id().with("password"),
                                &mut state.password,
                                "Password",
                            );
                            let field = ui.add(edit);
                            field::trailing(ui, &field, ICON_VISIBILITY, "Show password", true);
                        });
                    });
                    shown(ui, "disabled", |ui| {
                        column(ui, 280., |ui| {
                            field::label(ui, "Token");
                            let edit = field::with_trailing(ui, ui.scope_id().with("token"), &mut state.token, "Token");
                            let field = ui.add(edit);
                            field::trailing(ui, &field, ICON_VISIBILITY, "Show token", false);
                        });
                    });
                });
                states(ui, "field::with_leading and field::leading", |ui| {
                    shown(ui, "a search box, empty", |ui| {
                        column(ui, 280., |ui| {
                            let edit =
                                field::with_leading(ui, ui.scope_id().with("search"), &mut state.search, "Search");
                            let field = ui.add(edit);
                            field::leading(ui, &field, ICON_SEARCH);
                        });
                    });
                    shown(ui, "filled", |ui| {
                        column(ui, 280., |ui| {
                            let edit = field::with_leading(
                                ui,
                                ui.scope_id().with("search filled"),
                                &mut state.search_filled,
                                "Search",
                            );
                            let field = ui.add(edit);
                            field::leading(ui, &field, ICON_SEARCH);
                        });
                    });
                });
            });
        });
}

/// The select, the switch and the keyboard's ring
/// `select::button`, `toggle::switch` off and on, `focus::ring` outside and inside a control's
/// edge, and `toggle::row` off, on and ringed.
#[test]
fn mockup_fields_controls() {
    Mockup::new("fields.controls")
        .themes(Themes::Both)
        .size(vec2(WIDTH, 405.))
        .show(Controls::default, |ui, state| {
            gallery(ui, |ui| {
                states(ui, "select::button", |ui| {
                    shown(ui, "", |ui| {
                        select::button(
                            ui,
                            vec2(220., ui.spacing().interact_size.y),
                            "Recently added",
                            "Sort by",
                            false,
                        );
                    });
                    shown(ui, "a narrow one", |ui| {
                        select::button(ui, vec2(140., ui.spacing().interact_size.y), "A-Z", "Sort by", false);
                    });
                });
                states(ui, "toggle::switch", |ui| {
                    shown(ui, "", |ui| {
                        let mut off = state.on;
                        toggle::switch(ui, &mut off, "Autoplay");
                    });
                    shown(ui, "on", |ui| {
                        let mut on = true;
                        toggle::switch(ui, &mut on, "Autoplay");
                    });
                });
                states(ui, "focus::ring", |ui| {
                    shown(ui, "Outside", |ui| ring_sample(ui, focus::Place::Outside));
                    shown(ui, "Inside", |ui| ring_sample(ui, focus::Place::Inside));
                });
                states(ui, "toggle::row", |ui| {
                    shown(ui, "off", |ui| {
                        column(ui, 300., |ui| {
                            toggle::row(ui, false, "Autoplay next episode", button::Focus::Tab, false);
                        });
                    });
                    shown(ui, "on", |ui| {
                        column(ui, 300., |ui| {
                            toggle::row(ui, true, "Autoplay next episode", button::Focus::Tab, false);
                        });
                    });
                    shown(ui, "ringed", |ui| {
                        column(ui, 300., |ui| {
                            toggle::row(ui, true, "Autoplay next episode", button::Focus::Skip, true);
                        });
                    });
                });
            });
        });
}

// ## Pills and badges
// The item page's tags, the library's filter values, the watched tick, the status dot and the
// player's small labels.

/// The pills
/// `pill::tag` and `icon_tag`, and `pill::filter` off, included, excluded and highlighted by the
/// keyboard.
#[test]
fn mockup_pills_tags() {
    Mockup::new("pills.tags")
        .themes(Themes::Both)
        .size(vec2(WIDTH, 178.))
        .show(
            || (),
            |ui, _| {
                gallery(ui, |ui| {
                    states(ui, "pill::tag and pill::icon_tag", |ui| {
                        shown(ui, "", |ui| pill::tag(ui, "4K"));
                        shown(ui, "", |ui| pill::icon_tag(ui, ICON_CLOSED_CAPTION, "Subtitles"));
                        shown(ui, "", |ui| pill::tag(ui, "1994"));
                    });
                    states(ui, "pill::filter", |ui| {
                        shown(ui, "off", |ui| pill::filter(ui, "Action", pill::Mark::Off, false));
                        shown(ui, "included", |ui| {
                            pill::filter(ui, "Action", pill::Mark::Included, false)
                        });
                        shown(ui, "excluded", |ui| {
                            pill::filter(ui, "Action", pill::Mark::Excluded, false)
                        });
                        shown(ui, "highlighted", |ui| {
                            pill::filter(ui, "Action", pill::Mark::Off, true)
                        });
                    });
                });
            },
        );
}

/// The badges
/// `badge::check` in a card's corner, `dot` in the success, warning and muted colours, and
/// `label` as the player's bars draw one.
#[test]
fn mockup_pills_badges() {
    Mockup::new("pills.badges")
        .themes(Themes::Both)
        .size(vec2(WIDTH, 334.))
        .show(
            || (),
            |ui, _| {
                gallery(ui, |ui| {
                    states(ui, "badge::check", |ui| {
                        shown(ui, "the corner it marks", check_sample);
                    });
                    states(ui, "badge::dot", |ui| {
                        shown(ui, "success", |ui| badge::dot(ui, theme::tokens_of(ui).success));
                        shown(ui, "warning", |ui| badge::dot(ui, theme::tokens_of(ui).warning));
                        shown(ui, "muted", |ui| {
                            badge::dot(ui, theme::tokens_of(ui).muted_foreground);
                        });
                    });
                    states(ui, "badge::label", |ui| {
                        shown(ui, "", |ui| {
                            let tokens = theme::tokens_of(ui);
                            badge::label(ui, "Transcoding", tokens.inset, tokens.ink_muted);
                        });
                        shown(ui, "in the player's colours", |ui| {
                            badge::label(ui, "1.25×", over_video::SURFACE_SUBTLE, over_video::MUTED_FOREGROUND);
                        });
                    });
                });
            },
        );
}

// ## Rows
// The lists the app draws: a plain line, a person, and an item with its picture.

/// The plain rows
/// `ListRow` sunk, inside a `list_row::block` with each `Leading` and a detail, then one that is
/// highlighted, one with the keyboard's ring, one with the reorder handles, and a compact one.
#[test]
fn mockup_rows_list() {
    Mockup::new("rows.list")
        .themes(Themes::Both)
        .size(vec2(WIDTH, 320.))
        .show(
            || (),
            |ui, _| {
                gallery(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 32.;
                        column(ui, 540., |ui| {
                            shown(ui, "sunk, with a detail at the right end", |ui| {
                                list_row::ListRow::new("A row on its own")
                                    .sunk()
                                    .detail(Some("12:04"))
                                    .show(ui);
                            });
                            shown(ui, "in a list_row::block, with each Leading", |ui| {
                                list_row::block(ui, |ui| {
                                    ui.spacing_mut().item_spacing.y = 0.;
                                    list_row::ListRow::new("A number")
                                        .leading(list_row::Leading::Number(1))
                                        .show(ui);
                                    list_row::ListRow::new("An icon")
                                        .leading(list_row::Leading::Icon(
                                            ICON_CLOSED_CAPTION,
                                            theme::tokens_of(ui).ink_muted,
                                        ))
                                        .show(ui);
                                    list_row::ListRow::new("What's playing")
                                        .leading(list_row::Leading::Playing)
                                        .detail(Some("4:05"))
                                        .show(ui);
                                    list_row::ListRow::new("The cursor's row")
                                        .leading(list_row::Leading::Number(4))
                                        .highlighted(true)
                                        .show(ui);
                                });
                            });
                        });
                        column(ui, 380., |ui| {
                            shown(ui, "ringed", |ui| {
                                list_row::ListRow::new("The keyboard is here")
                                    .sunk()
                                    .ringed(true)
                                    .show(ui);
                            });
                            shown(ui, "with the ↑ ↓ ✕ handles, while active", |ui| {
                                list_row::ListRow::new("A playlist entry")
                                    .sunk()
                                    .detail(Some("1h 32m"))
                                    .handles(list_row::Handles { up: true, down: false }, "Remove from playlist")
                                    .highlighted(true)
                                    .show(ui);
                            });
                            shown(ui, "compact", |ui| {
                                list_row::ListRow::new("A compact row").compact().sunk().show(ui);
                            });
                        });
                    });
                });
            },
        );
}

/// The people rows
/// `AvatarRow` plain, with a status dot and an action, removable, and highlighted with three
/// avatars; then `avatar_row::person`, and `avatar::paint` for a few names.
#[test]
fn mockup_rows_avatar() {
    Mockup::new("rows.avatar")
        .themes(Themes::Both)
        .size(vec2(WIDTH, 372.))
        .show(
            || (),
            |ui, _| {
                gallery(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 32.;
                        column(ui, 540., |ui| {
                            shown(ui, "plain", |ui| {
                                avatar_row::AvatarRow::new(&["dev"], "dev", "Signed in", "dev, signed in")
                                    .show(ui, true);
                            });
                            shown(ui, "a status dot and an action", |ui| {
                                avatar_row::AvatarRow::new(&["alex"], "Alex's group", "2 people", "Alex's group")
                                    .status(theme::tokens_of(ui).success)
                                    .action("Join")
                                    .show(ui, true);
                            });
                            shown(ui, "removable", |ui| {
                                avatar_row::AvatarRow::new(&["dev"], "dev", "Last played yesterday", "dev")
                                    .remove("Sign out")
                                    .show(ui, true);
                            });
                            shown(ui, "highlighted, with three avatars", |ui| {
                                avatar_row::AvatarRow::new(
                                    &["dev", "alex", "sam"],
                                    "Movie night",
                                    "Watching now",
                                    "Movie night",
                                )
                                .highlighted(true)
                                .show(ui, true);
                            });
                        });
                        column(ui, 360., |ui| {
                            shown(ui, "avatar_row::person", |ui| {
                                avatar_row::person(ui, "Ada Lovelace", Some("you"));
                                avatar_row::person(ui, "Grace Hopper", None);
                            });
                            shown(ui, "avatar::paint", |ui| {
                                let (rect, _) = ui.allocate_exact_size(vec2(3. * 44., 44.), Sense::hover());
                                for (index, name) in ["Ada", "Grace", "dev"].iter().enumerate() {
                                    let at = pos2(rect.left() + 22. + index as f32 * 44., rect.center().y);
                                    avatar::paint(ui, at, 18., name);
                                }
                            });
                        });
                    });
                });
            },
        );
}

/// The item rows
/// `media_row::season` closed, open and highlighted; then a `MediaRow` for an episode of the
/// fixtures, its picture the blurhash it carries.
#[test]
fn mockup_rows_media() {
    Mockup::new("rows.media")
        .themes(Themes::Both)
        .size(vec2(WIDTH, 585.))
        .show(MediaRows::new, |ui, state| {
            gallery(ui, |ui| {
                let images = images(ui, &mut state.cache);
                let poster = crate::widgets::text::poster_image(&state.season);
                let season = &state.season;
                states(ui, "media_row::season", |ui| {
                    shown(ui, "closed", |ui| {
                        media_row::season(
                            ui,
                            images,
                            season,
                            poster.clone(),
                            "8 episodes · 2 left",
                            300.,
                            false,
                            false,
                        );
                    });
                    shown(ui, "open", |ui| {
                        media_row::season(
                            ui,
                            images,
                            season,
                            poster.clone(),
                            "8 episodes · 2 left",
                            300.,
                            true,
                            false,
                        );
                    });
                    shown(ui, "highlighted", |ui| {
                        media_row::season(
                            ui,
                            images,
                            season,
                            poster.clone(),
                            "8 episodes · 2 left",
                            300.,
                            false,
                            true,
                        );
                    });
                });
                let episode = &state.episode;
                let still = crate::widgets::text::landscape_image(episode);
                named(ui, "MediaRow, played, with its overview", |ui| {
                    column(ui, 880., |ui| {
                        media_row::MediaRow::new(episode, still, "Test Show S01E01", "S1:E1 · 10m")
                            .overview(Some(
                                "The one where the server hands over a still, and its blurhash stands in \
                                 for the picture until the image arrives.",
                            ))
                            .played(true)
                            .show(ui, images);
                    });
                });
                ui.add_space(media_row::GAP);
                named(ui, "MediaRow, highlighted, its Watched toggle off", |ui| {
                    column(ui, 880., |ui| {
                        media_row::MediaRow::new(episode, None, "Test Show S01E02", "S1:E2 · 10m")
                            .overview(Some(
                                "The same row with no picture to ask for, so its grey placeholder shows.",
                            ))
                            .highlighted(true)
                            .show(ui, images);
                    });
                });
            });
        });
}

// ## Surfaces and popups
// The raised panels the app puts its forms on, the menus it hangs from buttons, and a dialog
// drawn in place rather than over a page.

/// The surfaces
/// `surface::frame` with a centred `heading`, its three notices and a `waiting` line; beside it
/// `heading_left` on a surface, and a `strip` holding a page's toolbar.
#[test]
fn mockup_surfaces_panel() {
    Mockup::new("surfaces.panel")
        .themes(Themes::Both)
        .size(vec2(WIDTH, 390.))
        .show(
            || (),
            |ui, _| {
                gallery(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 32.;
                        column(ui, 400., |ui| {
                            shown(ui, "surface::frame, heading, notice", |ui| {
                                surface::frame(ui).show(ui, |ui| {
                                    surface::heading(ui, "Sign in", "to your own server");
                                    ui.add_space(surface::GAP_HEADING);
                                    surface::notice(ui, surface::Kind::Danger, "Wrong username or password.");
                                    ui.add_space(surface::GAP_FIELD);
                                    surface::notice(ui, surface::Kind::Info, "The server is starting up.");
                                    ui.add_space(surface::GAP_FIELD);
                                    surface::notice(ui, surface::Kind::Success, "Signed in.");
                                    ui.add_space(surface::GAP_FIELD);
                                    surface::waiting(ui, "Waiting for approval…");
                                });
                            });
                        });
                        column(ui, 440., |ui| {
                            shown(ui, "surface::heading_left", |ui| {
                                surface::frame(ui).show(ui, |ui| {
                                    surface::heading_left(ui, "Delete playlist", Some("This can't be undone."));
                                    ui.add_space(surface::GAP_HEADING);
                                    ui.label(label::muted(
                                        ui,
                                        "A dialog's heading sits against its left edge, since a dialog isn't \
                                         a centred form.",
                                    ));
                                });
                            });
                            shown(ui, "surface::strip", |ui| {
                                surface::strip(ui).show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = surface::GAP_ITEM;
                                        select::button(
                                            ui,
                                            vec2(180., ui.spacing().interact_size.y),
                                            "Title",
                                            "Sort by",
                                            false,
                                        );
                                        button::text(ui, "Clear filters", true);
                                        button::quiet_icon(ui, ICON_TUNE, "Filters", None, 1., false);
                                    });
                                });
                            });
                        });
                    });
                });
            },
        );
}

/// The popups
/// `popup::frame` with `popup::row`s: a highlighted one, a checked one with a detail, a
/// separator and a danger one. Beside it a panel's frame with `panel_heading` and
/// `panel_section`.
#[test]
fn mockup_surfaces_popup() {
    Mockup::new("surfaces.popup")
        .themes(Themes::Both)
        .size(vec2(WIDTH, 280.))
        .show(
            || (),
            |ui, _| {
                gallery(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 40.;
                        column(ui, 240., |ui| {
                            ui.label(call(ui, "popup::frame and popup::row"));
                            ui.add_space(4.);
                            let tokens = *theme::tokens_of(ui);
                            popup::frame(&tokens)
                                .corner_radius(CornerRadius::same(popup::RADIUS))
                                .inner_margin(Margin::same(6))
                                .show(ui, |ui| {
                                    ui.set_width(228.);
                                    let entries = [
                                        popup::Entry::new("Play"),
                                        popup::Entry::new("Add to favorites").icon(ICON_FAVORITE).checked(true),
                                        popup::Entry::new("Download").icon(ICON_WATCH_LATER).detail("1.4 GB"),
                                        popup::Entry::new("Remove from queue").separated(),
                                        popup::Entry::new("Delete").icon(ICON_DELETE).danger(),
                                    ];
                                    for (index, entry) in entries.iter().enumerate() {
                                        popup::row(ui, entry, true, index == 0, tokens.inset_hover);
                                    }
                                });
                        });
                        column(ui, 340., |ui| {
                            ui.label(call(ui, "popup::panel_heading and panel_section"));
                            ui.add_space(4.);
                            let tokens = *theme::tokens_of(ui);
                            popup::frame(&tokens)
                                .corner_radius(CornerRadius::same(popup::RADIUS))
                                .inner_margin(Margin::same(popup::PANEL_PAD as i8))
                                .show(ui, |ui| {
                                    ui.set_width(340.);
                                    popup::panel_heading(
                                        ui,
                                        "SyncPlay",
                                        Some(("2 in group", tokens.success)),
                                        "Watch with other people on the same server.",
                                    );
                                    popup::panel_section(ui, "Now playing", 30., |ui| {
                                        ui.label(label::note(ui, "1 of 2"));
                                    });
                                    ui.label(label::muted(ui, "Test Movie (2024)"));
                                });
                        });
                    });
                });
            },
        );
}

/// The dialog in place
/// `surface::frame_of` with a `heading_left` and `dialog::buttons`, drawn on the page rather than
/// as a modal, so the gallery shows a dialog's surface without a page to dim.
#[test]
fn mockup_surfaces_dialog() {
    Mockup::new("surfaces.dialog")
        .themes(Themes::Both)
        .size(vec2(WIDTH, 300.))
        .show(
            || (),
            |ui, _| {
                gallery(ui, |ui| {
                    let tokens = *theme::tokens_of(ui);
                    named(ui, "surface::frame_of", |ui| {
                        // A fixed column: `dialog::buttons` centres its row in the height it's
                        // given, which in a page-sized `Ui` would be the whole mockup.
                        column(ui, 460., |ui| {
                            surface::frame_of(&tokens).show(ui, |ui| {
                                ui.set_width(400.);
                                surface::heading_left(ui, "Delete playlist", Some("This can't be undone."));
                                ui.add_space(surface::GAP_GROUP);
                                ui.label(label::muted(ui, "Do you want to delete this playlist?"));
                                dialog::buttons(ui, |ui| {
                                    dialog::cancel(ui);
                                    dialog::danger(ui, "Delete", button::State::Ready);
                                });
                            });
                        });
                    });
                });
            },
        );
}

// ## States
// What a page shows while it loads or has nothing to show: the mascot in its glow, a line of
// text, an empty state, a failed load with Retry, and a panel's compact version.

/// The page states
/// `state::spinner`, `empty` (the confused mascot), `error` with its notice and *Retry*,
/// `message`, `icon_message` for a panel, and the mascot watching.
#[test]
fn mockup_states_page() {
    Mockup::new("states.page")
        .themes(Themes::Both)
        .size(vec2(WIDTH, 555.))
        .show(
            || (),
            |ui, _| {
                gallery(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 22.;
                        state_cell(ui, "state::spinner", state::spinner);
                        state_cell(ui, "state::empty", |ui| {
                            state::empty(ui, "This library is empty.");
                        });
                        state_cell(ui, "state::error", |ui| {
                            state::error(ui, "Couldn't load the library: the server didn't answer.");
                        });
                    });
                    ui.add_space(theme::SPACE.md);
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 22.;
                        state_cell(ui, "state::message", |ui| {
                            state::message(ui, "Type in the search box to search.");
                        });
                        state_cell(ui, "state::icon_message", |ui| {
                            state::icon_message(ui, ICON_SEARCH, "Nothing matched.");
                        });
                        state_cell(ui, "mascot::watching", |ui| {
                            let (rect, _) = ui.allocate_exact_size(vec2(mascot::PAGE, mascot::PAGE), Sense::hover());
                            mascot::watching(ui, rect, mascot::Watch::default(), true);
                        });
                    });
                });
            },
        );
}

// ## Cards and shelves
// The cards a grid or a shelf is made of, the grey placeholder a loading grid shows, the library
// tiles, and a shelf of cards. This one is drawn by a page, so its person's photo can come from
// the fixture server; the rest of the pictures are the items' blurhashes.

/// The cards
/// `card::poster`, `landscape`, a person whose photo failed and one without, the placeholder, a chapter
/// with and without its image, and `library_tile`; then a `shelf`.
#[test]
fn mockup_cards_items() {
    let server = FixtureServer::start();
    server.file(
        "GET",
        "/Items/95a1149311cb37cc7d064729e0f0af1c/Images/Primary",
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../packaging/tabbyfin-artwork.png"),
        "image/png",
    );
    Mockup::new("cards.items")
        .themes(Themes::Both)
        .size(vec2(WIDTH, 915.))
        .behind(|ui| backdrop::fill(ui.painter(), ui.max_rect(), theme::tokens_of(ui)))
        .page(&server, |_| Box::new(Cards::new()));
}

// ## Over the video
// The player's own dark set, drawn on a grey as the video would be. Its colours are
// `theme::over_video`'s, so both themes draw these the same.

/// The prompts
/// `countdown_pill::show` counting down and with its mark alone, `prompt::quiet` beside it, and
/// `prompt::note` with its action inside the right end.
#[test]
fn mockup_player_prompts() {
    Mockup::new("player.prompts")
        .themes(Themes::Both)
        .size(vec2(WIDTH, 230.))
        .show(
            || (),
            |ui, _| {
                over_video_page(ui, |ui| {
                    states(ui, "countdown_pill::show and prompt::quiet", |ui| {
                        shown(ui, "counting down", |ui| {
                            countdown_pill::show(ui, "Skip intro", ICON_SKIP_NEXT, Some(0.35), pill_colours());
                        });
                        shown(ui, "the ring's mark alone", |ui| {
                            countdown_pill::show(ui, "Play now", ICON_PLAY_ARROW, None, pill_colours());
                        });
                        shown(ui, "prompt::quiet", |ui| {
                            prompt::quiet(
                                ui,
                                "Cancel",
                                prompt::Quiet {
                                    ink: over_video::FOREGROUND,
                                    outline: over_video::QUIET_OUTLINE,
                                    hover: over_video::ROUND_HOVER,
                                },
                            );
                        });
                    });
                    named(ui, "prompt::note", |ui| {
                        prompt::note(ui, "Resume from 12:04?", ICON_PLAY_ARROW, "Start over", note_colours());
                    });
                });
            },
        );
}

/// The player's controls
/// `tabs::bar` and `segmented::show` from the settings menu, `stepper::row` before and after its
/// value moved, `media_row::chapter` current and not, and the control bar's `RoundButton` with a
/// `badge::label`.
#[test]
fn mockup_player_controls() {
    Mockup::new("player.controls")
        .themes(Themes::Both)
        .size(vec2(WIDTH, 335.))
        .show(PlayerControls::default, |ui, state| {
            over_video_page(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 36.;
                    let tokens = over_video::TOKENS;
                    column(ui, 440., |ui| {
                        named(ui, "tabs::bar", |ui| {
                            let tabs = [
                                tabs::Tab {
                                    value: Tab::General,
                                    icon: ICON_TUNE,
                                    label: "General",
                                },
                                tabs::Tab {
                                    value: Tab::Audio,
                                    icon: ICON_AUDIOTRACK,
                                    label: "Audio",
                                },
                                tabs::Tab {
                                    value: Tab::Subtitles,
                                    icon: ICON_SUBTITLES,
                                    label: "Subtitles",
                                },
                            ];
                            tabs::bar(
                                ui,
                                &tabs,
                                Tab::Audio,
                                tabs::Colours {
                                    track: tokens.tab_bar,
                                    selected: tokens.input,
                                    hover: ghost_hover(),
                                    ink: tokens.foreground,
                                    ink_muted: over_video::MUTED_FOREGROUND,
                                },
                            );
                        });
                        named(ui, "segmented::show", |ui| {
                            let labels = ["0.5×", "0.75×", "1×", "1.25×", "1.5×"];
                            segmented::show(
                                ui,
                                &labels,
                                Some(2),
                                segmented::Colours {
                                    track: tokens.muted,
                                    hover: tokens.input,
                                    ink: tokens.foreground,
                                    chosen: over_video::BRAND,
                                    chosen_ink: over_video::BRAND_FOREGROUND,
                                },
                            );
                        });
                        named(ui, "stepper::row", |ui| {
                            // A stepper's buttons take their ids from the `Ui`'s scope, so each
                            // row is drawn in a scope of its own. The arrow glyphs in a key
                            // hint are the system fonts' here, so they're spelled out.
                            for (salt, value, moved) in
                                [("stepper-at-rest", "0.0 s", false), ("stepper-moved", "+1.5 s", true)]
                            {
                                ui.scope_builder(
                                    UiBuilder::new().id_salt(salt).layout(Layout::top_down(Align::Min)),
                                    |ui| {
                                        ui.set_width(440.);
                                        stepper::row(
                                            ui,
                                            "Delay",
                                            Some("Shift + Left/Right"),
                                            value,
                                            moved,
                                            stepper_colours(),
                                        );
                                    },
                                );
                                ui.add_space(theme::SPACE.xs);
                            }
                        });
                    });
                    column(ui, 440., |ui| {
                        let images = images(ui, &mut state.cache);
                        named(ui, "media_row::chapter", |ui| {
                            media_row::chapter(ui, images, None, "Intro", "0:00", true, chapter_colours());
                            ui.add_space(media_row::GAP);
                            media_row::chapter(ui, images, None, "Credits", "45:00", false, chapter_colours());
                        });
                        named(ui, "RoundButton and badge::label, in the player's colours", |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.;
                                round_button::RoundButton::icon(ICON_PLAY_ARROW, "Play")
                                    .colours(CONTROLS)
                                    .show(ui);
                                round_button::RoundButton::icon(ICON_CLOSED_CAPTION, "Subtitles")
                                    .colours(CONTROLS)
                                    .on(true)
                                    .show(ui);
                                round_button::RoundButton::icon(ICON_ARROW_BACK, "Back")
                                    .colours(CONTROLS)
                                    .enabled(false)
                                    .show(ui);
                                badge::label(
                                    ui,
                                    "Transcoding",
                                    over_video::SURFACE_SUBTLE,
                                    over_video::MUTED_FOREGROUND,
                                );
                            });
                        });
                    });
                });
            });
        });
}

// ## Other
// The pieces that aren't a control: a code to read off, and the window's backdrop with its glow.

/// The code and the backdrop
/// `code::show` with its copy button; then `backdrop::fill`, `glow`, `fade_into` and `wash`: the
/// gradient under every page, the light behind the mascot, and the two ways a picture melts into
/// the page.
#[test]
fn mockup_other_utility() {
    Mockup::new("other.utility")
        .themes(Themes::Both)
        .size(vec2(WIDTH, 315.))
        .show(
            || (),
            |ui, _| {
                gallery(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 32.;
                        column(ui, 420., |ui| {
                            named(ui, "code::show", |ui| code::show(ui, "A1B2C3D4"));
                        });
                        column(ui, 440., |ui| {
                            named(ui, "backdrop::fill and backdrop::glow", |ui| {
                                let tokens = theme::tokens_of(ui);
                                let (rect, _) = ui.allocate_exact_size(vec2(400., 100.), Sense::hover());
                                backdrop::fill(ui.painter(), rect, tokens);
                                backdrop::glow(ui.painter(), rect.center(), 90., tokens);
                            });
                            named(ui, "backdrop::fade_into and backdrop::wash", |ui| {
                                let tokens = theme::tokens_of(ui);
                                let window = ui.max_rect();
                                let (rect, _) = ui.allocate_exact_size(vec2(400., 52.), Sense::hover());
                                ui.painter().rect_filled(rect, 8, tokens.brand);
                                ui.painter().add(backdrop::fade_into(rect, window, 0.2, tokens));
                                let below = rect.translate(vec2(0., 60.));
                                let (_, _) = ui.allocate_exact_size(vec2(400., 52.), Sense::hover());
                                ui.painter().rect_filled(below, 8, tokens.brand);
                                ui.painter().add(backdrop::wash(below, window, 0.35, 0.6, tokens));
                            });
                        });
                    });
                });
            },
        );
}

// ---------------------------------------------------------------------------------------------
// The page the cards are drawn by, and the few helpers the mockups share.

/// The window's backdrop with the content inset from the mockup's edges, as a page draws itself.
/// A caption naming the call a widget comes from: `xs` monospace in `ink_muted`, so code names
/// keep their case (an overline would put them in capitals).
fn call(ui: &Ui, name: &str) -> egui::RichText {
    let mut font = theme::text::font(ui, "xs");
    font.family = egui::FontFamily::Monospace;
    egui::RichText::new(name)
        .font(font)
        .color(theme::tokens_of(ui).ink_muted)
}

fn gallery(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    backdrop::fill(ui.painter(), ui.max_rect(), theme::tokens_of(ui));
    egui::Frame::NONE.inner_margin(Margin::same(MARGIN)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        content(ui);
    });
}

/// The player's grey, which both themes draw the over-the-video widgets on.
fn over_video_page(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    ui.painter().rect_filled(ui.max_rect(), 0., Color32::from_gray(0x26));
    egui::Frame::NONE.inner_margin(Margin::same(MARGIN)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        content(ui);
    });
}

/// A call's name over what it draws.
fn named<R>(ui: &mut Ui, caption: &str, draw: impl FnOnce(&mut Ui) -> R) -> R {
    ui.label(call(ui, caption));
    ui.add_space(4.);
    let drawn = draw(ui);
    ui.add_space(theme::SPACE.sm);
    drawn
}

/// A call's name over a row of its states.
fn states<R>(ui: &mut Ui, caption: &str, draw: impl FnOnce(&mut Ui) -> R) -> R {
    ui.label(call(ui, caption));
    ui.add_space(4.);
    let drawn = ui
        .horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 18.;
            ui.spacing_mut().item_spacing.y = 6.;
            draw(ui)
        })
        .inner;
    ui.add_space(theme::SPACE.sm);
    drawn
}

/// One state: the widget, with the state's name under it when it isn't obvious.
fn shown<R>(ui: &mut Ui, note: &str, draw: impl FnOnce(&mut Ui) -> R) -> R {
    ui.vertical(|ui| {
        let drawn = draw(ui);
        if !note.is_empty() {
            ui.add_space(4.);
            ui.label(label::note(ui, note));
        }
        drawn
    })
    .inner
}

/// A fixed-width column, for a widget that takes the whole width it's given.
fn column(ui: &mut Ui, width: f32, draw: impl FnOnce(&mut Ui)) {
    ui.allocate_ui_with_layout(vec2(width, 0.), Layout::top_down(Align::Min), |ui| {
        ui.set_width(width);
        draw(ui);
    });
}

/// A caption over a state in a fixed box, for one that centres itself in what it's given.
fn state_cell(ui: &mut Ui, caption: &str, draw: impl FnOnce(&mut Ui)) {
    ui.vertical(|ui| {
        ui.label(call(ui, caption));
        ui.add_space(4.);
        ui.allocate_ui_with_layout(vec2(300., 340.), Layout::top_down(Align::Min), |ui| {
            ui.set_width(300.);
            draw(ui);
        });
    });
}

/// A sample control with the keyboard's ring round it.
fn ring_sample(ui: &mut Ui, place: focus::Place) {
    let tokens = theme::tokens_of(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(160., 42.), Sense::hover());
    ui.painter().rect_filled(rect, tokens.radius_control, tokens.inset);
    focus::ring(ui, rect, tokens.radius_control as f32, place);
}

/// A card's picture with the watched tick in its corner.
fn check_sample(ui: &mut Ui) {
    let tokens = theme::tokens_of(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(150., 100.), Sense::hover());
    ui.painter().rect_filled(rect, image::RADIUS, tokens.muted);
    badge::check(ui, rect);
}

/// The image cache a mockup draws cards with. Nothing ticks it, so its images stay their
/// blurhashes and nothing goes to a server.
fn images<'a>(ui: &Ui, cache: &'a mut Option<ImageCache>) -> &'a mut ImageCache {
    cache.get_or_insert_with(|| {
        let client = Client::new("http://localhost:8096", session::client_info("gallery")).expect("a server URL");
        ImageCache::new(client, ui.ctx())
    })
}

/// The text fields' contents, one per field the gallery shows.
struct Fields {
    empty: String,
    filled: String,
    password: String,
    token: String,
    search: String,
    search_filled: String,
}

impl Default for Fields {
    fn default() -> Self {
        Self {
            empty: String::new(),
            filled: "dev".to_owned(),
            password: "correct horse".to_owned(),
            token: String::new(),
            search: String::new(),
            search_filled: "firefly".to_owned(),
        }
    }
}

/// The switches' positions.
#[derive(Default)]
struct Controls {
    on: bool,
}

/// The season and episode the item rows draw, from the fixtures.
struct MediaRows {
    season: BaseItemDto,
    episode: BaseItemDto,
    cache: Option<ImageCache>,
}

impl MediaRows {
    fn new() -> Self {
        let seasons: QueryResult<BaseItemDto> = fixture_json("seasons");
        let episodes: QueryResult<BaseItemDto> = fixture_json("episodes");
        Self {
            season: seasons.items.into_iter().next().expect("a season fixture"),
            episode: episodes.items.into_iter().next().expect("an episode fixture"),
            cache: None,
        }
    }
}

/// The image cache the player's chapter rows draw with.
#[derive(Default)]
struct PlayerControls {
    cache: Option<ImageCache>,
}

/// The settings menu's tabs, as the player names them.
#[derive(Clone, Copy, PartialEq)]
enum Tab {
    General,
    Audio,
    Subtitles,
}

/// The page the cards are drawn by: the one family whose pictures come from a server, because a
/// person's photo has no blurhash to stand in for it.
struct Cards {
    movies: Vec<BaseItemDto>,
    views: Vec<BaseItemDto>,
    people: Vec<BaseItemPerson>,
    chapters: Vec<ChapterInfo>,
}

impl Cards {
    fn new() -> Self {
        let movies: QueryResult<BaseItemDto> = fixture_json("items_movies");
        let views: QueryResult<BaseItemDto> = fixture_json("user_views");
        let movie: BaseItemDto = fixture_json("item_movie");
        Self {
            movies: movies.items,
            views: views.items,
            people: movie.people,
            chapters: movie.chapters,
        }
    }
}

impl Page for Cards {
    fn title(&self) -> &str {
        "Cards"
    }

    fn show(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        let images = &mut cx.services.images;
        gallery(ui, |ui| {
            let movies = &self.movies;
            let poster = movies.first().expect("a movie fixture");
            let landscape = movies.get(1).expect("a second movie fixture");
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 24.;
                shown(ui, "card::poster", |ui| {
                    card::poster(ui, images, poster, false);
                });
                shown(ui, "card::landscape", |ui| {
                    card::landscape(ui, images, landscape, false);
                });
                let photo = self.people.first().cloned().map(|mut person| {
                    person.primary_image_tag = Some("gallery".to_owned());
                    person
                });
                shown(ui, "card::person, its photo failed", |ui| {
                    if let Some(person) = &photo {
                        card::person(ui, images, person, 1., false);
                    }
                });
                shown(ui, "without one, on their pastel", |ui| {
                    if let Some(person) = self.people.get(1) {
                        card::person(ui, images, person, 1., false);
                    }
                });
                shown(ui, "card::placeholder", card::placeholder);
            });
            ui.add_space(theme::SPACE.md);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 24.;
                if let Some(chapter) = self.chapters.first() {
                    shown(ui, "card::chapter, no image for it", |ui| {
                        card::chapter(ui, images, "gallery", 0, chapter, false);
                    });
                    shown(ui, "card::chapter_without_image", |ui| {
                        card::chapter_without_image(ui, 0, chapter, false);
                    });
                }
                shown(ui, "card::poster_scaled", |ui| {
                    card::poster_scaled(ui, images, poster, 0.6, false);
                });
                column(ui, 236., |ui| {
                    ui.label(call(ui, "library_tile::show"));
                    ui.add_space(4.);
                    for view in &self.views {
                        if matches!(view.name.as_str(), "Movies" | "Collections") {
                            library_tile::show(ui, images, view, false);
                        }
                    }
                });
            });
            ui.add_space(theme::SPACE.md);
            ui.label(call(ui, "shelf::shelf_with_gaps"));
            ui.add_space(4.);
            shelf::shelf_with_gaps(
                ui,
                "gallery-shelf",
                "Continue watching",
                16.,
                12.,
                shelf::PADDING,
                |ui| {
                    for index in 0..6 {
                        let item = &movies[index % movies.len()];
                        card::poster(ui, images, item, index == 1);
                    }
                },
            );
        });
    }
}

/// The player's control bar's buttons: white icons with no frame at rest, a white circle at 16 %
/// under the pointer, and `BRAND` while on. Copied from `views/player.rs`, where they're private.
const CONTROLS: round_button::Colours = round_button::Colours {
    circle: over_video::ROUND_HOVER,
    ink: over_video::FOREGROUND,
    ink_hover: over_video::FOREGROUND,
    on: over_video::BRAND,
};

/// A prompt's countdown pill, `BRAND` with the dark ink on it.
fn pill_colours() -> countdown_pill::Colours {
    countdown_pill::Colours {
        fill: over_video::BRAND,
        hover: over_video::BRAND_HOVER,
        ink: over_video::BRAND_FOREGROUND,
        track: over_video::BRAND_TRACK,
    }
}

/// The resume prompt's pill: the menus' surface, its *Start over* in `BRAND`.
fn note_colours() -> prompt::Note {
    prompt::Note {
        frame: popup::over_video_frame(0),
        ink: over_video::FOREGROUND,
        action: over_video::BRAND,
        action_hover: over_video::BRAND_HOVER,
        action_ink: over_video::BRAND_FOREGROUND,
    }
}

/// A chapter row's colours over the video.
fn chapter_colours() -> media_row::ChapterColours {
    media_row::ChapterColours {
        current: over_video::BRAND.gamma_multiply(0.16),
        ink_current: over_video::BRAND,
        hover: over_video::ROW_HOVER,
        ink: over_video::FOREGROUND,
        ink_muted: over_video::MUTED_FOREGROUND,
    }
}

/// A delay's stepper colours over the video.
fn stepper_colours() -> stepper::Colours {
    let tokens = over_video::TOKENS;
    stepper::Colours {
        fill: tokens.muted,
        hover: tokens.input,
        ink: tokens.foreground,
        ink_muted: over_video::MUTED_FOREGROUND,
    }
}

/// The settings menu's tab hover: the classic accent at half strength.
fn ghost_hover() -> Color32 {
    let [r, g, b, _] = over_video::TOKENS.accent.to_srgba_unmultiplied();
    Color32::from_rgba_unmultiplied(r, g, b, 128)
}
