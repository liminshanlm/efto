# efto Help

efto is a Windows launcher (macOS support in progress: the CLI and search/launch/plugin chain work today, GUI tracked in `docs/macos-support-plan.md`): press `Alt+Space`, type, and press Enter to launch apps or run actions. Version 0.1.0.

## 1. Getting Started

1. Press `Alt+Space` to open the search box (change the hotkey in Settings → Launcher).
2. Type an app name, pinyin, initials, URL, file path, or built-in command.
3. Press `Enter` to run the selected item, `Esc` to clear the input, or `Esc` again to hide the window.
4. Mouse: copyable rows (calculator, timestamps, command output) copy on **single click**; apps, plugins, and files run on **double click**.
5. Use `↑` `↓` or `Ctrl+P` `Ctrl+N` to move, or scroll with the mouse wheel.

| Key | Action |
| --- | --- |
| `Alt+Space` | Show / focus the search box |
| `Enter` | Run the selected item; with no results, open the input as a path or URL |
| `Esc` | Clear input; hide the window when empty; cancel a running command |
| `↑` `↓` / `Ctrl+P` `Ctrl+N` | Move the selection |
| `←` `→` | Move horizontally in the default panel |
| `Shift` | Hold to preview; press again to close |
| `Alt+1..9` | Run the Nth row in the search results; in the default panel, run the Nth favorite tool |
| `Tab` | Open the file action menu on a file result |
| `Ctrl+Enter` | Add a file result to the file buffer |
| `Alt+R` | Rebuild the app index |
| `Alt+P` | Open Settings |

The Universal Actions hotkey defaults to `Ctrl+Alt+Space` (toggle or change it in Settings → Launcher; not available on macOS):
select text in any app, press it, then copy, paste back into the foreground window, search the web,
or open the selected link/path. With a model configured in Settings → Model and the translation
(`translate`) / AI chat (`ai-chat`) plugins installed, the menu also offers "Translate selection"
(results inline in the main window) and "Ask AI" (opens the AI plugin with the selected text).

## 2. App Search and Launch

- The index covers Start Menu, Desktop shortcuts, registry App Paths, and common install folders; `.lnk` files are resolved to their real targets.
- Pinyin, initials, and subsequence matching are supported, e.g. `wx` or `weixin` both find WeChat.
- Usage frequency affects ranking, so frequently launched apps move up over time.
- In Settings → Index you can:
  - choose which entry sources are indexed;
  - toggle registry indexing and install-folder scanning;
  - add custom index folders;
  - set the refresh interval; `0` means manual rebuild only (`Alt+R`).
- Built-in ignore rules skip uninstallers, help files, and similar noise; custom folders use the same rules.

## 3. Open Directly

No index is needed for these inputs:

- Full URLs: `https://example.com`, `http://...`.
- Windows Settings: `ms-settings:display`, `ms-settings:windowsupdate`. Typing just
  `ms-settings:` lists common pages; keep typing (e.g. `ms-settings:blue`) to filter.
- Existing files or folders: `C:\Windows`, `D:\project\readme.md`.
- User paths starting with `~`: `~\Downloads`, `~\.efto`.
- With no results, `Enter` retries the input as a path or URL.

## 4. Built-in Sources and Strict Rules

Built-in sources appear only when the input matches their rules, so they never hijack app search:

- **Calculator**: the whole line must be a valid expression (`12*3+7`, `sqrt(2)`); plain text with letters never triggers it.
- **Timestamps**: decimal digits in the valid seconds/milliseconds range; ordinary numbers like `2026` are not misread.
- **System time formats**: eight fixed formats (`2026-11-19`, `2026/11/19 08:30`, `08:30:00`, …) only when the whole line matches.
- **Everything file search**: when `es.exe` is available, file rows are appended after app results and score below every app entry; they never push apps out.
- **Browser bookmarks**: Chrome / Edge bookmarks participate only when "Search bookmarks" is enabled, and also rank after apps.
- **Snippets**: appear only when the input exactly equals a snippet keyword; prefixes or extra text never trigger.
- **Web search**: type `<keyword> <text>`, e.g. `g rust lifetimes` or `wiki Eiffel`. The keyword must be the first token and text must follow; once matched, local results are hidden.
  - Typing the keyword alone shows a usage hint row ("Search Bing…") without displacing local apps; pressing `Enter` on it only echoes the usage.
  - Default keywords: `b` Baidu, `g` Bing, `gh` GitHub, `wiki` Wikipedia. Add or edit them in Settings → Web Search; URLs use `{q}` as the placeholder.
  - When nothing local matches, enabling "Fallback search when empty" appends one search row for the fallback engine; it always ranks after real results.
