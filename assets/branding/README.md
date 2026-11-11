# Branding

Pixel-art icon and wordmark for rusty-tip. Everything in this folder is
generated from the 1× sources in `source/` by `export.py`.

The icon is drawn at 24×24. 24, 48, 96 and 192 are exact scales. 32, 64, 128,
256 and 512 use the largest multiple of 24 that fits, centred with a
transparent margin (75 % fill at 32 and 64, 94 % from 128 up).

| Path | What |
|---|---|
| `source/icon-24.png`, `source/lockup.png` | 1× sources; edit these |
| `icon/rusty-tip-{24..512}.png` | Icon at common sizes |
| `icon/rusty-tip.svg` | Pixel-exact vector, `crispEdges` |
| `icon/rusty-tip.ico` | Windows icon, 24–256 px, each frame pre-rendered |
| `icon/rusty-tip-{48,96}.rgba` | Raw RGBA for `egui::IconData` |
| `hicolor/` | freedesktop icon theme tree (24–512 px + scalable) |
| `logo/rusty-tip-lockup-{1,2,4,8}x.png`, `.svg` | Icon + "rusty tip" wordmark |
| `social/social-preview-{dark,light}.png` | 1280×640 GitHub social card |
| `rusty-tip.desktop` | Desktop entry for `rusty-tip-gui` |
| `rusty-tip.rc` | Windows resource script for the `.exe` icon |

Scaling is nearest-neighbour by integer factors only.

Regenerate after editing a source:

```nu
nix shell nixpkgs#python3Packages.pillow -c python3 assets/branding/export.py
```

## README header

```html
<p align="center">
  <img src="assets/branding/logo/rusty-tip-lockup-4x.png" alt="rusty tip" width="320">
</p>
```

The palette was picked to read on both GitHub themes, so one transparent image
is enough. The social card is uploaded under *Settings → General → Social
preview*; it isn't read from the repo.

## GUI window icon (eframe 0.33)

The `.rgba` files are raw pixels, so `include_bytes!` is all that's needed: no
PNG decoder, no extra feature on eframe.

```rust
let icon = egui::IconData {
    rgba: include_bytes!("../../assets/branding/icon/rusty-tip-96.rgba").to_vec(),
    width: 96,
    height: 96,
};
let options = eframe::NativeOptions {
    viewport: egui::ViewportBuilder::default()
        .with_app_id("rusty-tip")
        .with_icon(icon),
    ..Default::default()
};
```

`with_icon` works on X11 and Windows. **Wayland ignores it**: the compositor
looks up the icon through the desktop entry whose name matches the app id. So
`with_app_id("rusty-tip")`, `rusty-tip.desktop` and the `hicolor/` icons have
to be installed together for niri/sway to show it.

## Linux install (flake)

```nix
postInstall = ''
  install -Dm644 assets/branding/rusty-tip.desktop -t $out/share/applications
  mkdir -p $out/share/icons
  cp -r assets/branding/hicolor $out/share/icons/
'';
```
## Windows `.exe` icon

The icon shown in Explorer and the taskbar comes from a resource compiled into
the binary, which needs a step in `build.rs`. Options:

- **`embed-resource`** (build-dep): compiles `rusty-tip.rc` with the platform
  resource compiler. Small, does one thing; the `.rc` file is already here.
- **`winresource`**: generates the resource from Cargo metadata (icon plus
  version info, product name). More convenient, more dependencies.
- **Skip it**: the window icon from `with_icon` still shows in the taskbar
  while the app runs. Only the `.exe` file in Explorer stays generic.

Either way, gate it on the *target*, not the host, since `cfg!(windows)` in a
build script describes the machine running the build:

```rust
if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
    // compile assets/branding/rusty-tip.rc here
}
```
