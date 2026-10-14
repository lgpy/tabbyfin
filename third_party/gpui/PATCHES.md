# Vendored GPUI crates

These are copies of GPUI crates from crates.io, at the exact version gpui-kit pins. They
carry small patches for in-app video (`docs/in-app-video.md`) and are applied over the
crates.io versions with `[patch.crates-io]` in the workspace `Cargo.toml`.

| Crate | Version | Why |
|---|---|---|
| `gpui-pre` | 0.3.9 | `SurfaceSource::SharedTexture` / `Window::paint_shared_texture` on Windows |
| `gpui-pre-windows` | 0.3.9 | DirectX renderer draws `PaintSurface`s from shared D3D11 textures |

Trimmed from the crates.io copies: `gpui-pre/examples/` (only used by tests) and the
`Cargo.lock` files.

**Upgrading gpui-kit:**
1. Copy the new versions from `~/.cargo/registry/src/*/`.
2. Re-apply the changes from the commits that touch this folder. Each one is described below.
3. Bump the versions here.

## Patches

1. **`gpui-pre`: Windows shared-texture surfaces.**
   - Adds `SharedTexture` (share handle and size) and a `PaintSurface.shared_texture` field on Windows.
   - Adds `SurfaceSource::SharedTexture`, with `surface()` enabled on Windows.
   - Adds `Window::paint_shared_texture`.
2. **`gpui-pre-windows`: drawing surfaces.** *Not done yet.* `draw_surfaces` is still the upstream stub. See `docs/in-app-video.md`, phase 2.