- **System commands**: appear only when the whole input equals one of: `sleep`, `restart`, `logout`/`logoff`, `emptytrash`, `screensaver`; press Enter to run.
  - Restart, log off, and empty Recycle Bin ask for confirmation first; cancel does nothing. Inputs with extra text such as `sleep now` never trigger.

## 5. Shift Preview

- Hold `Shift` to preview the selected item; releasing does not close it, pressing `Shift` again does.
- Supports text, images, video, web pages, PDF, local HTML, and file properties.
- Web previews show Open / Close buttons; `Shift` also closes them.
- When the preview takes focus, "hide on blur" is suppressed; return to the search box to continue.

## 6. Favorite Tools

> On macOS today: search/launch apps, calculator & time, command source, plugins, clipboard
> history, snippets, lock screen, window tiling/half-screen/minimize/restore, Universal Actions,
> file buffer, the shortcut guide, system commands (sleep/restart/log out/empty trash/
> screensaver), file search (Spotlight), System Settings search, the hosts plugin, and window
> list/focus. Window tiling and keyboard simulation and screenshots need Accessibility / Screen Recording in
> Settings → Permissions.

The default panel (empty input) contains "Recent" and "Favorite tools":

- **Screenshot**: select a region, then drag inside the selection to move it as a whole (annotations move with it) or drag the empty area to select again; annotate from the toolbar — rectangle/ellipse/arrow/pen/highlighter/mosaic/text/eraser (number keys `1-8`), an 8-color palette, three stroke widths, and `Ctrl+Z`/`Ctrl+Y` undo/redo. Annotations are baked into the copy/save/pin output. Scrolling long screenshots are also supported (not combinable with annotations).
- **Clipboard**: records text, images, and file lists; click to copy, `Ctrl+Enter` to copy and paste into the foreground window, `P` to pin, `Delete` to remove.
- **Lock screen**: lock Windows immediately.
- **Window actions**: selecting it opens a submenu — grid tiling (Tile windows), minimize other windows (keeping the one under the search box), minimize all windows, left/right half, and undo the last tiling. Grid tiling only touches the search box's monitor; minimizing applies to all monitors; minimized windows are skipped, and efto, the desktop, and the taskbar never participate. Per-tool hotkeys still tile directly without opening the menu.
- **Shortcut guide**: remembers the app that was in front before efto opened, then lists its shortcuts in a separate window. Switch apps from the dropdown, filter inside the window, press `Esc` to close. Built-in coverage includes Windows, Office, browsers, VS Code, Photoshop, Figma, IDEA, and Trae; apps that are not covered fall back to Windows shortcuts.
  - Override or extend the built-in data with `~/.efto/shortcuts/<exe>.json`: groups with the same name merge by `keys` (user data wins), new groups are appended, and a broken file only affects itself.
  - The CLI can import the same format: `efto.exe cli import-shortcuts --file <path> --json` (reads stdin when `--file` is omitted; `--dry-run` reports without writing).
