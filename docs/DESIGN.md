# The cozy look

The design reference for Tabbyfin's new look. The sign-in and restoring screens are the first in it (2026-10-29/30, commits `e70588c`, `4d8c3d5`, `59fc96d`); this doc writes down what they settled so the next screens start from it, and lists what's still open. It's a draft to refine as more screens move over: when a screen settles something new, add it here.

Since C1 the whole app draws in the cozy palette: `theme::visuals` (egui's colours for every page) is built from the cozy tokens, and gpui-kit's token names are aliases of them (see *Colour tokens*). The other pages still have gpui-kit's layout and widgets, and they move to the look's structure one at a time.

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
| `avatars[5]` | lavender, periwinkle, pink, mint, peach | same | initial circles, picked by a hash of the name |
| `avatar_foreground` | `#1d1230` | `#1d1230` | initial on the pastel |
| `danger` (existing) | `#ef4444` | `#f87171` | danger notice |
| `info` (existing) | `#06b6d4` | `#22d3ee` | info notice |
| `success` (existing) | `#22c55e` | `#4ade80` | success notice |

gpui-kit's names, which `main` and the pages not yet restructured draw with, are aliases of these (`Tokens::cozy_aliases`), so a page that reads `primary` gets `brand`:

| gpui-kit name | Is | Where it shows |
|---|---|---|
| `background` | `backdrop_top` | the page and top bar's flat fill, an item banner's fade |
| `foreground`, `accent_foreground` | `ink` | text |
| `muted_foreground`, `tab_foreground` | `ink_muted` | subtitles, hints, card subtitles |
| `primary`, `primary_hover`, `primary_foreground` | `brand`, `brand_hover`, `brand_foreground` | primary buttons, the card progress bar and played badge |
| `popover`, `button` | `surface` | menus, dialogs, buttons at rest |
| `muted`, `accent`, `button_hover`, `button_active`, `secondary_active` | `inset_hover` | chips, image placeholders, highlighted rows, hovered buttons |
| `secondary`, `tab_bar` | `inset` | checkbox boxes, slider rails, tab tracks |
| `border`, `input` | `surface_border` | outlines |
| `radius` | `radius_control` (12) | controls |

`overlay`, `selection`, the status colours and `radius_lg` (12, for cards and popups) keep their own values. The page's flat fill is `backdrop_top`, the gradient's top end, not the lilac bottom: in dark the bottom end (`#1d1633`) is lighter than `surface` (`#1a1526`), so raised things would look sunk, and in light `inset_hover` rows and `surface_border` outlines disappear against it. The gradient itself goes under the shell with C2. The focus ring (egui's `selection.stroke`) and the cursor rings are `brand`. The player keeps gpui-kit's dark set over the video (`theme::over_video::TOKENS`).

Light `brand` is deeper than the mascot's lavender so white text passes AA (5.9:1); dark uses the mascot's lavender itself with dark text. The status colours (`danger`, `info`, `success`) are too pale for text on a light surface (green and cyan about 2.3:1 on white), so they tint a notice's box, outline and icon, and its text stays `ink`.

## Type

The app's text table (`theme::text`, scaled by the text-size setting). Cozy screens use:

| Style | Size | Use |
|---|---|---|
| `2xl` | 24 | surface title |
| `lg` | 18 | icons inside fields |
| `base` | 16 | body, field text, button labels, row titles, surface subtitle |
| `sm` | 14 | field labels, waiting line, asides and subtle links |
| `xs` | 12 | row subtitles |
| monospace 36 | — | a code to read off (Quick Connect) |

Titles are `ink`; everything secondary is `ink_muted`. Get fonts with `text::font(ui, "<name>")`, not literal sizes (the code widget is the one fixed-size exception).

## Shape and spacing

- Radii: `radius_surface` 22 for panels, `radius_control` 12 for everything on them (fields, buttons, rows, insets, notices).
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

- **`surface::frame(ui)`** — the panel. **`surface::heading(ui, title, subtitle)`** — centred `2xl` title over a `base` muted line; every panel starts with one. The title names the step ("Sign in to *server*", "Approve this sign-in", "Welcome back"); the subtitle says what to do or where you are.
- **`surface::waiting(ui, line)`** — small spinner and a muted `sm` line, centred together. For a wait the user can't act on ("Waiting for approval…", "server is starting").
- **`field::label` + `field::single_line`** — muted `sm` label above a full-width `base` field. Hint text only while empty *and* unfocused (egui's hint next to the caret reads as typed text). **`field::with_trailing` + `field::trailing`** — an icon button inside the field's right end (show/hide password): painted, muted at rest, `ink` over an `inset_hover` circle on hover.
- **`button::brand`** — the main action, full width, `brand` fill. **`button::quiet`** — same size, outlined on the surface. **`button::subtle`** — muted, underlined `sm` link with no frame, for asides under the panel ("Use another server", "Cancel"). Buttons take a `State`: `Ready`, `Disabled`, or `Loading` (disabled, spinner at the right end, label doesn't move). A view with several actions runs them through one helper so the running one loads and the others disable. `brand` and `quiet` also take a `Focus`: `Tab` (a Tab stop, pressed by Space or Enter, as on sign-in) or `Skip` (clicks only, `Sense::CLICK`), which dialogs use because their keys are their own (`widgets/dialog.rs`).
- **`avatar::paint`** — a pastel circle with the name's initial; the colour is stable per name across launches.
- **`AvatarRow`** — avatar, title, muted subtitle and an optional ✕, as one focusable button (the ✕ is a second focus stop). Inset fill, `inset_hover` on hover/focus, 1.5 pt `brand` ring on focus.
- **`code::show`** — a code to read off: inset, 36 pt monospace split in two halves, copy button at the right end that turns into a check mark for 1.5 s.
- **`surface::notice(ui, Kind, text)`** — a message under the heading: a box tinted with the kind's colour at 10 %, a 1 pt outline in it, `radius_control`, 12 × 9 margin, then the kind's icon (the toasts' icons, `lg`) and the message in `ink` at `base`, wrapping under itself rather than under the icon. Kinds: `Danger` (something failed), `Info` (something to know), `Success` (something worked). The icon names the kind to AccessKit ("Error", "Information", "Success"), so the kind isn't told by colour alone.
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

## Behaviour

- **Focus.** Check focus by the focused widget's id (`ctx.memory(|m| m.focused())`), not `Response::has_focus`, which is false while the window lacks the OS's focus. The focus ring is `brand`, 1 pt on fields (as wide as the resting outline, so the field doesn't shrink on focus) and 1.5 pt on rows.
- **Keyboard.** Enter in a field advances to the next field or runs the main action. A step that needs a field focuses it.
- **Busy.** Nothing awaits; while a request runs its button loads, the others and the fields are disabled, and the error clears.
- **Errors.** One message at a time, in a danger notice under the heading; the mascot looks confused while it shows.
- **Animations off.** The mascot holds its rest pose and jumps to its targets; nothing asks for repaints.
- **Accessibility.** Painted widgets give AccessKit a role and label (`widget_info`): rows are buttons named by the account, the code is a label with the code itself, the copy button says "Copy code"/"Copied".

## Applying it to other pages

The rollout plan is `docs/ROADMAP.md`'s [cozy look backlog](ROADMAP.md#cozy-look-backlog) (C0–C13). It starts with one pass that retints the whole app through egui's `Visuals`, pulls the widgets pages draw for themselves into shared components, then restructures one screen at a time. Below is where it currently stands on the open questions. These are proposals until each item's mockup confirms them, and when one is settled, move it into the sections above.

- **Components.** Views compose and widgets draw: anything another page could use lives in `widgets/` and in the component gallery (C3).
- **Order.** The component library, then shared pieces (popups, selects, toasts, dialogs), then Settings as the first full page, then the top bar, then the browsing pages. The player goes last and changes least.
- **Backdrop on browsing pages.** Yes, the gradient goes under the whole shell, without the glow. The top bar sits on the backdrop with no fill.
- **Cards and shelves.** Cards stay image-first: no surface around them, image radius `radius_lg` (22 is too round for posters). The cursor ring, progress and played badge use `brand`.
- **Mascot and glow.** Sign-in, restoring, loaders and empty states only. The glow always goes with the mascot.
- **Selects, toggles, menus.** Inset boxes and `inset_hover` rows, with a cozy toggle/checkbox (C3, C4).
- **The player.** Stays dark (`theme::over_video`) and only borrows `brand` for its accents.
- **Retiring gpui-kit's tokens.** They're aliases since C1; C13 removes them.

## Testing

Headless UI tests for the sign-in screen are in `crates/app/src/ui_tests/login.rs` (egui_kittest); hardware checks are in `docs/TESTING.md`. The `ui-mockup` skill renders a widget or page as an egui screenshot, which is the quickest way to try a variant of the look before building it.
