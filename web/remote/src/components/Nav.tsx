import type { ViewName } from "../router/router.ts";
import { useRoute } from "../router/useRouter.ts";
import { useT } from "../i18n/useT.ts";
import { Icon, type IconName } from "./Icon.tsx";

const ITEMS: { view: ViewName; icon: IconName; labelKey: "nav.downloads" | "nav.add" | "nav.dashboard" | "nav.speed" | "nav.settings" }[] = [
  { view: "downloads", icon: "list", labelKey: "nav.downloads" },
  { view: "add", icon: "plus", labelKey: "nav.add" },
  { view: "dashboard", icon: "dashboard", labelKey: "nav.dashboard" },
  { view: "speed", icon: "speed", labelKey: "nav.speed" },
  { view: "settings", icon: "settings", labelKey: "nav.settings" },
];

function isCurrent(view: ViewName, current: ViewName): boolean {
  return view === current || (view === "downloads" && current === "detail");
}

/** Bottom tab bar (phones) and sidebar (>=900px) sharing one item list; CSS picks which shows. */
export function Nav() {
  const [route, navigate] = useRoute();
  const t = useT();

  return (
    <>
      <nav class="tabbar" aria-label={t("app.name")}>
        {ITEMS.map((item) => (
          <button
            key={item.view}
            type="button"
            class={`tabbar-item${isCurrent(item.view, route.view) ? " is-active" : ""}`}
            aria-current={isCurrent(item.view, route.view) ? "page" : undefined}
            onClick={() => navigate({ view: item.view, taskId: null })}
          >
            <Icon name={item.icon} />
            <span>{t(item.labelKey)}</span>
          </button>
        ))}
      </nav>
      <nav class="sidebar" aria-label={t("app.name")}>
        <div class="sidebar-brand">{t("app.name")}</div>
        {ITEMS.map((item) => (
          <button
            key={item.view}
            type="button"
            class={`sidebar-item${isCurrent(item.view, route.view) ? " is-active" : ""}`}
            aria-current={isCurrent(item.view, route.view) ? "page" : undefined}
            onClick={() => navigate({ view: item.view, taskId: null })}
          >
            <Icon name={item.icon} />
            <span>{t(item.labelKey)}</span>
          </button>
        ))}
      </nav>
    </>
  );
}
