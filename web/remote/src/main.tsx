import { render } from "preact";
import { App } from "./App.tsx";
import { consumeTokenFromUrl, setTokenOnly } from "./state/auth.ts";
import "./styles/app.css";

// Accept `?token=` from the swoop://pair QR handoff before the app decides whether to show Pair.
const handoffToken = consumeTokenFromUrl();
if (handoffToken) {
  setTokenOnly(handoffToken);
}

const root = document.getElementById("app");
if (root) {
  render(<App />, root);
}