- **Snippets**: maintain keywords and bodies in Settings → Snippets; bodies support `{date}`/`{time}`/`{datetime}`. Type the exact keyword and press Enter: the expanded text is copied and pasted back into the previous window. There is no system-wide per-keystroke expansion.
- **File buffer**: batch operations for multiple files. Press `Ctrl+Enter` on a file result or choose "Add to file buffer" in its action menu, then open "File buffer" from the default panel to copy to clipboard, copy to…, move to…, delete to Recycle Bin, or clear.
- **Color picker**: freezes the current screen when opened, shows the pixel under the cursor in a magnifier with HEX/RGB, and copies HEX on click or `Enter`; `Esc` cancels. Animated/video colors are read from the frozen frame. Not in Quick Tools by default; enable it in Settings → Quick Tools.
- **Sticky note**: creates a new always-on-top note each time; text autosaves, the title bar drags the window, and `×` closes and deletes that note. Multiple notes can be pinned at once and are restored at their last positions after restart. Not in Quick Tools by default; enable it in Settings → Quick Tools.
- Custom apps/plugins can also join Quick Tools; run them with `Alt+1..9` on the default panel, or select with `←` `→` and press Enter.
- **File actions**: press `Tab` on a file result (or click the `⋯` at the row end) to open the action menu: open, open with, reveal in Explorer, copy path, copy file, add to buffer, copy to…, move to…, delete to Recycle Bin.
- Custom apps and plugins can be added as favorite tools; press `Alt+1..9` or use `←` `→` and Enter.
- **Per-tool hotkeys**: bind global hotkeys to screenshot, clipboard, lock screen, window actions, shortcut guide, snippets, and file buffer in Settings → Tool Hotkeys, so one press runs the tool without opening the search box. They default to `Ctrl+Alt+1…7` in that order; color picker and sticky note do not take a default hotkey but can be bound on the same page. Click a field and press the combination to record it. Leave a field empty to unbind; combinations that duplicate the launcher or Universal Actions hotkey are discarded.
- Recent items can be cleared at any time.

## 7. Plugins

- Plugins are described by `efto-plugin.json` and load automatically from the plugin folder; import and inspect them in Settings → Plugins.
- A trigger keyword opens the plugin; command aliases are supported.
- Plugins can register dynamic results, open their own UI, and call `efto.*` capabilities (notifications, clipboard, files, HTTP, AI, …).
- TypeScript is type-stripped only, with no runtime code generation: `enum`, value `namespace`, decorators, and JSX are rejected.
- There is no Node/npm: plugins run in an embedded V8 and cannot `require` CommonJS packages.

## 8. Settings

Settings has seven pages:

- **General**
  - Appearance: theme (system / dark / light).
  - Launcher: global hotkey, Universal Actions hotkey and toggle, max results, hide on blur, always on top.
  - Favorite tools: enable and order screenshot, clipboard, lock screen, tile windows, shortcut guide, snippets, and file buffer.
  - Index: entry sources, auto refresh, registry index, install-folder scan, custom folders.
  - Browser bookmarks: whether to search them.
  - Everything: status of the app and `es.exe`, with one-click `es.exe` install.
  - Command execution: see section 13.
  - Language: system / 简体中文 / English.
