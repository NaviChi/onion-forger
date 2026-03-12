import React from "react";
import ReactDOM from "react-dom/client";
import "./index.css";

class AppErrorBoundary extends React.Component<
  { children: React.ReactNode },
  { error: Error | null }
> {
  state = { error: null as Error | null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error("[ui] render boundary caught an error", error, info);
  }

  render() {
    if (this.state.error) {
      return (
        <div
          style={{
            minHeight: "100vh",
            display: "grid",
            placeItems: "center",
            padding: "32px",
            background: "#040308",
            color: "#f8fafc",
          }}
        >
          <div
            style={{
              width: "min(720px, 100%)",
              border: "1px solid rgba(255, 0, 85, 0.35)",
              background: "rgba(20, 12, 40, 0.94)",
              borderRadius: "14px",
              boxShadow: "0 18px 54px rgba(0, 0, 0, 0.45)",
              padding: "24px",
              fontFamily: "Inter, sans-serif",
            }}
          >
            <div style={{ fontSize: "0.8rem", color: "#ff6b9d", letterSpacing: "0.08em", textTransform: "uppercase" }}>
              UI Recovery
            </div>
            <h1 style={{ margin: "10px 0 12px", fontSize: "1.35rem" }}>Renderer failed before the window could stay interactive.</h1>
            <p style={{ margin: 0, color: "#c5bfd9", lineHeight: 1.6 }}>
              The app caught a render exception instead of dropping to a blank frame.
            </p>
            <pre
              style={{
                margin: "16px 0 0",
                padding: "14px",
                background: "rgba(0, 0, 0, 0.32)",
                borderRadius: "10px",
                border: "1px solid rgba(255, 255, 255, 0.06)",
                color: "#e5e7eb",
                whiteSpace: "pre-wrap",
                wordBreak: "break-word",
                fontFamily: "JetBrains Mono, monospace",
                fontSize: "0.8rem",
              }}
            >
              {this.state.error.message}
            </pre>
            <button
              type="button"
              onClick={() => window.location.reload()}
              style={{
                marginTop: "18px",
                borderRadius: "8px",
                border: "1px solid rgba(0, 229, 255, 0.35)",
                background: "rgba(0, 229, 255, 0.12)",
                color: "#00E5FF",
                fontFamily: "JetBrains Mono, monospace",
                fontSize: "0.85rem",
                padding: "10px 16px",
                cursor: "pointer",
              }}
            >
              Reload UI
            </button>
          </div>
        </div>
      );
    }

    return this.props.children;
  }
}

function shouldForceBrowserAppSurface(): boolean {
  if (typeof window === "undefined") {
    return false;
  }

  const params = new URLSearchParams(window.location.search);
  return params.get("surface") === "app";
}

function AppBootstrap() {
  const [Component, setComponent] = React.useState<React.ComponentType | null>(null);

  React.useEffect(() => {
    const isTauriRuntime = typeof (window as any).__TAURI_INTERNALS__ !== "undefined";
    const loader =
      isTauriRuntime || shouldForceBrowserAppSurface()
        ? import("./App")
        : import("./BrowserPreviewApp");
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
    <AppErrorBoundary>
      <AppBootstrap />
    </AppErrorBoundary>
  </React.StrictMode>,
);
