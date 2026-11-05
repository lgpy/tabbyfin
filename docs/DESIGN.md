# The cozy look

How Tabbyfin looks: its principles, palette, type and shapes, the shared components every screen is built from, and the layout of the main screens. `theme::visuals` builds egui's colours from the same tokens, so egui's own widgets match the app's. When a screen settles something new, add it here.

## Principles

- **Warm, soft, quiet.** The mascot's lavender instead of neutral greys; large radii; soft shadows instead of hard outlines. Nothing shouts except the one main action.
- **One main action per view.** It's the only `brand` button. Everything else is `quiet` (a real alternative) or `subtle` (a way out or aside, under the panel).
- **Layers, not lines.** Three depths: the *backdrop* (the window), a *surface* raised off it, and *insets* sunk into the surface (fields, rows, codes). Depth is shown by fill and shadow; borders are 1 pt and close to the fill.
- **The mascot is part of the UI.** Where it appears it reacts to what's happening (watching the caret, eyes shut for a password, confused over an error), and the glow behind it lights the panel below. It appears on sign-in, restoring and the page states (loading, empty, failed) only, and the glow always goes with it; the player's loader over the video is the one mascot without a glow.
- **Both themes are designed, not derived.** Light is "a pale lilac morning", dark "a nightlight: deep plum, lit from behind the mascot". Each token has a value per theme, chosen by eye and checked for contrast.
- **Every widget reads its colours from `theme::tokens_of(ui)`.** No literal colours in views.
- **Views compose, widgets draw.** Anything another page could use lives in `widgets/` and in the component gallery, never as a helper private to one view.

## Layers

| Layer | What it is | Drawn by |
|---|---|---|
| Backdrop | Vertical gradient `backdrop_top` → `backdrop_bottom` over the whole window, plus an optional soft `glow` pooled behind the mascot | `widgets::backdrop::fill`, `backdrop::glow` (meshes; egui has no gradients) |
| Surface | A panel: `surface` fill, 1 pt `surface_border`, `radius_surface` (22), 28 pt padding, shadow offset 0/12, blur 40 in `surface_shadow` | `widgets::surface::frame(ui)` |
| Inset | Sunk into a surface: `inset` fill, `inset_hover` under the pointer, `radius_control` (12) | text fields (`theme::visuals`), `AvatarRow`, `code::show` |

The CentralPanel holding the sign-in screen is frameless (`Frame::NONE`) so the backdrop covers the window.

**Under the shell.** The signed-in window is on the same backdrop: the shell paints `backdrop::fill` over the whole window, and the top bar and the page are frameless panels on it, with no fill and no separator of their own. Browsing pages have no glow, since the glow goes with the mascot. Where a page scrolls under the bar, it marks its top edge with `backdrop::scroll_edge`: a 1 pt `surface_border` hairline across the scroll area's top, drawn only once the page has scrolled (it comes in over the first 12 points), so a page at its top runs into the bar with nothing between them. The hairline was picked over a 16 pt fade of the backdrop over the content, which read as a smudge over bright posters. Every page's main scroll area calls it (Home, item, library, search, both of Settings' columns). Something that fades a picture into the page (an item's banner) fades into the gradient at its own height with `backdrop::fade_into`, not into a flat colour, which would leave a step.

## Colour tokens

`theme::Tokens` (`crates/app/src/theme.rs`), one set per theme.

| Token | Light | Dark | Use |
|---|---|---|---|
| `backdrop_top` | `#fcfaff` | `#0e0b16` | window gradient, top |
| `backdrop_bottom` | `#eee6fa` | `#1d1633` | window gradient, bottom |
| `glow` | `#c396e7` @ 97 | `#c396e7` @ 77 | light behind the mascot, at its peak |
| `surface` | `#ffffff` | `#1a1526` | raised panel |
| `surface_border` | `#e8e0f4` | `#2c2440` | panel and inset outlines |
| `surface_shadow` | plum @ 26 | black @ 110 | panel shadow |
| `inset` | `#f8f5fc` | `#120e1b` | fields, rows, code |
| `inset_hover` | `#f0eaf9` | `#221b31` | hovered/focused inset, hovered icon circle |
| `ink` | `#1f182b` | `#f4f0fa` | text |
| `ink_muted` | `#605770` | `#a89fbb` | labels, subtitles, hints, icons at rest |
| `brand` | `#7b4fb0` | `#c396e7` | main action, focus ring |
| `brand_hover` | `#6a429e` | `#d2adf0` | main action hovered/pressed |
| `brand_foreground` | white | `#1d1230` | text on `brand` |
| `danger_action` | `#dc2626` | `#f87171` | a main action that deletes (`button::danger`) |
| `danger_action_hover` | `#b91c1c` | `#fca5a5` | the same hovered/pressed |
| `danger_action_foreground` | white | `#1d1230` | text on `danger_action` (4.8:1, 6.4:1) |
| `scrim` | `ink` @ 30 % | `backdrop_top` @ 60 % | the page under a dialog |
| `avatars[5]` | lavender, periwinkle, pink, mint, peach | same | initial circles, picked by a hash of the name |
| `avatar_foreground` | `#1d1230` | `#1d1230` | initial on the pastel |
| `danger` | `#ef4444` | `#f87171` | danger notice |
| `info` | `#06b6d4` | `#22d3ee` | info notice |
| `success` | `#22c55e` | `#4ade80` | success notice |

