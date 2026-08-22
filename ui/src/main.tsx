import React from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { UiErrorBoundary } from "./UiErrorBoundary";
import "./styles.css";

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <UiErrorBoundary scope="Application" className="application-error-boundary">
      <App />
    </UiErrorBoundary>
  </React.StrictMode>
);
