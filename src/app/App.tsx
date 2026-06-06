import { useState } from "react";

import { I18nProvider } from "./i18n";
import { ThemeProvider } from "./theme";
import { Home } from "./views/Home";
import { Settings } from "./views/Settings";
import { About } from "./views/About";
import { Uninstall } from "./views/Uninstall";
import { CodexConfig } from "./views/CodexConfig";

type View = "home" | "settings" | "about" | "uninstall" | "config";

function Shell() {
  const [view, setView] = useState<View>("home");

  switch (view) {
    case "settings":
      return (
        <Settings
          onBack={() => setView("home")}
          onOpenAbout={() => setView("about")}
          onOpenUninstall={() => setView("uninstall")}
          onOpenConfig={() => setView("config")}
        />
      );
    case "about":
      return <About onBack={() => setView("settings")} />;
    case "uninstall":
      return <Uninstall onBack={() => setView("settings")} />;
    case "config":
      return <CodexConfig onBack={() => setView("settings")} />;
    default:
      return <Home onOpenSettings={() => setView("settings")} />;
  }
}

export function App() {
  return (
    <ThemeProvider>
      <I18nProvider>
        <Shell />
      </I18nProvider>
    </ThemeProvider>
  );
}
