import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles.css";

/** Enables automated teardown/project switch without confirm dialogs (`?e2e=1`). */
function initE2eAutomationFlags(): void {
  const params = new URLSearchParams(window.location.search);
  if (params.get("e2e") === "1") {
    (window as Window & { __TESHI_E2E__?: boolean }).__TESHI_E2E__ = true;
    localStorage.setItem("TESHI_AUTO_TEARDOWN", "1");
  }
}

initE2eAutomationFlags();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