- **Snippets**: maintain keyword, title, and body; keywords cannot contain spaces and must be unique.
- **Web Search**: manage search engines (keyword, name, URL containing `{q}`), the empty-result fallback toggle, and the fallback engine.
- **Tool Hotkeys**: bind independent global hotkeys to the 7 built-in tools.
- **Plugins**: import plugins, list installed plugins, disable / re-enable each one (disabled plugins drop out of search and can't be triggered; their files stay), open the plugin folder.
- **Runtimes**: Node.js and Python interpreter paths, with a check button. When left empty, efto auto-detects them from the system PATH at startup; you can still edit them. If a runtime is missing or fails the check, the card shows install guidance and a download link (Node.js LTS; for Python, tick "Add python.exe to PATH" during setup).
- **Models**: OpenAI-compatible model list and default model for `efto.ai.chat`.

Changes apply immediately; hotkey and theme changes propagate to all windows.

## 9. Tray Menu

Right-click (or left-click) the tray icon:

- **Show search box**: same as the hotkey.
- **Settings…**: open Settings.
- **Help**: open this window.
- **Quit efto**: stop the plugin runtime and exit.

## 10. CLI

Call efto without the GUI, for scripts and agents:

```powershell
efto.exe cli index --json
efto.exe cli search "chrome" --limit 5 --json
efto.exe cli launch --id <hit-id>
efto.exe cli plugins --json
efto.exe cli plugin-run <plugin-id> --wait-ms 3000 --json
efto.exe cli run-command "git status" --json
efto.exe cli import-shortcuts --file shortcuts.json --json
efto.exe cli capabilities --json
```

- Exit codes: `0` ok, `1` execution failed, `2` bad arguments, `3` not found.
- Most subcommands accept `--json` for machine-readable output.
- `--dry-run` prints the action instead of running it (`launch`, `files`, `action`, `focus`, `run-command`, `import-shortcuts`).
- `import-shortcuts` takes the same JSON shape as `~/.efto/shortcuts/<exe>.json`:
  `{"app":{"exe":["trae cn.exe"],"name":"Trae"},"groups":[{"name":"Editing","items":[{"keys":"Ctrl+Enter","action":"Insert a new line"}]}]}`.
  Rows merge by group name plus `keys`, imported data wins, and re-importing is idempotent; `capabilities --json` embeds the full example.
- Use `capabilities --json` to list every subcommand and flag supported by this build.

## 11. Data and Configuration

- Data root: `%USERPROFILE%\.efto`
  - `settings.json`: all settings.
  - `snippets.json`: snippets; `shortcuts/`: user overrides for the shortcut guide.
  - `clipboard.json` and `clipboard-images/`: clipboard history and images.
  - Index cache and usage history: rebuilt on next start if deleted.
  - Plugin data, pinned images, and more.
- `EFTO_PLUGIN_DIRS`: extra plugin folders (`;` separated).
- `EFTO_LOCALE`: override the locale identifier.
- `RUST_LOG`: log level, e.g. `RUST_LOG=debug`; logs go to stderr and never pollute `--json`.

## 12. Known Limits and Troubleshooting

- **No Node runtime**: plugins only use efto capabilities; CommonJS packages are unavailable.
- **TS is type-stripped only**: no tsc output; `enum`, value `namespace`, decorators, and JSX fail with an explicit error.
- **File search needs Everything**: without it or `es.exe`, file results are absent; install `es.exe` from Settings.
- **Some UWP apps may be missing**: Store apps without classic shortcuts may not be indexed; recreate a Start Menu shortcut.
- **Hotkey conflicts**: if another app owns the combo, startup logs an error; pick another combo and save.
- **Empty index**: after three consecutive empty auto-rebuilds, auto refresh stops; check index sources and press `Alt+R`.

## 13. Command Execution

Type the command prefix (default `>`) in the search box:

```text
> ipconfig /all
> git status
> python -V
```

- Entry rule: only a **line-start prefix** enters the command source; once it matches, app/calculator/file results are all suppressed. The default prefix is `>`, configurable in Settings, but letters, digits, and whitespace are rejected.
- Output modes:
  - **Inline** (default): the command runs in the background and stdout / stderr come back line by line; click any line to copy, and stderr is shown in red.
  - **Terminal window**: runs in a separate terminal, for interactive programs (not yet on macOS — it errors explicitly; Terminal.app integration is v1.5).
- Press `Esc` while running to kill the child process immediately.
- Timeout defaults to 10 seconds (1–60), and also kills the process.
- Shells per platform: Windows offers PowerShell, PowerShell 7+, cmd, Git Bash, WSL, and Custom;
  macOS offers zsh, bash, fish, and Custom (default zsh). A settings file reused across platforms falls back to the platform default shell at load.
  - WSL accepts a distro name; empty means the default distro.
  - Custom requires an executable path and accepts an argument template (`{cmd}` is replaced by the command).
  - The "Check" button in Settings runs a real `echo` to verify the shell.
- Security: commands run with your current account; pipes, redirection, and environment variables depend on the chosen shell. Avoid echoing secrets in shared scripts.

Equivalent CLI:

```powershell
efto.exe cli run-command "ipconfig" --json
efto.exe cli run-command "ipconfig" --dry-run
```
