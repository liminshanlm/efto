# Third-Party Licenses

efto's own source is MIT (see [`LICENSE`](LICENSE)). This file records every third-party
component that is built into, distributed with, or bundled alongside efto — and says which
artifact actually carries which notice.

Counts below were taken on **2026-11-23** from the dependency graph pinned in `Cargo.lock`
(670 packages, including the three workspace crates). Re-check them with the commands at the
end of this file rather than trusting these numbers indefinitely.

## What ships in which artifact

| Artifact | `LICENSE` | This file | `licenses/*.txt` | Plugin notices |
| --- | --- | --- | --- | --- |
| `efto-<ver>-windows-x64-portable.zip` | ✓ | ✓ | ✓ | ✓ (each plugin directory keeps its own) |
| `efto_<ver>_x64-setup.exe` (NSIS) | ✓ (license page + installed next to `efto.exe`) | ✓ | ✓ | ✓ (bundled plugins) |
| `efto-<ver>-macos-arm64.dmg` | ✓ (inside `efto.app/Contents/Resources`) | ✓ | ✓ | ✓ (plugins travel in the same Resources directory) |
| `<plugin>-<version>.zip` | — | — | — | ✓ |

The zip/installer/bundle contents come from two places: the staging steps in
`.github/workflows/release.yml` (portable zip) and `bundle.licenseFile` plus `bundle.resources`
in `crates/app/tauri.conf.json` (NSIS installer and the macOS `.app`). Adding a license file
means touching those, not just this document.

**Remaining gap.** `licenses/MIT.txt` is the generic MIT text with no copyright line, and no
aggregated per-crate holder list exists anywhere. MIT and BSD ask for the *notice*, not only
the text, so a generated attribution file (`cargo-bundle-licenses` or `cargo-about`, driven
from `Cargo.lock` and wired into the release job) is still owed. The MPL-2.0, Unicode-3.0,
Apache-2.0, BSL-1.0 and V8 side of the obligation is covered by the texts above, since those
licenses do not depend on per-crate copyright lines.

**Not exercised by a build.** The `bundle.licenseFile` / `bundle.resources` additions were
validated as configuration (JSON schema, paths resolving) and by walking the workflow, but no
Windows NSIS build or macOS bundle build was run as part of the change that added them. The
first `v*` tag push is what will actually produce them.

## Components built into the binaries

| Component | Version / source | License | License text |
| --- | --- | --- | --- |
| V8 (embedded JavaScript engine) | rusty_v8 152.2.0 / v8 152.2.0 | BSD-3-Clause | [`licenses/V8-BSD-3-Clause.txt`](licenses/V8-BSD-3-Clause.txt) |
| rusty_v8 (the Rust binding crate) | 152.2.0, https://crates.io/crates/v8 | MIT | [`licenses/MIT.txt`](licenses/MIT.txt) |
| Tauri | 2.x, https://tauri.app | MIT OR Apache-2.0 | [`licenses/MIT.txt`](licenses/MIT.txt), [`licenses/Apache-2.0.txt`](licenses/Apache-2.0.txt) |
| deno_ast / SWC (TypeScript type stripping) | https://crates.io/crates/deno_ast | MIT OR Apache-2.0 | [`licenses/MIT.txt`](licenses/MIT.txt), [`licenses/Apache-2.0.txt`](licenses/Apache-2.0.txt) |
| cssparser, cssparser-macros, selectors, dtoa-short, option-ext, smartstring | https://crates.io/crates/cssparser | MPL-2.0 (`smartstring` is `MPL-2.0+`) | [`licenses/MPL-2.0.txt`](licenses/MPL-2.0.txt) |
| ICU4X (`icu_*`, `ixdtf`) — Unicode data and locale tables | 2.4.0, https://github.com/unicode-org/icu4x | Unicode-3.0 | [`licenses/Unicode-3.0.txt`](licenses/Unicode-3.0.txt) |
| clipboard-win, error-code, ryu, ryu-js | https://crates.io/crates/clipboard-win | BSL-1.0 | [`licenses/BSL-1.0.txt`](licenses/BSL-1.0.txt) |
| `es.exe` (Everything command-line tool) | https://www.voidtools.com | Not redistributed | Fetched by `scripts/fetch-es.ps1` at the user's request; never bundled in this repository or in any release artifact |

Everything else in the graph is MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC, Zlib,
Unlicense, CC0-1.0, 0BSD, or a dual/tri-license that includes one of those. Their copyright
notices and license texts live in the corresponding crate sources; the exact versions are
pinned in `Cargo.lock`.

## What each license family obliges

