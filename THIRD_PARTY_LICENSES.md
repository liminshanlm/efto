# Third-Party Licenses

efto itself is licensed under the MIT License (see `LICENSE`). This file records the
third-party components that are built into, or distributed with, efto builds, and where
their license texts can be found.

The complete transitive dependency graph is pinned in `Cargo.lock`. License expressions
for every crate can be re-checked with:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/cargo.ps1 -Cmd "metadata --format-version 1"
```

## Components that require attribution

| Component | Version / source | License | License text |
| --- | --- | --- | --- |
| V8 (embedded JavaScript engine) | rusty_v8 152.2.0 / v8 152.2.0 | BSD-3-Clause | [`licenses/V8-BSD-3-Clause.txt`](licenses/V8-BSD-3-Clause.txt) |
| rusty_v8 | 152.2.0, https://crates.io/crates/v8 | MIT | [`licenses/MIT.txt`](licenses/MIT.txt) |
| Tauri | 2.x, https://tauri.app | MIT OR Apache-2.0 | [`licenses/MIT.txt`](licenses/MIT.txt), [`licenses/Apache-2.0.txt`](licenses/Apache-2.0.txt) |
| deno_ast / SWC | https://crates.io/crates/deno_ast | MIT OR Apache-2.0 | [`licenses/MIT.txt`](licenses/MIT.txt), [`licenses/Apache-2.0.txt`](licenses/Apache-2.0.txt) |
| cssparser, selectors | https://crates.io/crates/cssparser | MPL-2.0 | [`licenses/MPL-2.0.txt`](licenses/MPL-2.0.txt) |
| unicode-ident and related Unicode data crates | https://crates.io/crates/unicode-ident | Unicode-3.0 | [`licenses/Unicode-3.0.txt`](licenses/Unicode-3.0.txt) |
| Everything command-line tool (`es.exe`) | https://www.voidtools.com | Not redistributed | Downloaded by the user at runtime; never bundled in this repository or in release installers |

The remaining Rust dependencies are distributed under MIT, Apache-2.0, BSD-2-Clause,
BSD-3-Clause, ISC, Zlib, Unicode-3.0, or MPL-2.0. Their copyright notices and license
texts are retained in the corresponding crate sources and can be obtained from
[crates.io](https://crates.io) using the exact versions in `Cargo.lock`.

### MPL-2.0 note

MPL-2.0 is a file-level copyleft license. efto links these crates without modifying their
files, and does not relicense its own source. Source for the MPL-2.0 crates is available
from their upstream repositories and from crates.io at the versions pinned in `Cargo.lock`.

### V8 note

V8 is distributed under the BSD-3-Clause license. V8 itself lists externally maintained
libraries (PCRE test suite, layout tests, Strongtalk assembler, Valgrind client API,
Wasm C/C++ API headers) in its `LICENSE` file; those notices are reproduced in
[`licenses/V8-BSD-3-Clause.txt`](licenses/V8-BSD-3-Clause.txt).

## Assets and trademarks

- efto application icons and plugin `logo.png` files are self-drawn for this project.
- The `quickweb` plugin no longer bundles third-party logos. Site avatars are generated
  from the trigger word and a hash-derived color at render time.
- Product names such as Windows, Chrome, Edge, Everything, Figma, and Trae are trademarks
  of their respective owners and are used only descriptively (for example, to label a
  shortcut or a link target). No endorsement or affiliation is implied.
