# Third-party software

Tabbyfin itself is licensed under the GNU General Public License, version 3 or later (`LICENSE`). Source: https://github.com/lgpy/tabbyfin.

Tabbyfin plays video with **libmpv**, the library form of mpv, running inside the app.

- **Windows:** the package includes `libmpv-2.dll`, a build from
  https://github.com/shinchiro/mpv-winbuild-cmake. The version is pinned in `packaging/mpv.lock`.
- **Linux:** uses the system libmpv (`libmpv2`).
- **macOS:** uses Homebrew's libmpv (`brew install mpv`) for now.

**License:** GPL-2.0-or-later for these builds, see https://github.com/mpv-player/mpv/blob/master/Copyright.

**Source code:** https://github.com/mpv-player/mpv. The shinchiro build records the exact mpv commit in its file name.

## The UI (all packages)

The UI and renderer are the Rust **egui** stack, statically linked into the app:
[egui](https://github.com/emilk/egui) (`egui`, `eframe`, `egui-wgpu`, `epaint`) and
[wgpu](https://wgpu.rs), under MIT OR Apache-2.0, plus `egui_material_icons`
(https://github.com/lucasmerlin/hello_egui), under MIT. The exact versions are in `Cargo.lock`.

The binary embeds egui's default fonts and the Material Symbols icons. Their licences require
the licence texts to be handed out with them, so every package carries them in `licenses/`
(the `.deb` and Flatpak keep them beside this file):

| Files | Project | License | Text |
|---|---|---|---|
| `Ubuntu-Light.ttf` | Ubuntu font family, https://design.ubuntu.com/font | Ubuntu Font Licence 1.0 | `licenses/Ubuntu-Font-Licence-1.0.txt` |
| `Hack-Regular.ttf` | Hack, https://sourcefoundry.org/hack | MIT (Bitstream Vera License for the glyphs it inherits) | `licenses/Hack.txt` |
| `egui-icons.ttf`, a subset of emoji-icon-font | https://github.com/jslegers/emoji-icon-font | MIT | `licenses/emoji-icon-font.txt` |
| `MaterialSymbolsRounded-Regular.ttf`, `MaterialSymbolsRounded_Filled-Regular.ttf` | Material Symbols, https://fonts.google.com/icons | Apache-2.0 | `licenses/Material-Symbols.txt`, `licenses/Apache-2.0.txt` |

## ANGLE (Windows)

The Windows package also includes **ANGLE** (`libEGL.dll`, `libGLESv2.dll`), which runs OpenGL ES on Direct3D 11. mpv draws through it so hardware-decoded video stays on the GPU. It's MSYS2's build (https://packages.msys2.org/base/mingw-w64-angleproject), pinned in `packaging/angle.lock`, along with the MinGW runtime DLLs it needs. Their license texts are in `licenses/`.

| Files | Project | License |
|---|---|---|
| `libEGL.dll`, `libGLESv2.dll` | ANGLE, https://chromium.googlesource.com/angle/angle | BSD-3-Clause |
| `libgcc_s_seh-1.dll`, `libstdc++-6.dll` | GCC runtime libraries, https://gcc.gnu.org | GPL-3.0 with the GCC Runtime Library Exception |
| `libwinpthread-1.dll` | mingw-w64 winpthreads, https://www.mingw-w64.org | MIT and BSD-3-Clause |
| `zlib1.dll` | zlib, https://zlib.net | zlib |
