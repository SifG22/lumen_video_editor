# Lumen Video Editor app icon

A white **clapperboard with a play button** on a rounded Lumen Video Editor-violet tile, drawn by SifG22 in
the same style as the Lumen Photo Editor icon: simple, flat and readable on light and dark
taskbars.

## Palette

| Colour | Hex | Used for |
|---|---|---|
| Lumen Video Editor violet, top | `#a787ff` | the tile, a soft vertical fade… |
| Lumen Video Editor violet, bottom | `#7a4bef` | …to this colour |
| White | `#ffffff` | the clapperboard |

## Geometry

- A 512-unit viewBox tile, rounded square `rx=112`, full bleed (no frame or border).
- The clapper arm is raised 14° about its hinge at the body's top-left corner. Stripes on the arm
  and on the body's top band are cut out of the white, so the tile shows through; so is the play
  button, centred in the body.
- macOS renders (`lumen.icns`, `lumen-macos-512.png`) put the tile in Apple's grid (824 of
  1024 px, transparent margin). Windows, Linux and the web use the full-bleed tile.

## Files

| File | What |
|---|---|
| `lumen.svg` | master, used for 48 px and up (and the hicolor scalable icon) |
| `lumen-small.svg` | bolder variant for 32–40 px: two wide stripes on the arm, none on the body |
| `lumen-tiny.svg` | plain variant for 16–24 px: arm and body without stripes, bigger play button |
| `lumen-1024.png` | 1024 px render |
| `lumen-macos-512.png` | runtime Dock icon on macOS (embedded by `apps/lumen/src/main.rs`) |
| `lumen.icns` | macOS bundle icon (`CFBundleIconFile`) |
| `lumen.ico` | Windows icon, 16–256 px (embedded in `lumen.exe` by `apps/lumen/build.rs`) |
| `hicolor/<size>/apps/net.sifg.lumen.png`, `hicolor/scalable/…svg` | Linux icon theme, used by `packaging/linux/net.sifg.lumen.desktop` |

The web app's `apps/lumen-web/web/favicon.png` (128 px) and the About dialog and Home screen
(`crates/ui-egui/src/brand.rs`, from the 256 px hicolor PNG) use the same artwork.

## How the icon reaches each OS

- **macOS:** the window icon set at startup (`ViewportBuilder::with_icon`) is the Dock and app-switcher
  icon when the app runs unbundled. A `.app` bundle points `CFBundleIconFile` at `lumen.icns`.
- **Windows:** `build.rs` embeds `lumen.ico` as the exe's resource icon (Explorer, Start menu,
  pinned taskbar), and the runtime icon covers the window, taskbar and Alt-Tab.
- **Linux:** the runtime icon plus `with_app_id("net.sifg.lumen")`, which Wayland matches to
  the `.desktop` file. Install the `hicolor/` tree under `/usr/share/icons/hicolor/` and the desktop
  file under `/usr/share/applications/`.

## Regenerate

Edit the three SVGs, then run `packaging/icons.sh` (needs `resvg`; `iconutil` on macOS for the
`.icns`; the `.ico` is packed by `cargo xtask ico`). Every derived file needs its `.attribution`
sidecar and a row in `ATTRIBUTION.md`. Licence: [LICENSE.txt](LICENSE.txt) (MIT OR Apache-2.0).
