# ShareDesk Pro branding

ShareDesk Pro is a branded build of [RustDesk](https://github.com/rustdesk/rustdesk) (AGPL-3.0).

- `icon.svg` – app icon source (used for `res/icon.*`, `res/*x*.png`, `flutter/assets/icon.svg`
  and `flutter/windows/runner/resources/app_icon.ico`).
- `icon-small.svg` – simplified icon for 16–40 px sizes (tray icon, small .ico layers).
- `flutter/assets/logo*.png` – logo shown on the main window.

The Windows build runs from `.github/workflows/sharedesk-windows.yml` (Actions → "ShareDesk Pro - Windows build" → Run workflow).
It reads the server address and public key from the repository secrets `RENDEZVOUS_SERVER` and `RS_PUB_KEY`,
renames the app to `ShareDesk-Pro` and publishes the installers under the `sharedesk-pro` release.
