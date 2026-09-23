import { useEffect } from "preact/hooks";
import { useStore } from "./state/useStore.ts";
import { authStore } from "./state/auth.ts";
import { settingsStore, resolveTheme } from "./state/settings.ts";
import { initEventsLifecycle } from "./state/wsSingleton.ts";
import { apiClient } from "./api/singleton.ts";
import { setTaskSnapshot, setQueueSummaries } from "./state/tasks.ts";
import { useRoute } from "./router/useRouter.ts";
import { Header } from "./components/Header.tsx";
import { Nav } from "./components/Nav.tsx";
import { Toasts } from "./components/Toasts.tsx";
import { Pair } from "./views/Pair.tsx";
import { Downloads } from "./views/Downloads.tsx";
import { Detail } from "./views/Detail.tsx";
import { Add } from "./views/Add.tsx";
import { Dashboard } from "./views/Dashboard.tsx";
import { Speed } from "./views/Speed.tsx";
import { Settings } from "./views/Settings.tsx";

let lifecycleStarted = false;

export function App() {
  const auth = useStore(authStore);
  const settings = useStore(settingsStore);
  const [route] = useRoute();

  useEffect(() => {
    if (!lifecycleStarted) {
      lifecycleStarted = true;
      initEventsLifecycle();
    }
  }, []);

  useEffect(() => {
    const root = document.documentElement;
    root.setAttribute("data-theme", resolveTheme(settings.theme));
  }, [settings.theme]);

  useEffect(() => {
    if (!auth.token) return;
    apiClient.listRows({}).then(setTaskSnapshot).catch(() => {});
    apiClient.queueSummaries().then(setQueueSummaries).catch(() => {});
  }, [auth.token]);

  if (!auth.token) {
    return (
      <>
        <Pair />
        <Toasts />
      </>
    );
  }

  return (
    <div class="app-shell">
      <Header />
      <Nav />
      <main class="app-main" id="main-content">
        {route.view === "downloads" && <Downloads />}
        {route.view === "detail" && <Detail />}
        {route.view === "add" && <Add />}
        {route.view === "dashboard" && <Dashboard />}
        {route.view === "speed" && <Speed />}
        {route.view === "settings" && <Settings />}
        {route.view === "pair" && <Downloads />}
      </main>
      <Toasts />
    </div>
  );
}