- **MIT / BSD-2 / BSD-3 / ISC / Zlib / BSL-1.0** — reproduce the copyright notice and license
  text in binary distributions. This is the obligation the gaps above are about.
- **Apache-2.0** — include a copy of the license. Apache-2.0 also requires reproducing an
  upstream `NOTICE` file *if the upstream ships one*; no crate in this graph ships a `NOTICE`
  file (checked against the unpacked registry sources on 2026-11-23), so no extra notice
  aggregation is owed.
- **MPL-2.0** — file-level copyleft. efto links these crates without modifying their files and
  does not relicense its own source. Source for the MPL-2.0 crates is available from their
  upstream repositories and from crates.io at the `Cargo.lock` versions, which satisfies the
  source-availability requirement.
- **Unicode-3.0 / CC0-1.0 / 0BSD / Unlicense** — permissive, no attribution required; recorded
  here for completeness.
- **Dual and tri-licensed crates** — efto takes the MIT or Apache-2.0 branch. Notably `r-efi`
  offers `MIT OR Apache-2.0 OR LGPL-2.1-or-later`; the LGPL branch is not used. **No
  GPL/AGPL-only dependency exists in the graph.**

### V8 note

V8 is distributed under BSD-3-Clause. V8's own `LICENSE` lists the externally maintained
libraries it carries (PCRE test suite, WebKit-derived layout tests, the Strongtalk assembler,
the Valgrind client API, Wasm C/C++ API headers); those notices are reproduced verbatim in
[`licenses/V8-BSD-3-Clause.txt`](licenses/V8-BSD-3-Clause.txt) together with the BSD-3-Clause
text and Google's copyright line.

## Plugins that carry third-party code

- **`dev-tools`** is an efto adaptation of [Ctool](https://github.com/baiy/Ctool) 2.3.0
  (MIT, © 2017 baiy). Its `LICENSE` and `THIRD_PARTY_NOTICES.md` live inside the plugin
  directory, so they travel with both the portable zip and the macOS bundle. `assets/` is the
  prebuilt Ctool front-end bundle and embeds Vue, Element Plus, ECharts, CryptoJS, jsrsasign,
  pako, and js-yaml; their notices are retained in the asset files as `@license`,
  `Copyright`, or `SPDX-License-Identifier` comments. `icon/` and `favicon.ico` are upstream
  Ctool icon assets. If those assets are ever replaced or re-minified, keep the upstream
  headers.
- **`file-share`** declares `https://gitee.com/yuDeJiJie/file-share` as its homepage. The
  provenance of its logo and UI (self-written vs. derived from that project) is not recorded
  anywhere in this repository — **this needs confirming before the plugin is described as
  original work**.
- **`quickweb`** bundles no third-party logos. Avatars are generated at render time from the
  trigger word plus a hash-derived hue (`plugins/quickweb/ui/app.js`).

## Not redistributed by efto

- **Everything / `es.exe`** — downloaded from voidtools by the user (`scripts/fetch-es.ps1`).
- **WebView2 Runtime** — Microsoft component shipped with Windows/Edge; required, not bundled.
- **Node.js and Python** — invoked through the `node` / `python` capabilities; the user's own
  installations are used, nothing is redistributed.
- **macOS and Windows system frameworks** (Cocoa/WebKit, Win32/COM) — part of the OS.
- **NSIS** (zlib/libpng) — build-time installer generator; the Windows installer embeds its
  stub code, and its license text is available from the NSIS project.

## Assets and trademarks

- The efto application icons and most plugin `logo.png` files are self-drawn for this project.
  The exception is `dev-tools`, whose icon and favicon are upstream Ctool assets (MIT) — see
  the plugin's own notices.
- Product names such as Windows, Chrome, Edge, Everything, Figma, and Trae are trademarks of
  their respective owners and are used only descriptively (for example, to label a shortcut or
  a link target). No endorsement or affiliation is implied.

## Re-checking this file

The dependency graph, not this table, is the source of truth. On Windows:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/cargo.ps1 -Cmd "metadata --format-version 1"
```

On macOS / Linux:

```bash
scripts/cargo.sh "metadata --format-version 1"
```

Pipe either into the same jq/python step to list every distinct `license` expression, and
diff the result against the sections above. Two things to look for specifically: any new
`MPL-*` or `(L|A)GPL-*` entry, and any package whose `license` field is empty (a
`license-file`-only crate needs its text vendored under `licenses/`).

Adding `cargo-bundle-licenses` or `cargo-about` to CI is the intended fix for the known gaps:
it generates a complete attribution file from the lock file, which can then be dropped into
every artifact instead of relying on this hand-maintained list.
