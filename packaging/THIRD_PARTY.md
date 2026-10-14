# Third-party software

jelly-gpui plays video with **libmpv**, the library form of mpv, running inside the app.

- **Windows:** the package includes `libmpv-2.dll`, a build from
  https://github.com/shinchiro/mpv-winbuild-cmake. The version is pinned in `packaging/mpv.lock`.
- **Linux:** uses the system libmpv (`libmpv2`).
- **macOS:** uses Homebrew's libmpv (`brew install mpv`) for now.

**License:** GPL-2.0-or-later for these builds, see https://github.com/mpv-player/mpv/blob/master/Copyright.

**Source code:** https://github.com/mpv-player/mpv. The shinchiro build records the exact mpv commit in its file name.

## ANGLE (Windows)

The Windows package also includes **ANGLE** (`libEGL.dll`, `libGLESv2.dll`), which runs OpenGL ES on Direct3D 11. mpv draws through it so hardware-decoded video stays on the GPU. It's MSYS2's build (https://packages.msys2.org/base/mingw-w64-angleproject), pinned in `packaging/angle.lock`, along with the MinGW runtime DLLs it needs. Their license texts are in `licenses/`.

| Files | Project | License |
|---|---|---|
| `libEGL.dll`, `libGLESv2.dll` | ANGLE, https://chromium.googlesource.com/angle/angle | BSD-3-Clause |
| `libgcc_s_seh-1.dll`, `libstdc++-6.dll` | GCC runtime libraries, https://gcc.gnu.org | GPL-3.0 with the GCC Runtime Library Exception |
| `libwinpthread-1.dll` | mingw-w64 winpthreads, https://www.mingw-w64.org | MIT and BSD-3-Clause |
| `zlib1.dll` | zlib, https://zlib.net | zlib |
