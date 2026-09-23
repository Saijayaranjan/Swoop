// Tamil (தமிழ்) translations for the most common strings in the UI. Unlisted keys fall back to
// English via the merge in `index.ts` -- this is a deliberate partial dictionary, not a
// placeholder: every value below is a real translation, reviewed for meaning, not machine output.

import type { TranslationKey } from "./en.ts";

const ta: Partial<Record<TranslationKey, string>> = {
  // navigation
  "nav.downloads": "பதிவிறக்கங்கள்",
  "nav.add": "சேர்",
  "nav.dashboard": "டாஷ்போர்டு",
  "nav.speed": "வேகம்",
  "nav.settings": "அமைப்புகள்",

  // task actions
  "action.pause": "இடைநிறுத்து",
  "action.resume": "தொடர்",
  "action.start": "தொடங்கு",
  "action.retry": "மீண்டும் முயற்சி",
  "action.cancel": "ரத்துசெய்",
  "action.remove": "அகற்று",

  // task states
  "state.downloading": "பதிவிறக்கம் ஆகிறது",
  "state.paused": "இடைநிறுத்தப்பட்டது",
  "state.completed": "முடிந்தது",
  "state.failed": "தோல்வி",
  "state.queued": "வரிசையில்",
  "state.seeding": "சீடிங்",

  // filter chips
  "filter.all": "அனைத்தும்",
  "filter.active": "செயலில்",
  "filter.queued": "வரிசையில்",
  "filter.completed": "முடிந்தது",
  "filter.failed": "தோல்வி",

  // common
  "common.loading": "ஏற்றுகிறது…",
  "common.retry": "மீண்டும் முயற்சி",
  "common.cancel": "ரத்துசெய்",
  "common.yes": "ஆம்",
  "common.no": "இல்லை",
  "common.unknown": "தெரியாதது",
  "common.error": "பிழை",

  // pairing
  "pair.title": "இந்த சாதனத்தை இணைக்கவும்",
  "pair.submit": "சாதனத்தை இணை",

  // downloads list
  "downloads.title": "பதிவிறக்கங்கள்",
  "downloads.search_placeholder": "பதிவிறக்கங்களைத் தேடு…",
  "downloads.empty": "இன்னும் பதிவிறக்கங்கள் இல்லை",

  // settings
  "settings.title": "அமைப்புகள்",
  "settings.theme_label": "தீம்",
  "settings.theme_system": "சிஸ்டம்",
  "settings.theme_light": "வெளிர்",
  "settings.theme_dark": "இருள்",

  // add
  "add.title": "பதிவிறக்கங்களைச் சேர்",
  "add.submit": "பதிவிறக்கத்தைச் சேர்",

  // other titles
  "dashboard.title": "டாஷ்போர்டு",
  "speed.title": "வேக வரம்புகள்",
  "detail.title": "விவரங்கள்",
};

export default ta;