`selection` is the text selected in a field, and `radius_lg` (12) is for cards and popups. Where something needs one flat colour for the page (egui's `panel_fill`, what a card's hover dims with) it's `backdrop_top`, the gradient's top end, not the lilac bottom: in dark the bottom end (`#1d1633`) is lighter than `surface` (`#1a1526`), so raised things would look sunk, and in light `inset_hover` rows and `surface_border` outlines disappear against it. The gradient's lilac end passes for text: `ink_muted` (card subtitles, shelf titles) is 5.6:1 on light `backdrop_bottom` and 6.9:1 on dark, `ink` 14:1 and 15:1. The focus ring (egui's `selection.stroke`) and the cursor rings are `brand`. The player stays dark over the video in either theme, with its colours in `theme::over_video`: its menus are neutral greys (`MENU`, `MENU_BORDER`, `MENU_INK`, `MENU_INSET`, `MENU_INSET_HOVER`, `TAB_TRACK`), and it borrows one accent from the palette, the dark `brand` (`over_video::BRAND`, with `BRAND_FOREGROUND` on it): the seek bar's played part (its thumb a white dot, which stands out on bright and dark pictures alike; the volume slider stays white), the playing track's tick, the chosen speed, the current chapter (its row tinted with `brand` at 16 % and its name in `brand`) and a prompt's main button (*Play now*, *Start over*). Its menu rows hover in a neutral grey (`over_video::ROW_HOVER`), not `inset_hover`, whose purple reads as a tint over the picture. Its control bars' buttons are `RoundButton`s in white over the video: no frame at rest, white at 16 % (`over_video::ROUND_HOVER`) under the pointer, and a button whose menu is open keeps that circle with its icon in `BRAND`. The top bar's title is `lg`, an episode's series and number beside it in a muted `sm`, and the clock is `sm`.

Light `brand` is deeper than the mascot's lavender so white text passes AA (5.9:1); dark uses the mascot's lavender itself with dark text. The status colours (`danger`, `info`, `success`) are too pale for text on a light surface (green and cyan about 2.3:1 on white), so they tint a notice's box, outline and icon, and its text stays `ink`.

## Type

The app's text table (`theme::text`, scaled by the text-size setting):

| Style | Size | Use |
|---|---|---|
| `2xl` | 24 | surface title |
| `xl` | 20 | page and section titles: the top bar's, Settings' sections, shelves |
| `lg` | 18 | icons inside fields |
| `base` | 16 | body, field text, button labels, row titles, surface subtitle |
| `sm` | 14 | field labels, waiting line, asides and subtle links |
| `xs` | 12 | row subtitles |
| monospace 36 | — | a code to read off (Quick Connect) |

Titles are `ink`; everything secondary is `ink_muted`. Get fonts with `text::font(ui, "<name>")`, not literal sizes (the code widget is the one fixed-size exception).

## Shape and spacing

- Radii: `radius_surface` 22 for panels, `radius_control` 12 for everything on them (fields, buttons, rows, insets, notices). Popups sit between: 16 for menus and toasts (`popup::RADIUS`), 10 for tooltips.
- Heights: buttons 42 (`button::COZY_HEIGHT`), avatar rows 56, code inset 76. Field margin 12 × 9.
- Panel padding 28. The sign-in column is 400 wide, centred horizontally and vertically (measured last frame, so a step change asks for one repaint to recentre; taller than the window, it scrolls and Tab scrolls the focused field into view).
- Vertical rhythm on a surface: named gaps in `widgets/surface.rs`, each a step of `theme::SPACE`. Use the name, not a number:

  | Gap | Value | Between |
  |---|---|---|
  | `GAP_HEADING` | `SPACE.xl` 24 | the heading and the step's content |
  | `GAP_GROUP` | `SPACE.lg` 16 | groups: a row list and what follows it, the last field and the main button, a code and its waiting line, the panel and the aside under it |
  | `GAP_FIELD` | `SPACE.md` 12 | field and field; a notice or waiting line and what follows it |
  | `GAP_ITEM` | `SPACE.sm` 8 | items of a list: row and row, stacked buttons |

  Inside a heading, the title sits `SPACE.xs` (4) above its subtitle; a field's label sits `SPACE.xxs` (2) above its field.

## Components

All in `crates/app/src/widgets/`. The component gallery, `crates/app/src/ui_tests/gallery.rs`, draws every widget below in each of its states in both themes, one mockup per family: it's the catalogue to look at when changing one, and a new widget or a new state of one is added there. Render it with `python -I .agents/skills/ui-mockup/render.py crates/app/src/ui_tests/gallery.rs --no-open`, which builds it with the `mockup` feature, so a widget change that breaks it fails that build.

