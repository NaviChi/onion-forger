import React from "react";
import ReactDOM from "react-dom/client";
import "./index.css";

function AppBootstrap() {
  const [Component, setComponent] = React.useState<React.ComponentType | null>(null);

  React.useEffect(() => {
    const isTauriRuntime = typeof (window as any).__TAURI_INTERNALS__ !== "undefined";
    const loader = isTauriRuntime ? import("./App") : import("./BrowserPreviewApp");
    loader.then((mod) => {
      setComponent(() => mod.default);
    });
  }, []);

  if (!Component) {
    return null;
  }

  return <Component />;
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <AppBootstrap />
  </React.StrictMode>,
);
