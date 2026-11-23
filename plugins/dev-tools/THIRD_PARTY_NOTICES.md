# dev-tools third-party notices

This plugin is an efto adaptation of [Ctool](https://github.com/baiy/Ctool) 2.3.0.

- Upstream author: baiy
- Upstream license: MIT (see `LICENSE`)
- efto adaptation: `main.js`, `ui/bridge.js`, `lib/routes.js`, `index.html`

## Bundled front-end assets

`assets/` contains the prebuilt Ctool 2.3.0 front-end bundle. It includes third-party
libraries such as Vue, Element Plus, ECharts, CryptoJS, jsrsasign, pako, and js-yaml.
Their license notices are retained in the corresponding asset files as `@license`,
`Copyright`, or `SPDX-License-Identifier` comments. At the time of this check the bundle
contained 31 `Copyright`, 20 `@license`, and 5 `SPDX-License-Identifier` markers.

If an asset is ever replaced or re-minified, keep the upstream license header.

## Icons

`icon/` is copied from upstream. `icon_utools.png` and `icon_chrome.png` are upstream
assets; efto's UI does not reference them. `favicon.ico` and `icon/icon_1024.png` are the
upstream Ctool icon assets, covered by the upstream MIT license.

efto does not provide a uTools compatibility layer, and this plugin does not claim
compatibility with any third-party launcher.
