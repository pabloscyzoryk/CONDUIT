import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { AppErrorBoundary } from "./components/layout/AppErrorBoundary";
import "./styles/tokens.css";
import "./styles/palettes.css";
import "./styles/global.css";
import "./styles/ui.css";
import "./styles/workspace.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <AppErrorBoundary><App /></AppErrorBoundary>
  </StrictMode>,
);
