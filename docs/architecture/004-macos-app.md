# macOS application design

## Build & structure

- SwiftPM package at `apps/macos` (`swift-tools-version: 5.9`, `platforms: [.macOS(.v14)]`),
  built with **Command Line Tools only** — no Xcode project. `scripts/build-macos.sh` builds the
  Rust static library (`osprey-ffi`, universal when both targets are installed), runs
  `uniffi-bindgen` to produce `OspreyFFI.swift` + modulemap, `swift build -c release`, then assembles
  `Osprey.app` (Info.plist, `Osprey.icns`, embedded `osprey` CLI + native host at
  `Contents/MacOS/osprey`, `Contents/Resources/`), ad-hoc codesigns, and `hdiutil` a DMG.
- Targets: `OspreyFFI` (system-library target wrapping the generated header/modulemap + link
  flags), `OspreyKit` (Swift façade over the FFI: `EngineClient`, stores, formatting, l10n),
  `OspreyApp` (SwiftUI/AppKit executable), `OspreyKitTests`.
- **`@State` shim**: the macOS 27 SDK implements `@State` as a macro whose plugin only ships in
  Xcode. Use `@ViewState` (a property wrapper over `SwiftUICore.State`, in `OspreyKit/Support/
  ViewState.swift`) everywhere `@State` would be used. `@Observable`, `@Environment`, `@Bindable`,
  `@FocusState`, `@AppStorage`, `@SceneStorage` work normally.

## Information architecture (sidebar → content → inspector)

Sidebar (NavigationSplitView, collapsible, with counts):
- **Dashboard** — aggregate speed, bandwidth utilisation sparkline (24 h), active/queued/scheduled,
  completed & failed today, disk usage per destination volume, recent downloads, network status.
- **Downloads** — the main table; smart filters as a segmented control above it (All · Active ·
  Queued · Scheduled · Completed · Failed · Torrents · Media); search field in the toolbar
  (`⌘F`), filter tokens (queue, category, domain, tag, date, size).
- **Queues** (each queue as a row; drag tasks between queues; per-queue concurrency/limits/schedule).
- **Torrents** — same table scoped to torrents with peers/seeds/ratio columns.
- **Scheduled** — tasks + schedules with the next window.
- **Site Grabber** — sessions; crawl configuration; result tree grouped by domain/type with
  select-all/by type/by extension/exclude patterns; "Add selected".
- **History** — searchable, sortable, exportable; re-download; clear.
- **Categories · Rules · Automation · Recipes** (grouped under "Organise").
- **Devices** — paired devices, pairing QR + code, audit log.
- Settings (`⌘,`): General · Downloads · Network · Bandwidth · Torrents · Browser · Notifications ·
  Remote · Automation · Privacy · Updates · Advanced (diagnostics, logs).

Downloads table (`Table` backed by `NSTableView`, lazy, supports 1,000+ rows): file icon + name +
domain subtitle · progress bar with percentage · speed · ETA · size · state pill · queue/category ·
health dot. Row context menu = every task action. Toolbar: Add (`⌘N`, also paste-URL `⌘⇧V`),
Pause/Resume, Remove, Speed mode menu, Search. Multi-select applies actions to all.

Inspector (`.inspector`, `⌘⌥I`) tabs: Overview · Connections (segment map with live per-segment
speed) · Files (torrent files with selection/priorities; HLS segment progress) · Headers · Network
(mirrors, proxy, TLS, resolved IP, HTTP version) · Checksums · History (task log timeline) · Events
· Diagnostics (health breakdown with inputs, "Copy diagnostics").

Add sheet: URL/magnet/torrent/metalink/playlist detection as you type; probe result (name, size,
type, resumable, duplicate warning, free space); destination, queue, category, recipe; advanced
disclosure (connections, limits, headers, auth credential, proxy, checksum, schedule, sequential/
file selection for torrents, media variant picker).

Menu bar extra (`MenuBarExtra`, window style): global ↓/↑ speed, active count, tiny sparkline,
speed-mode picker (Unlimited · Balanced · Browsing · Custom), recent completions (click to reveal),
Pause all / Resume all / Retry failed, Add URL / Paste URL, Open Osprey, Open downloads folder.

Dock: badge = active count; progress on the icon (`NSDockTile` with a custom view); Dock menu.

## Native integrations (Swift side)

Notifications (`UNUserNotificationCenter`, categories with actions Open / Reveal / Retry), Finder
reveal (`NSWorkspace.activateFileViewerSelecting`), Finder tags (`URLResourceValues.tagNames`),
Quick Look (`QLPreviewPanel` via `NSResponder` chain, space bar), drag & drop of URLs/files/
`.torrent` onto the window and the Dock icon, Services ("Download with Osprey"), URL scheme
`osprey://add?url=…` and `magnet:`/`.torrent`/`.metalink` document types, Keychain (via the
FFI's `store_credential`), login item (`SMAppService.mainApp`), power assertion
(`IOPMAssertionCreateWithName(kIOPMAssertPreventUserIdleSystemSleep)` while `GlobalStats.active > 0`
and the setting is on), environment probe (`NWPathMonitor` for availability/expensive/constrained,
`IOPSCopyPowerSourcesInfo` for AC/battery, VPN via `utun` interfaces in `getifaddrs`, Wi-Fi SSID
omitted unless location permission is granted) → `update_environment`, sleep/quit actions on
`ReadyForSleep`, native-messaging manifest installation for Chrome/Chromium/Edge/Brave/Arc/Firefox,
`LSFileQuarantineEnabled` in Info.plist, Sparkle-free updater install/relaunch, system appearance.

## Design language ("quiet instrument")

- Typography: SF Pro; table rows 13 pt with 11 pt secondary; monospaced digits for speeds/sizes.
- Colour: system semantic colours; one accent (system accent); state colours limited to
  success/warning/error; health dot uses four steps. Materials: sidebar `.sidebar`, inspector
  `.underWindowBackground`. No custom chrome, no gradients, no rounded "cards" inside tables.
- Motion: progress bars animate; list changes use default `NSTableView` animations; respect
  Reduce Motion.
- Empty states: an SF Symbol, one sentence, one primary action ("Add a download", "Paste a link").
- Loading: skeleton rows for the first snapshot only; everything else is event-driven.
- Accessibility: every control labelled; table rows expose state/progress to VoiceOver; full
  keyboard navigation (`⌘1…9` sidebar, `space` Quick Look, `⌫` remove, `⌘.` pause/resume,
  `⌘R` reveal); Dynamic Type not applicable on macOS but honour the system text size setting.
- Localisation: `Localizable.xcstrings`-style catalog implemented as `Localizable.strings` per
  language (`en`, `hi`, `ta` scaffolds), accessed via `String(localized:)`; Rust keys
  (`state.*`, `error.*`, `health.*`, `pause.*`) are mapped in `OspreyKit/L10n.swift`.
