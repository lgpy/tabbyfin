# The cozy look

The design reference for Tabbyfin's new look. The sign-in and restoring screens are the first in it (2026-10-29/30, commits `e70588c`, `4d8c3d5`, `59fc96d`); this doc writes down what they settled so the next screens start from it, and lists what's still open. It's a draft to refine as more screens move over: when a screen settles something new, add it here.

Since C1 the whole app draws in the cozy palette: `theme::visuals` (egui's colours for every page) is built from the cozy tokens, and the classic token names are aliases of them (see *Colour tokens*). Since C2 the backdrop's gradient runs under the whole shell as well. The other pages still have their classic layout and widgets, and they move to the look's structure one at a time.

## Principles

- **Warm, soft, quiet.** The mascot's lavender instead of neutral greys; large radii; soft shadows instead of hard outlines. Nothing shouts except the one main action.
- **One main action per view.** It's the only `brand` button. Everything else is `quiet` (a real alternative) or `subtle` (a way out or aside, under the panel).
- **Layers, not lines.** Three depths: the *backdrop* (the window), a *surface* raised off it, and *insets* sunk into the surface (fields, rows, codes). Depth is shown by fill and shadow; borders are 1 pt and close to the fill.
- **The mascot is part of the UI.** Where it appears it reacts to what's happening (watching the caret, eyes shut for a password, confused over an error), and the glow behind it lights the panel below.
- **Both themes are designed, not derived.** Light is "a pale lilac morning", dark "a nightlight: deep plum, lit from behind the mascot". Each token has a value per theme, chosen by eye and checked for contrast.
- **Every widget reads its colours from `theme::tokens_of(ui)`.** No literal colours in views.

## Layers

| Layer | What it is | Drawn by |
|---|---|---|
| Backdrop | Vertical gradient `backdrop_top` → `backdrop_bottom` over the whole window, plus an optional soft `glow` pooled behind the mascot | `widgets::backdrop::fill`, `backdrop::glow` (meshes; egui has no gradients) |
| Surface | A panel: `surface` fill, 1 pt `surface_border`, `radius_surface` (22), 28 pt padding, shadow offset 0/12, blur 40 in `surface_shadow` | `widgets::surface::frame(ui)` |
| Inset | Sunk into a surface: `inset` fill, `inset_hover` under the pointer, `radius_control` (12) | text fields (`theme::visuals`), `AvatarRow`, `code::show` |

The CentralPanel holding a cozy screen is frameless (`Frame::NONE`) so the backdrop covers the window.

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
| `ink` | `#1f182b` | `#f4f0fa` | text on cozy surfaces |
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
| `danger` (existing) | `#ef4444` | `#f87171` | danger notice |
| `info` (existing) | `#06b6d4` | `#22d3ee` | info notice |
| `success` (existing) | `#22c55e` | `#4ade80` | success notice |

The classic names, which `main` and the pages not yet restructured draw with, are aliases of these (`Tokens::cozy_aliases`), so a page that reads `primary` gets `brand`:

| Classic name | Is | Where it shows |
|---|---|---|
| `background` | `backdrop_top` | what a card's hover dims with, and flat fills off the shell (the player's pills) |
| `foreground`, `accent_foreground` | `ink` | text |
| `muted_foreground`, `tab_foreground` | `ink_muted` | subtitles, hints, card subtitles |
| `primary`, `primary_hover`, `primary_foreground` | `brand`, `brand_hover`, `brand_foreground` | primary buttons, the card progress bar and played badge |
| `popover`, `button` | `surface` | menus, dialogs, buttons at rest |
| `muted`, `accent`, `button_hover`, `button_active`, `secondary_active` | `inset_hover` | chips, image placeholders, highlighted rows, hovered buttons |
| `secondary`, `tab_bar` | `inset` | checkbox boxes, slider rails, tab tracks |
| `border`, `input` | `surface_border` | outlines |
| `radius` | `radius_control` (12) | controls |
| `overlay` | `scrim` | the page under a dialog |