- **`surface::frame(ui)`** — the panel (`frame_of(tokens)` where there's no `Ui` yet). **`surface::heading(ui, title, subtitle)`** — centred `2xl` title over a `base` muted line; every panel starts with one. The title names the step ("Sign in to *server*", "Approve this sign-in", "Welcome back"); the subtitle says what to do or where you are. **`surface::heading_left(ui, title, Option<subtitle>)`** — the same against the left edge, for a surface that isn't a centred form (a dialog), the subtitle only when there's something to explain.
- **`surface::strip(ui)`** — a surface one row of controls tall for a page's toolbar: the surface's fill and outline at radius 16 (`STRIP_RADIUS`), 8 × 6 inside (`STRIP_PAD`), and the popup's closer shadow, since a panel's would be as tall as the strip.
- **`surface::waiting(ui, line)`** — small spinner and a muted `sm` line, centred together. For a wait the user can't act on ("Waiting for approval…", "server is starting").
- **`field::label` + `field::single_line`** — muted `sm` label above a full-width `base` field. Hint text only while empty *and* unfocused (egui's hint next to the caret reads as typed text). **`field::with_trailing` + `field::trailing`** — an icon button inside the field's right end (show/hide password), named and tooltipped by its label: a `RoundButton` 28 across with the icon at `lg`. `field::trailing_button` and `trailing_center` give the same button for a caller that sets its focus (the search box's clear button skips it). **`field::with_leading` + `field::leading`** — an icon painted inside the field's left end (a search box's magnifier), in `ink_muted` at `lg`, with `leading_room` as the margin that keeps the text clear of it. Drawn after the field, which would cover it.
- **`button::brand`** — the main action, full width, `brand` fill. **`button::quiet`** — same size, outlined on the surface; **`button::quiet_danger`** is the same with its label in `danger`, for leaving or losing something that isn't the view's main action (*Leave group*). **`button::subtle`** — muted, underlined `sm` link with no frame, for asides under the panel ("Use another server", "Cancel"). Buttons take a `State`: `Ready`, `Disabled`, or `Loading` (disabled, spinner at the right end, label doesn't move). A view with several actions runs them through one helper so the running one loads and the others disable. `brand` and `quiet` also take a `Focus`: `Tab` (a Tab stop, pressed by Space or Enter, as on sign-in) or `Skip` (clicks only, `Sense::CLICK`), which dialogs use because their keys are their own (`widgets/dialog.rs`). **`button::danger`** — `brand`'s shape in `danger_action`, for a main action that deletes something. **`button::text(ui, label, enabled)`** — a small action written out in `brand` at `sm` with no frame, for a row of controls where a button would be one box too many (*Clear filters*); clicks only, dimmed while disabled. **`button::link(ui, label, size)`** — a link inside the text around it (an episode's series line, a Settings note): `label` then a › in `ink_muted` at the caller's size, underlined while the pointer is on it, and a `Link` named by `label` to AccessKit.
- **`dialog::show`** — a dialog: an `egui::Modal` in a surface frame over the page dimmed with `scrim`, `heading_left`, `GAP_GROUP` under it. **`dialog::buttons`** puts its buttons at the bottom right, `GAP_ITEM` apart: `dialog::main` (`brand`) or `dialog::danger`, then `dialog::cancel` (`quiet`), all `Focus::Skip` and as wide as their label plus room for the loading spinner (96 at least). **`dialog::show_page`** is a page a dialog turned to from its first one: a Back button (36, `Focus::Skip`) before the heading; Back, Esc and the phone's back return to the first page, a tap outside closes the dialog. **`frosted::Frosted`** puts a dialog over the page blurred, under its scrim: the window captured as it opens, shrunk to a pixel every 4 points and three box passes of 4 run over it (about a 16-point Gaussian), drawn on `Order::Middle`. The picture is taken once, so it doesn't follow the page; the dialog waits for it (up to 400 ms, then opens on the scrim alone). The phone's library filters use it. A list in a dialog (*Add to playlist*) is inset rows: `inset` at rest, `inset_hover` on the row Enter would pick, with the focus ring inside it while the keyboard put it there.
- **`avatar::paint`** — a pastel circle with the name's initial; the colour is stable per name across launches.
- **`AvatarRow::new(names, title, subtitle, label)`** — avatar, title, muted subtitle, as one button named by `label`. Inset fill, `inset_hover` on hover/focus, the focus ring (inside) on focus or `.highlighted(true)` (a list that keeps its own keyboard). Several names overlap as 14-point avatars (up to three, each parted from the one under it by a ring of the row's fill) for a group. `.status(colour)` puts a dot before the subtitle. At the right end, `.remove(tooltip)` adds a ✕ (a second focus stop), or `.action(label)` a compact `brand` button (30 tall, radius 10, `brand_hover` with the row) that is part of the row: a click anywhere on it presses it. `.focus(Focus::Skip)` for clicks only. **`avatar_row::person(ui, name, aside)`** — a person in a list that isn't a list of buttons: a 14-point avatar, the name in `ink` at `base` and an optional `xs` muted aside ("you"), 36 tall, a `ListItem` to AccessKit.
- **`RoundButton`** (`round_button.rs`) — a round button holding an icon (`RoundButton::icon`) or a person's avatar (`RoundButton::avatar`, 4 points inside the circle): no frame at rest and the icon in `ink_muted`; under the pointer an `inset_hover` circle and `ink`. `.on(bool)` makes it a toggle (selected to AccessKit): on keeps the circle with the icon in `brand`. Disabled is dimmed `ink_muted` at 45 %. `.ringed(true)` (or egui's focus) puts `focus::ring` round it, a circle round the circle (round the avatar for an avatar). Named and tooltipped by its label; `.tooltip(text)` gives it a tooltip of its own (the player's, with the key), and `.colours(Colours)` paints it in the caller's colours instead of the theme's (the player's, over the video). Sizes: 40 with a 24-point icon (the top bar), 30/18 (the SyncPlay panel's toggles), 30/18 (a list's ↑ ↓ ✕ handles as well), 28/`lg` (a field's trailing button), 40/22 and 48/26 (the player's control bars). `round_button::glyph` centres an icon on its own bounds, which the icon font needs.
- **`button::brand_icon(ui, icon, label, ringed)`** — `brand` as wide as its icon (22) and label need, 42 tall, for a page's main action in a row of others (the item page's Play/Resume). **`button::quiet_icon(ui, icon, label, on, icon_scale, ringed)`** — the actions beside it: a 42-point square with `radius_control`, `surface` fill and a `surface_border` outline, the icon in `ink_muted` (`ink` and `inset_hover` under the pointer); with `on` it's a toggle (selected to AccessKit) that while on is `inset_hover` with a `brand` outline and icon. Both are `Sense::CLICK` only; `ringed` draws `focus::ring` outside for a page's cursor. Squares rather than round buttons, since on the item page the actions sit on the backdrop's picture.
- **`MediaRow`** (`media_row.rs`) — an item in a list (a season's episodes, a playlist's entries, the open season's episodes on a series): `inset` at rest, `inset_hover` under the pointer or `.highlighted(true)` with the ring inside, `radius_control`, `media_row::GAP` (8) apart, 10 inside. The wide image (radius 6), the heading in `ink` at `base`, a muted `sm` line, up to three lines of `sm` overview in `ink_muted`, and a round Watched toggle (36/20) level with the heading. `.compact(true)` is a season's episode: a 160×90 image, two lines of overview and a 32/18 toggle, in `media_row::episode_columns` of at least 420, so a 12-episode season fits in about a window. `.handles(…)` adds a playlist's ↑ ↓ ✕ (`list_row::handles`, round, 30/18) before the toggle, shown only while the row is active; the text keeps their room either way, so it doesn't reflow on hover. **`media_row::season`** — a series' season as a row 84 tall (poster, name, a muted line, a chevron), in `season_columns` of at least 300; the open one keeps `inset_hover` with the shared `brand` bar (`list_row::open_bar`) and the chevron turned up in `brand`. A `Button` to AccessKit, selected while open. **`media_row::chapter(ui, images, image, name, start, current, ChapterColours)`** is a row of the player's chapters menu: 8 round a 128 × 72 thumbnail (radius 6), radius 10, 2 apart; the name at `base` over its start at `sm`, centred on the thumbnail. The current one is tinted `ChapterColours::current` with its name in `ink_current`, a hovered row in `hover`; the colours are the caller's since it sits over the video.
- **`ListRow`** (`list_row.rs`) — a one-line row of a list, painted and click-only because its list keeps the keyboard: a leading slot (`Leading::Icon(icon, colour)` at `lg`, `Number(n)` in `xs` `ink_muted` or the `brand` `Playing` mark, the last two in a 36-wide slot), a truncated title in `ink`, an optional `sm` muted `.detail` at the right, and `.handles(…)`, the ↑ ↓ ✕ shown only while it's active. Regular is `base` and 24 taller than the line; `.compact()` is `sm`, 36 tall at the default size. `.sunk()` gives a row its own `inset` at `radius_control`, as the Add to playlist dialog's, 6 apart; a plain row sits in **`list_row::block`**, one `inset` with a `surface_border` outline and 4 points inside, as the SyncPlay queue's, with corners concentric with it. Either way it's `inset_hover` under the pointer or while `.highlighted`, and `.ringed` draws `focus::ring` inside; `.no_hover()` is for a list whose highlight already follows the pointer. **`list_row::handles`** draws the three buttons at 30/18, 2 apart, for `MediaRow` too. Role and AccessKit label are `.role` and `.label`. **`list_row::open_bar(ui, rect)`** is the `brand` bar marking a row as open or selected: 3 wide and round at that width, 60 % of the row's height, down its left edge. The open Settings section's entry and a series' open season both use it.
- **`pill`** (`pill.rs`) — a fully round pill, sunk. **`pill::tag(ui, name)`** is the small static one: `inset`, a `surface_border` outline, `xs` text in `ink_muted`, with `pill::tag_width` so a wrapping row can break before one that wouldn't fit. **`pill::icon_tag(ui, icon, name)`** is the same shape with an icon before the name (`xs` size + 2, in `ink_muted`), for a tag that says what kind of thing it names (a Settings section's store). **`pill::filter(ui, name, Mark, highlighted)`** is the regular clickable one, at least 28 tall in `sm`: `inset` with the outline off, `brand` with a tick included, `inset` with a `danger` outline, text and minus excluded; `focus::ring` outside it when highlighted, and its state is in its AccessKit name.
- **`tabs::bar(ui, tabs, current, Colours)`** — equal tabs across the width, each an icon (18) and a label at `sm`, on a sunk track 40 tall at radius 12, the open one lifted off it. **`stepper::row(ui, label, keys, value, moved, Colours)`** — a value changed a step at a time, 36 tall: the label at `sm`, optional keys in a muted `xs` aside, and at the right a pill 128 × 30 of − value +. The reset (↺) shows beside the pill only once `moved`, so a setting at rest offers nothing to undo. **`segmented::show(ui, labels, current, Colours)`** — one short choice of a few on a sunk track 32 tall, each as wide as its label plus an equal share of the rest, the chosen one a pill. All three take their colours from the caller, since they sit over the video, and none takes keyboard focus. The player's settings menu is built from them.
- **`countdown_pill::show(ui, label, icon, fraction, Colours)`** — a button that counts down, 44 tall and fully round: the label in `base`, then a 28-wide ring round a 16-point icon, closing clockwise from the top as `fraction` goes to 1 (`countdown_pill::progress`), or no ring for `None`. The colours are the caller's. The player's skip button is one in `over_video::BRAND`, with `BRAND_FOREGROUND` on it, and so is the up-next card's *Play now*, its ring the time left before autoplay (no ring without autoplay). Takes no keyboard focus; a button to AccessKit, named by its label.
- **`prompt`** (`prompt.rs`) — the rest of the player's prompts, 44 tall and fully round like the countdown pill so the prompts in the player's corner read as one family. **`prompt::quiet(ui, label, Quiet)`** is an outlined pill (the up-next card's *Cancel*). **`prompt::note(ui, text, icon, label, Note)`** is a pill on the caller's frame holding `base` text and, inside its right end, a compact action 36 tall (the resume prompt, "Resumed from 12:04" with *Start over*). Colours are the caller's; nothing takes keyboard focus. **`popup::over_video_frame(margin)`** is the surface they and the player's menus sit on: `over_video::MENU` with a `MENU_BORDER` outline, `popup::RADIUS` and a dark shadow.
- **`badge`** (`badge.rs`) — **`badge::check(ui, rect)`** is a 16-wide `brand` circle with a tick centred by its glyph, 4 in from `rect`'s top right corner (a watched card). **`badge::dot(ui, colour)`** is an 8-wide status dot beside a heading's note, and **`badge::dot_at`** places one, 6 wide (`DOT_SMALL`) in a row's subtitle. **`badge::label(ui, name, fill, colour)`** is `xs` text on a pill, 2 above and below and 8 either side, fully round at any text size; its colours are the caller's since the player's labels sit on the video (`over_video::SURFACE_SUBTLE`). `badge::label_width` measures one for a bar.
- **Cards** (`card.rs`) — image-first, with no surface round them: the picture at radius 12 (`image::RADIUS`; a panel's 22 is too round for posters), the title in `ink` at `sm` and a muted `xs` line under it. The played badge (`badge::check`) and the cursor's ring (`focus::ring`, inside) are `brand`. The progress bar is a round bar 5 tall, 10 in from the image's sides and bottom: a frosted track (white at 27 %) filled with `over_video::BRAND` in both themes, since it sits on the picture, over a black wash up the image's bottom 48 (eased in, 43 % at the edge) so its length shows on bright pictures too. Home's play button (`card::play_button`, under the pointer) is a `brand` circle in the image's middle with a soft shadow (0 × 4, blur 16) and the arrow centred by its glyph; with the pointer on it, it grows 10 % and turns `brand_hover`. A person card without a picture is their `avatar::colour` pastel with the initials at `2xl` in `avatar_foreground`, so a cast member has the colour their avatar has anywhere else.
- **`shelf`** (`shelf.rs`) — a titled row of cards: the title `xl` in `ink` (a chevron after it in `ink_muted` when it leads to a page), and at the row's right end two `RoundButton` arrows, 32 across with 20-point icons, shown once the row overflows and dimmed at its end. The title row is the arrows' 32 tall.
- **`library_tile::show`** — a library as a small surface, Home's *My Media* row: 236 × 76, `surface` with its `surface_border` outline (`ink_muted` under the pointer), radius 16 and the popups' shadow; the library's picture as a 96 × 56 thumbnail at radius 10, or its kind's icon in `brand` on `inset_hover` when it has none, then its name in `ink` at `base`, cut short when it doesn't fit. A button named by the library; the cursor's ring is inside it. Tiles rather than image cards, so libraries read as places to go, not things to watch.
- **`code::show`** — a code to read off: inset, 36 pt monospace split in two halves, copy button at the right end that turns into a check mark for 1.5 s.
- **`surface::notice(ui, Kind, text)`** — a message under the heading: a box tinted with the kind's colour at 10 %, a 1 pt outline in it, `radius_control`, 12 × 9 margin, then the kind's icon (the toasts' icons, `lg`) and the message in `ink` at `base`, wrapping under itself rather than under the icon. Kinds: `Danger` (something failed), `Info` (something to know), `Success` (something worked). The icon names the kind to AccessKit ("Error", "Information", "Success"), so the kind isn't told by colour alone.
- **`focus::ring(ui, rect, radius, Place)`** — the keyboard's ring, wherever the keyboard (egui focus or a page's cursor) sits on something: 2 pt `brand`, the same on every widget and page. `Place::Outside` for free-standing controls (bar buttons, selects, pills, the item page's actions): 2 pt clear of the control, with the control's radius plus the gap, so a circle gets a circle and a pill a pill. `Place::Inside` for rows and cards, on their own edge, since the lists and scroll areas they sit in would clip a ring outside them. **`focus::ring_widget(ui, &response)`** rings an egui `Button` or `TextEdit` on the theme's widget radius.
- **`label::muted`, `note`, `title`, `heading`, `overline`** — the text styles beyond body text, as `RichText`: `muted` is `sm` `ink_muted` (a waiting or empty line, a field's label, an aside), `note` `xs` `ink_muted` (a panel's status or hint), `title` `base` strong `ink` (a panel's or group's title), `heading` `xl` `ink` not strong (a page's section heading: a shelf's title, a list's heading), `overline` `xs` strong capitals in `ink_muted` (a heading over a menu's or a panel's section). Over the video (the player), recolour with `.color(…)`.
- **Popups** (`popup::PopupMenu`, `popup::panel`, tooltips, toasts) — a `surface` with its `surface_border` outline and `theme::popup_shadow`, the surface's shadow colour at 0 × 6 offset and 20 blur (a surface's is 12 and 40). **`popup::frame(tokens)`** is that frame without radius or margin. Menus: radius 16, 6 pt round the rows, the highlighted row an `inset_hover` pill concentric with the panel (radius 10), a danger row's highlight `danger` at 14 %, separators in `surface_border`. Tooltips are egui's `Frame::popup`, which `theme::visuals` gives the same frame at radius 10 with a 10 × 6 margin; dialogs set their own frame on the `Modal` so they don't take the tooltip's. **`popup::panel`** is a popup laid out like a small surface rather than a list of rows (the SyncPlay panel): the menus' frame and placement with `popup::PANEL_PAD` (16) inside. **`popup::panel_heading(ui, title, status, line)`** is its top: the title (`label::title` at `lg`) with an optional status dot and note at the right end, over a muted line saying what the panel is for. **`popup::panel_section(ui, title, height, right)`** is a section's overline `height` tall, with the caller's controls at the other end.
- **`toggle::switch(ui, &mut on, name)`** — an on/off switch, 40 × 22: a pill track sunk like an inset while off (`inset`, `surface_border` outline, `ink_muted` outline and `inset_hover` under the pointer) and filled with `brand` while on (`brand_hover` under the pointer), with a round knob 3 pt inside it, `ink_muted` off and `brand_foreground` on, so the state reads without colour too. The knob slides over egui's animation time (none with the animations setting off). Focusable: Space or Enter flips it, and `focus::ring` sits outside it. A `Switch` to AccessKit, named by the setting, with its state. Settings' flags are switches. **`toggle::row(ui, on, label, Focus, ringed)`** is the switch with its label across an inset row (`inset`, `inset_hover` on hover or while `ringed`, `radius_control`, 12 × 9 padding): the label in `ink` at `base` at the left, the track at the right, the whole row the target and one `Switch` to AccessKit. It doesn't flip itself, so the caller can refuse while busy; `Focus::Skip` keeps it out of egui's focus for a dialog that moves its own highlight (`ringed` then draws `focus::ring` inside it). The playlist dialog's sharing is one. A list that ticks several things (genres, tags) keeps `popup::row`'s ticks rather than switches.
- **`select::select` / `select::button`** — sunk like a field: `inset` fill, `surface_border` outline that darkens to `ink_muted` under the pointer, `radius_control`, the value in `ink` at `sm` and a muted caret; the keyboard gets `focus::ring` outside it. Its height is the caller's (32 by default, the toolbar's).
- **Toasts** (`toasts.rs`) — 340 wide in the bottom-right corner, 8 apart, each a popup surface with 14 pt padding: the kind's icon (20 pt, named to AccessKit like a notice's) on the first line, the title in `ink` at `base` over the text in `ink_muted` at `sm` (the text alone is in `ink`), and at the right end, centred on the toast, the action as a compact `brand` button (30 tall, radius 10) and a round close button that fills `inset_hover` on hover. Both buttons are `Sense::CLICK` only, so a toast never takes the keyboard from the player.
- **`mascot::watching(ui, rect, Watch, shadow)`** — the mascot reacting to the view: `look_at` (a caret or the thing being read), `eyes_shut`, `puzzled`. Its place is allocated first but it's painted last, so it follows this frame's caret. `mascot::loader` and `mascot::confused` remain for pages.
- **`state::spinner`, `empty`, `error`, `message`** — what a page shows instead of its content, centred in the space it has left. The page mascot (96) always sits in a small glow (`backdrop::glow`, radius 200, centred 18 below it). `spinner` is the loader; `empty` the confused mascot over a `base` `ink_muted` line 16 below; `error` the confused mascot over a danger `notice` at most 420 wide and a `brand` *Retry* (120 wide, `GAP_FIELD` under the notice), returning whether it was pressed; `message` a lone `base` `ink_muted` line (a note with nothing to retry). **`state::icon_message(ui, icon, message)`** is the compact one for a panel or a popover: the icon at 26 in a 48 `inset_hover` circle, the `sm` `ink_muted` line under it, `SPACE.sm` above and below each.

## Anatomy of a screen

The sign-in screen, as the template:

```
backdrop (gradient over the window)
  glow (centred a little below the mascot, radius 360)
  mascot (160, 16 gap above the panel, no shadow — it floats)
  surface
    heading: title / subtitle
    [notice]              — while something failed
    [waiting line]        — while the server is starting
    step content          — rows, fields, code
    brand button          — the one main action
    [quiet button]        — the alternative
  aside under the panel   — subtle link, or a muted note ("Works with Jellyfin 10.11 or newer")
```

The restoring screen is the same backdrop and glow with only the mascot, so a launch that does need the form doesn't flash.

Settings is the full-page template: the panels are the page's sections rather than one form, so the column is left-aligned and the surfaces stack.

```
backdrop (no glow)
  menu column (no line beside it)
    search field (icon inside, clear button at the right while it has text)
    overline per store, then its sections: inset_hover pill on hover, focus and the open one,
      which also gets list_row::open_bar and a brand icon
  section column (760 wide at most, centred in what's left)
    store banner on the backdrop: icon, lg title, muted line (and the server's saving status)
    surface per section, 24 apart, 24 padding (a lone panel's 28 is too roomy stacked)
      xl title in ink + pill::icon_tag (the store's icon and name), left-aligned,
        centred on each other
      rows: base label in ink with its control at the right (select, switch, quiet button),
        sm muted description, xs monospace key
```

The account section shows the user, server and address as one `AvatarRow` that isn't a button target, with the server's version under it.

The top bar and the SyncPlay panel:

```
top bar (72 tall, on the backdrop)
  logo · [Back] · page title (xl)
  search box               — sign-in's field, 40 tall, search icon (field::leading, lg, ink_muted) in its left end,
                             round clear button (field::trailing_button) while it has text
  SyncPlay · Settings      — RoundButton 40/24; SyncPlay on (brand) while in a group
  account                  — RoundButton::avatar, the user's pastel
SyncPlay panel (popup::panel, 340 wide)
  heading: title (label::title at lg) [· status dot + note] / muted line
  not in a group:
    JOIN A GROUP           — overline
    AvatarRow per group    — members' avatars, name, "who · state", brand Join
    New group              — quiet beside groups, brand with none (groups icon + muted line above it)
  in a group:
    WATCHING · n           — avatar_row::person per member, "you" on the user
    QUEUE  [shuffle][repeat] — round toggles 30/18
    list_row::block        — compact ListRows: ▶ in brand or the place number, name; ↑ ↓ ✕ on
                             the active row, focus::ring on the keyboard's
    Leave group            — button::quiet_danger
```

On a phone in portrait (`layout::Size::Compact`, touch under 600 points wide) the bar gives its places to a bottom bar, chosen over a floating bar the page runs under:

```
top bar (56 tall, on the backdrop)
  Back · title (lg)        — or the app's icon at 32 on Home; over Search, and over
                             Settings while its search is open, the search box in the
                             title's place, the buttons gone
  Search · SyncPlay · account
                           — RoundButton 40/24, sensing 48 (layout::grow), 8 apart;
                             Settings is in the account menu
bottom bar (64 tall, on the floor of the window)
  fill                     — backdrop_bottom, down under the gesture bar; a 1 pt
                             surface_border hairline over it
  Home · Libraries         — half the width each: icon 24 over an xs label; the open
                             one's icon brand on an inset_hover pill 56 × 30 and its
                             label ink, the others ink_muted; it slides down out of
                             the window while the page scrolls down (0.2 s, cubic out),
                             back up as it scrolls up or reaches its top
```

Home leaves out *My Media* there, as Libraries is a tap away. Settings is a list of its sections instead of the menu and column: a heading per store (`overline`), then one surface of rows, each its icon in `brand`, its title in `ink` at `base` over a `sm` muted line of what it has, and a chevron in `ink_muted`, at least 64 tall with an inset hairline between them and `inset_hover` while pressed. A row opens its section on a page of its own, titled with its name.

The pages keep 16 from the sides there (`layout::side`); the item page and the player have their own layouts there (below). Touch targets are 48 at least: lists draw their rows that tall, and smaller controls keep their size and sense 48 round their centre.

Menus and popovers open as a bottom sheet with touch (`widgets/sheet.rs`), except the top bar's (the account menu, SyncPlay), which drop down from their buttons as on the desktop, with the sheet's rows and at least 280 wide: a sheet from the bottom would seem to come from nowhere. The sheet: `surface` with its outline and the popup's shadow, `radius_surface` on its top corners only, at most 560 wide and centred, running under the gesture bar with its contents kept above it (or sitting on the soft keyboard). A 32 × 4 handle in `ink_muted` at half strength, then the title at `lg` in `ink`, then the rows: 56 tall, `base` text, 24-point icons, 16 inside an 8 margin, `inset_hover` while pressed, a chosen row's tick in `brand`, separators edge to edge. The page dims to `scrim` as it rises (240 ms, eased out). It scrolls past 85 % of the screen's height; a tap on the page, Back, or a pull down on the handle or title past 30 % of its height (or a flick) closes it.

One main action per view holds here as elsewhere: with groups to join, each row's *Join* is the `brand` one and *New group* is `quiet`; with none, *New group* is `brand`.

The player's settings menu (chosen over a side rail, a drill-down and a single view of everything):

```
popover (over_video, 340 wide, popup::RADIUS, 12 inside, hung from the gear whichever button opened it)
  tabs::bar                — Audio · Subtitles · General (no Subtitles without any)
  Audio, Subtitles:
    track rows             — popup::row, 5 before they scroll, the playing one ticked in BRAND
    separator              — popup::separator_in
    stepper::row           — Delay (Subtitles: with z x), and Subtitles' Size
  General:
    Speed  [ ]             — sm, its keys in a muted xs aside
    segmented::show        — 0.5× to 2×, the chosen one BRAND; "Normal in SyncPlay" in a group
```

The chapters menu shares its frame. The menu changes height between tabs.

The player's prompts and chapters:

```
up-next card (popup::over_video_frame, 320 wide, 16 inside, bottom right)
  UP NEXT                  — label::overline, muted
  name                     — base, strong
  [Play now ◔] [Cancel]    — countdown_pill (BRAND, its ring the time to autoplay) · prompt::quiet
resume prompt              — prompt::note: "Resumed from 12:04" in base, Start over in BRAND
chapters menu (over_video_frame, 6 inside)
  rows                     — media_row::chapter: 8 round a 128 × 72 thumbnail (radius 6),
                             radius 10, 2 apart; name in base over its start in muted sm,
                             centred on the thumbnail; the current one tinted BRAND at 16 %
                             with its name in BRAND
```

The item page:

```
backdrop image fading into the gradient (backdrop::fade_into), edge to edge
poster (or an episode's still) · series line (ink_muted, a link)
  title (40, ink)      brand_icon Play/Resume · quiet_icon squares: from start, shuffle,
                       trailer, Watched and Favorite (toggles), More
  meta, genres, studios (sm, ink_muted) · synopsis (base, ink) · tag pills
version select
Chapters shelf
Seasons: media_row::season rows in columns, the open one with a brand bar
  Season n: a compact MediaRow per episode, in episode_columns
Episodes (a season's page): a compact MediaRow per episode, in episode_columns
Playlist (a playlist's page): a full MediaRow per entry, ↑ ↓ ✕ on the active row
In this collection (a collection): poster cards
Cast & crew · Extras · More like this shelves
```

The item page on a phone (`layout::Size::Compact`, `views/item/compact.rs`), laid out like a card (chosen over a poster beside the title and a tall picture with the title over it):

```
backdrop image in a 300 band, the page's colour at 55 % over it, fading into the gradient
poster 150 × 225 (an episode's still as wide as the page), centred, 12 below the band's top,
  with a soft shadow
centred: series line (link) · title (26, ink) · meta (sm, ink_muted) · genres joined by · (sm)
Play/Resume            — brand_icon_wide across the page; under Resume a 4-point brand bar of
                         how far it is and "1h 14m left" (xs, ink_muted)
round actions          — From start, Shuffle, Trailer, Watched, Favorite, More: a 48 circle
                         (surface, surface_border; inset_hover with brand outline and icon
                         while on) with its name in xs under it, 72 cells centred in a row
synopsis               — base, 4 lines, then More / Less in brand (sm, strong)
Director · Writers · Studio — sm rows, the name in a 76 muted column
version                — the label in that column, the select across the rest, 40 tall
Seasons (a series)     — a shelf of season posters (0.6 of a poster), the open one ringed in
                         brand with its name in brand, "n to watch" (xs) under each; the next
                         episode's season is open from the start and a tap on it keeps it open
  Season n             — heading, then a row per episode: still 136 × 76.5 (radius 6, progress
                         bar), "2. Name" (base, 2 lines) over the runtime (sm) beside it, the
                         Watched toggle 36/20 at the right, two lines of overview (sm, ink_muted)
                         under the row; no fill at rest, inset_hover while pressed; 18 apart
Cast & crew            — round faces 64 across, the name (sm) and role (xs) centred under them
More like this         — posters at 0.7, 12 apart
```

The player on a phone (touch, `views/player/phone.rs`), over the video edge to edge, the controls inside the safe area (the larger side inset on both sides, then 12), chosen over a centre cluster with the menus at the top and a labelled row under the seek bar:

```
veil                   — over_video::VEIL over the picture, SCRIM down from the top and
                         SCRIM_STRONG up from the bottom
top                    — Back 44 · title (lg) over the episode line (sm, MUTED_FOREGROUND)
                         ……… [SyncPlay] [Transcoding] · Chapters · Subtitles (a toggle, BRAND
                         while on; a long press opens its tracks) · Settings (tune), 44/24
middle                 — play/pause 72 on a SURFACE_SUBTLE disc; the 10-second skips 56 at
                         ±112; previous and next episode 48 at ±212 (closer on a narrow screen)
bottom                 — the clock (sm, a tap switches to the time left) and the speed badge,
                         over the seek bar in a 40 row across the safe area
skip pill              — at the safe area's right, over the clock while the controls show, at
                         its bottom when they don't
up next                — in the skip pill's place, one row: "Up next" over the name (160
                         wide) beside Play now and Cancel, 8 inside, so it fits under the
                         middle's buttons
side sheet             — an egui::Modal at the right, 360 wide and the full height, MENU with
                         MENU_BORDER, 16 radius on its left corners, 16 inside, SHEET_BACKDROP
                         beside it: the title (lg) and a close button 36, then the settings
                         menu's tabs and rows (General with the speeds, then VERSION and the
                         versions) or the chapters' rows
```

The library page:

```
toolbar (surface::strip, 24 from the window's sides, 16 above it and above the grid)
  Progress · Rating · Genres and tags · Year · Resolution
                           — select boxes, 32 tall; Genres and tags is a select's box too
                             (its summary, a caret), opening the popover
  Clear filters            — button::text: brand, dimmed while there's nothing to clear
  n items                  — label::muted
  Sort [Name ▾] (↑)        — label::muted, a select, the order as RoundButton 32/20
jump bar (right edge, while sorted by name)
  # A B …                  — ink_muted at 11; the first row's letter (by its first card's name,
                             past a leading article) brand at 13; an inset_hover pill in ink
                             under the pointer; each a Button "Jump to X", the current selected;
                             a drag along it shows the letter it's on in a brand circle 56
                             across, 24 left of the bar at the finger's height, the letter in
                             brand_foreground at 2xl
phone toolbar (layout::Size::Compact)
  Filters                  — button::quiet_icon with the tune icon, on (brand) while
                             anything is filtered; opens a dialog of the filters over the
                             page frosted, one 48-tall row each (the name, its select at
                             the right), with Done (brand) and Clear filters (quiet) under
                             them and the count under its title. Genres and tags is a row
                             of its own: what's chosen in ink_muted at sm and a chevron
                             at the right, the whole row a button (inset_hover under the
                             finger) turning the dialog to their page: Back and the title,
                             "n chosen · n items" under it, the popover's contents with
                             the list as tall as the window leaves (window less 420, at
                             least 160), the search box not taking the keyboard until
                             tapped, then Done and Clear (genres and tags only)
  n items · [Name ▾] (↑)  — the count, then the sort without its label
genres and tags popover (popup::panel, 380 wide)
  search                   — field::with_leading + field::leading, search icon in its left end
  chosen pills             — the ones picked so far
  GENRES / TAGS            — overlines over the pills
  pills                    — 12 inset, full radius: inset + surface_border off (inset_hover
                             under the pointer), brand + tick included, inset + danger outline,
                             danger text and minus excluded; focus::ring outside the highlighted
```

The strip is the one surface on a browsing page: it sets the controls apart from the posters, whose own pictures are busy enough that selects standing on the backdrop among them read as part of the grid. The selects are insets sunk into it, as fields are in a panel.

## Behaviour

- **Focus.** Check focus by the focused widget's id (`ctx.memory(|m| m.focused())`), not `Response::has_focus`, which is false while the window lacks the OS's focus. Fields show focus as a 1 pt `brand` outline (as wide as the resting outline, so the field doesn't shrink on focus); everything else takes `focus::ring`.
- **Keyboard.** Enter in a field advances to the next field or runs the main action. A step that needs a field focuses it.
- **Busy.** Nothing awaits; while a request runs its button loads, the others and the fields are disabled, and the error clears.
- **Errors.** One message at a time, in a danger notice under the heading; the mascot looks confused while it shows.
- **Animations off.** The mascot holds its rest pose and jumps to its targets; nothing asks for repaints.
- **Accessibility.** Painted widgets give AccessKit a role and label (`widget_info`): rows are buttons named by the account, the code is a label with the code itself, the copy button says "Copy code"/"Copied".

## Testing

Headless UI tests of the screens are in `crates/app/src/ui_tests/` (egui_kittest); hardware checks are in `docs/TESTING.md`. The `ui-mockup` skill renders a widget or page as an egui screenshot, which is the quickest way to try a variant of the look before building it.
