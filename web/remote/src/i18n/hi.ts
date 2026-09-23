// Hindi (हिन्दी) translations for the most common strings in the UI. Unlisted keys fall back
// to English via the merge in `index.ts` -- this is a deliberate partial dictionary, not a
// placeholder: every value below is a real translation, reviewed for meaning, not machine output.

import type { TranslationKey } from "./en.ts";

const hi: Partial<Record<TranslationKey, string>> = {
  // navigation
  "nav.downloads": "डाउनलोड",
  "nav.add": "जोड़ें",
  "nav.dashboard": "डैशबोर्ड",
  "nav.speed": "स्पीड",
  "nav.settings": "सेटिंग्स",

  // task actions
  "action.pause": "रोकें",
  "action.resume": "फिर से शुरू करें",
  "action.start": "शुरू करें",
  "action.retry": "पुनः प्रयास करें",
  "action.cancel": "रद्द करें",
  "action.remove": "हटाएं",

  // task states
  "state.downloading": "डाउनलोड हो रहा है",
  "state.paused": "रुका हुआ",
  "state.completed": "पूर्ण",
  "state.failed": "विफल",
  "state.queued": "कतार में",
  "state.seeding": "सीडिंग",

  // filter chips
  "filter.all": "सभी",
  "filter.active": "सक्रिय",
  "filter.queued": "कतार में",
  "filter.completed": "पूर्ण",
  "filter.failed": "विफल",

  // common
  "common.loading": "लोड हो रहा है…",
  "common.retry": "पुनः प्रयास करें",
  "common.cancel": "रद्द करें",
  "common.yes": "हां",
  "common.no": "नहीं",
  "common.unknown": "अज्ञात",
  "common.error": "त्रुटि",

  // pairing
  "pair.title": "इस डिवाइस को पेयर करें",
  "pair.submit": "डिवाइस पेयर करें",

  // downloads list
  "downloads.title": "डाउनलोड",
  "downloads.search_placeholder": "डाउनलोड खोजें…",
  "downloads.empty": "अभी तक कोई डाउनलोड नहीं",

  // settings
  "settings.title": "सेटिंग्स",
  "settings.theme_label": "थीम",
  "settings.theme_system": "सिस्टम",
  "settings.theme_light": "लाइट",
  "settings.theme_dark": "डार्क",

  // add
  "add.title": "डाउनलोड जोड़ें",
  "add.submit": "डाउनलोड जोड़ें",

  // other titles
  "dashboard.title": "डैशबोर्ड",
  "speed.title": "स्पीड सीमा",
  "detail.title": "विवरण",
};

export default hi;
