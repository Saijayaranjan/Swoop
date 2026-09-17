# Browser extension behaviour

- **Interception**: `chrome.downloads.onDeterminingFilename` / `onCreated` — when enabled and the
  item matches (`settings.browser`: extension list, min size when known, not excluded domain /
  URL pattern), cancel the browser download and `POST /tasks` with `origin: "browser"`,
  `referer_page`, cookies for the URL (`chrome.cookies.getAll` → `options.cookies`), and the tab
  title as a hint. A toast in the popup confirms; a per-site "always use browser" toggle exists.
- **Context menus**: "Download with Osprey" on links, images, video, audio, page (all links),
  selection (links in selection). "Download all links on page…" opens the picker.
- **Media detection**: `webRequest.onHeadersReceived` (MV3: `declarativeNetRequest` cannot
  observe, so use `webRequest` in observe-only mode where allowed; Firefox: full `webRequest`)
  records responses whose `Content-Type` is `video/*`, `audio/*`, `application/vnd.apple.mpegurl`,
  `application/x-mpegurl`, `application/dash+xml`, or whose URL ends in `.m3u8/.mp4/.webm/.mp3/…`,
  per tab, with size/title. The content script also scans `<video>`/`<audio>`/`<source>` elements.
  Blob/MediaSource URLs and DRM (EME) streams are shown as "not downloadable".
- **Popup** tabs: Detected media · Links on page · Downloads (live via native events) · Settings.
  Item actions: Download, Queue (start=false), Copy URL, Open source.
- **Drag & drop / send URL**: the toolbar action accepts dropped links; keyboard shortcut
  `Alt+Shift+D` sends the current page URL.
- **Privacy**: nothing leaves the machine except to the local host; no analytics; the extension
  stores only its settings and site exclusions in `chrome.storage.local`.