`selection`, the status colours and `radius_lg` (12, for cards and popups) keep their own values. The page's flat fill is `backdrop_top`, the gradient's top end, not the lilac bottom: in dark the bottom end (`#1d1633`) is lighter than `surface` (`#1a1526`), so raised things would look sunk, and in light `inset_hover` rows and `surface_border` outlines disappear against it. Since C2 the shell is on the gradient itself, so `background` no longer fills a page; it stays `backdrop_top` for what still reads it. The gradient's lilac end passes for text: `ink_muted` (card subtitles, shelf titles) is 5.6:1 on light `backdrop_bottom` and 6.9:1 on dark, `ink` 14:1 and 15:1. The focus ring (egui's `selection.stroke`) and the cursor rings are `brand`. The player keeps the classic dark set over the video (`theme::over_video::TOKENS`).

Light `brand` is deeper than the mascot's lavender so white text passes AA (5.9:1); dark uses the mascot's lavender itself with dark text. The status colours (`danger`, `info`, `success`) are too pale for text on a light surface (green and cyan about 2.3:1 on white), so they tint a notice's box, outline and icon, and its text stays `ink`.

## Type

The app's text table (`theme::text`, scaled by the text-size setting). Cozy screens use:

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

All in `crates/app/src/widgets/`.

- **`surface::frame(ui)`** — the panel (`frame_of(tokens)` where there's no `Ui` yet). **`surface::heading(ui, title, subtitle)`** — centred `2xl` title over a `base` muted line; every panel starts with one. The title names the step ("Sign in to *server*", "Approve this sign-in", "Welcome back"); the subtitle says what to do or where you are. **`surface::heading_left(ui, title, Option<subtitle>)`** — the same against the left edge, for a surface that isn't a centred form (a dialog), the subtitle only when there's something to explain.
- **`surface::waiting(ui, line)`** — small spinner and a muted `sm` line, centred together. For a wait the user can't act on ("Waiting for approval…", "server is starting").
- **`field::label` + `field::single_line`** — muted `sm` label above a full-width `base` field. Hint text only while empty *and* unfocused (egui's hint next to the caret reads as typed text). **`field::with_trailing` + `field::trailing`** — an icon button inside the field's right end (show/hide password), named and tooltipped by its label: a `RoundButton` 28 across with the icon at `lg`. `field::trailing_button` and `trailing_center` give the same button for a caller that sets its focus (the search box's clear button skips it).
- **`button::brand`** — the main action, full width, `brand` fill. **`button::quiet`** — same size, outlined on the surface; **`button::quiet_danger`** is the same with its label in `danger`, for leaving or losing something that isn't the view's main action (*Leave group*). **`button::subtle`** — muted, underlined `sm` link with no frame, for asides under the panel ("Use another server", "Cancel"). Buttons take a `State`: `Ready`, `Disabled`, or `Loading` (disabled, spinner at the right end, label doesn't move). A view with several actions runs them through one helper so the running one loads and the others disable. `brand` and `quiet` also take a `Focus`: `Tab` (a Tab stop, pressed by Space or Enter, as on sign-in) or `Skip` (clicks only, `Sense::CLICK`), which dialogs use because their keys are their own (`widgets/dialog.rs`). **`button::danger`** — `brand`'s shape in `danger_action`, for a main action that deletes something.
- **`dialog::show`** — a dialog: an `egui::Modal` in a surface frame over the page dimmed with `scrim`, `heading_left`, `GAP_GROUP` under it. **`dialog::buttons`** puts its buttons at the bottom right, `GAP_ITEM` apart: `dialog::main` (`brand`) or `dialog::danger`, then `dialog::cancel` (`quiet`), all `Focus::Skip` and as wide as their label plus room for the loading spinner (96 at least). A list in a dialog (*Add to playlist*) is inset rows: `inset` at rest, `inset_hover` on the row Enter would pick, with the focus ring inside it while the keyboard put it there.
- **`avatar::paint`** — a pastel circle with the name's initial; the colour is stable per name across launches.
- **`AvatarRow::new(names, title, subtitle, label)`** — avatar, title, muted subtitle, as one button named by `label`. Inset fill, `inset_hover` on hover/focus, the focus ring (inside) on focus or `.highlighted(true)` (a list that keeps its own keyboard). Several names overlap as 14-point avatars (up to three, each parted from the one under it by a ring of the row's fill) for a group. `.status(colour)` puts a dot before the subtitle. At the right end, `.remove(tooltip)` adds a ✕ (a second focus stop), or `.action(label)` a compact `brand` button (30 tall, radius 10, `brand_hover` with the row) that is part of the row: a click anywhere on it presses it. `.focus(Focus::Skip)` for clicks only. **`avatar_row::person(ui, name, aside)`** — a person in a list that isn't a list of buttons: a 14-point avatar, the name in `ink` at `base` and an optional `xs` muted aside ("you"), 36 tall, a `ListItem` to AccessKit.
- **`RoundButton`** (`round_button.rs`) — a round button holding an icon (`RoundButton::icon`) or a person's avatar (`RoundButton::avatar`, 4 points inside the circle): no frame at rest and the icon in `ink_muted`; under the pointer an `inset_hover` circle and `ink`. `.on(bool)` makes it a toggle (selected to AccessKit): on keeps the circle with the icon in `brand`. Disabled is dimmed `ink_muted` at 45 %. `.ringed(true)` (or egui's focus) puts `focus::ring` round it, a circle round the circle (round the avatar for an avatar). Named and tooltipped by its label. Sizes: 40 with a 24-point icon (the top bar), 30/18 (the SyncPlay panel's toggles), 28/`lg` (a field's trailing button), 26/16 (a queue row's buttons). `round_button::glyph` centres an icon on its own bounds, which the icon font needs. Picked by a mockup, 2026-11-01.
- **Cards** (`card.rs`) — image-first, with no surface round them: the picture at radius 12 (`image::RADIUS`; a panel's 22 is too round for posters), the title in `ink` at `sm` and a muted `xs` line under it. The played badge, the progress bar's fill and the cursor's ring (`focus::ring`, inside) are `brand`. A person card without a picture is their `avatar::colour` pastel with the initials at `2xl` in `avatar_foreground`, so a cast member has the colour their avatar has anywhere else.
- **`shelf`** (`shelf.rs`) — a titled row of cards: the title `xl` in `ink` (a chevron after it in `ink_muted` when it leads to a page), and at the row's right end two `RoundButton` arrows, 32 across with 20-point icons, shown once the row overflows and dimmed at its end. The title row is the arrows' 32 tall.
- **`library_tile::show`** — a library as a small surface, Home's *My Media* row (C9, picked by a mockup over image cards): 236 × 76, `surface` with its `surface_border` outline (`ink_muted` under the pointer), radius 16 and the popups' shadow; the library's picture as a 96 × 56 thumbnail at radius 10, or its kind's icon in `brand` on `inset_hover` when it has none, then its name in `ink` at `base`, cut short when it doesn't fit. A button named by the library; the cursor's ring is inside it. Libraries read as places to go, not things to watch.
- **`code::show`** — a code to read off: inset, 36 pt monospace split in two halves, copy button at the right end that turns into a check mark for 1.5 s.
- **`surface::notice(ui, Kind, text)`** — a message under the heading: a box tinted with the kind's colour at 10 %, a 1 pt outline in it, `radius_control`, 12 × 9 margin, then the kind's icon (the toasts' icons, `lg`) and the message in `ink` at `base`, wrapping under itself rather than under the icon. Kinds: `Danger` (something failed), `Info` (something to know), `Success` (something worked). The icon names the kind to AccessKit ("Error", "Information", "Success"), so the kind isn't told by colour alone.
- **`focus::ring(ui, rect, radius, Place)`** — the keyboard's ring, wherever the keyboard (egui focus or a page's cursor) sits on something: 2 pt `brand`, the same on every widget and page. `Place::Outside` for free-standing controls (bar buttons, selects, pills, the item page's actions): 2 pt clear of the control, with the control's radius plus the gap, so a circle gets a circle and a pill a pill. `Place::Inside` for rows and cards, on their own edge, since the lists and scroll areas they sit in would clip a ring outside them. **`focus::ring_widget(ui, &response)`** rings an egui `Button` or `TextEdit` on the theme's widget radius. Picked over the old per-page rings (1–3 pt, `brand` or `ink`, square rings round round buttons) by a mockup, 2026-11-01.
- **`label::muted`, `note`, `title`, `overline`** — the text styles beyond body text, as `RichText`: `muted` is `sm` `ink_muted` (a waiting or empty line, a field's label, an aside), `note` `xs` `ink_muted` (a panel's status or hint), `title` `base` strong `ink` (a panel's or group's title), `overline` `xs` strong capitals in `ink_muted` (a heading over a menu's or a panel's section). Off the cozy inks (the player), recolour with `.color(…)`.
- **Popups** (`popup::PopupMenu`, `popup::panel`, tooltips, toasts) — a `surface` with its `surface_border` outline and `theme::popup_shadow`, the surface's shadow colour at 0 × 6 offset and 20 blur (a surface's is 12 and 40). **`popup::frame(tokens)`** is that frame without radius or margin. Menus: radius 16, 6 pt round the rows, the highlighted row an `inset_hover` pill concentric with the panel (radius 10), a danger row's highlight `danger` at 14 %, separators in `surface_border`. Tooltips are egui's `Frame::popup`, which `theme::visuals` gives the same frame at radius 10 with a 10 × 6 margin; dialogs set their own frame on the `Modal` so they don't take the tooltip's. Picked by a mockup, 2026-11-01. **`popup::panel`** is a popup laid out like a small surface rather than a list of rows (the SyncPlay panel): the menus' frame and placement with `popup::PANEL_PAD` (16) inside.
- **`toggle::switch(ui, &mut on, name)`** — an on/off switch, 40 × 22: a pill track sunk like an inset while off (`inset`, `surface_border` outline, `ink_muted` outline and `inset_hover` under the pointer) and filled with `brand` while on (`brand_hover` under the pointer), with a round knob 3 pt inside it, `ink_muted` off and `brand_foreground` on, so the state reads without colour too. The knob slides over egui's animation time (none with the animations setting off). Focusable: Space or Enter flips it, and `focus::ring` sits outside it. A `Switch` to AccessKit, named by the setting, with its state. Settings' flags are switches (C6). **`toggle::row(ui, on, label, Focus, ringed)`** is the switch with its label across an inset row (`inset`, `inset_hover` on hover or while `ringed`, `radius_control`, 12 × 9 padding): the label in `ink` at `base` at the left, the track at the right, the whole row the target and one `Switch` to AccessKit. It doesn't flip itself, so the caller can refuse while busy; `Focus::Skip` keeps it out of egui's focus for a dialog that moves its own highlight (`ringed` then draws `focus::ring` inside it). The playlist dialog's sharing is one.
- **`select::select` / `select::button`** — sunk like a field: `inset` fill, `surface_border` outline that darkens to `ink_muted` under the pointer, `radius_control`, the value in `ink` at `sm` and a muted caret; the keyboard gets `focus::ring` outside it. Its height is the caller's (32 by default, the toolbar's).
- **Toasts** (`toasts.rs`) — 340 wide in the bottom-right corner, 8 apart, each a popup surface with 14 pt padding: the kind's icon (20 pt, named to AccessKit like a notice's) on the first line, the title in `ink` at `base` over the text in `ink_muted` at `sm` (the text alone is in `ink`), and at the right end, centred on the toast, the action as a compact `brand` button (30 tall, radius 10) and a round close button that fills `inset_hover` on hover. Both buttons are `Sense::CLICK` only, so a toast never takes the keyboard from the player.
- **`mascot::watching(ui, rect, Watch, shadow)`** — the mascot reacting to the view: `look_at` (a caret or the thing being read), `eyes_shut`, `puzzled`. Its place is allocated first but it's painted last, so it follows this frame's caret. `mascot::loader` and `mascot::confused` remain for pages.

## Anatomy of a cozy screen

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

Settings is the full-page template (C6): the panels are the page's sections rather than one form, so the column is left-aligned and the surfaces stack.

```
backdrop (no glow)
  menu column (no line beside it)
    search field (icon inside, clear button at the right while it has text)
    overline per store, then its sections: inset_hover pill on hover, focus and the open one,
      which also gets a 3 pt brand bar and a brand icon
  section column (760 wide at most, centred in what's left)
    store banner on the backdrop: icon, lg title, muted line (and the server's saving status)
    surface per section, 24 apart, 24 padding (a lone panel's 28 is too roomy stacked)
      icon in ink_muted + xl title in ink, left-aligned
      rows: base label in ink with its control at the right (select, switch, quiet button),
        sm muted description, xs monospace key
```

The account section shows the user, server and address as one `AvatarRow` that isn't a button target, with the server's version under it.

The top bar and the SyncPlay panel (C7, picked by a mockup on 2026-11-01):

```
top bar (72 tall, on the backdrop)
  logo · [Back] · page title (xl)
  search box               — sign-in's field, 40 tall, search icon (lg + 2, ink_muted) in its left end,
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
    inset block of rows    — ▶ in brand or the place number, name; round ↑ ↓ ✕ on the active row
    Leave group            — button::quiet_danger
```

One main action per view holds here as elsewhere: with groups to join, each row's *Join* is the `brand` one and *New group* is `quiet`; with none, *New group* is `brand`.

## Behaviour

- **Focus.** Check focus by the focused widget's id (`ctx.memory(|m| m.focused())`), not `Response::has_focus`, which is false while the window lacks the OS's focus. Fields show focus as a 1 pt `brand` outline (as wide as the resting outline, so the field doesn't shrink on focus); everything else takes `focus::ring`.
- **Keyboard.** Enter in a field advances to the next field or runs the main action. A step that needs a field focuses it.
- **Busy.** Nothing awaits; while a request runs its button loads, the others and the fields are disabled, and the error clears.
- **Errors.** One message at a time, in a danger notice under the heading; the mascot looks confused while it shows.
- **Animations off.** The mascot holds its rest pose and jumps to its targets; nothing asks for repaints.
- **Accessibility.** Painted widgets give AccessKit a role and label (`widget_info`): rows are buttons named by the account, the code is a label with the code itself, the copy button says "Copy code"/"Copied".

## Applying it to other pages

The rollout plan is `docs/ROADMAP.md`'s [cozy look backlog](ROADMAP.md#cozy-look-backlog) (C0–C13). It starts with one pass that retints the whole app through egui's `Visuals`, pulls the widgets pages draw for themselves into shared components, then restructures one screen at a time. Below is where it currently stands on the open questions. These are proposals until each item's mockup confirms them, and when one is settled, move it into the sections above.

- **Components.** Views compose and widgets draw: anything another page could use lives in `widgets/` and in the component gallery (C3).
- **Order.** The component library, then shared pieces (popups, selects, toasts, dialogs), then Settings as the first full page, then the top bar, then the browsing pages. The player goes last and changes least.
- **Mascot and glow.** Sign-in, restoring, loaders and empty states only. The glow always goes with the mascot.
- **Toggles.** The switch landed with Settings (C6, see *Components*); the dialog's sharing is a switch row. Lists that tick several things (genres, tags) keep `popup::row`'s ticks. Selects and menus settled in C4.
- **The player.** Stays dark (`theme::over_video`) and only borrows `brand` for its accents.
- **Retiring the classic tokens.** They're aliases since C1; C13 removes them.

## Testing

Headless UI tests for the sign-in screen are in `crates/app/src/ui_tests/login.rs` (egui_kittest); hardware checks are in `docs/TESTING.md`. The `ui-mockup` skill renders a widget or page as an egui screenshot, which is the quickest way to try a variant of the look before building it.
