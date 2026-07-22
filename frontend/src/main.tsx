import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  createRootRoute,
  createRoute,
  createRouter,
  Link,
  Outlet,
  RouterProvider,
  useRouterState,
} from "@tanstack/react-router";

import "./styles.css";
import { inTauri } from "./ipc/api";
import { Library } from "./routes/Library";
import { Presets } from "./routes/Presets";
import { SongWorkspace } from "./routes/SongWorkspace";
import { Skills } from "./routes/Skills";
import { Builder } from "./routes/Builder";
import { ComposerRoute } from "./routes/ComposerRoute";
import { Outlines } from "./routes/Outlines";
import { SettingsPage } from "./routes/Settings";
import { GlobalTerminal } from "./components/GlobalTerminal";
import { FieldDrawerProvider } from "./components/FieldDrawer";

function Shell() {
  const path = useRouterState({ select: (s) => s.location.pathname });
  const is = (p: string) => (p === "/" ? path === "/" : path.startsWith(p));
  const [chatOpen, setChatOpen] = React.useState(false);
  const songId = path.match(/^\/song\/([^/]+)/)?.[1];
  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          SONGSMITH STUDIO
          <small>co-write with claude</small>
        </div>
        <nav className="nav">
          <Link to="/" className={is("/") ? "active" : ""}>Library</Link>
          <Link to="/presets" className={is("/presets") ? "active" : ""}>Style presets</Link>
          <Link to="/builder" className={is("/builder") ? "active" : ""}>Chord Builder</Link>
          <Link to="/outlines" className={is("/outlines") ? "active" : ""}>Outlines</Link>
          <Link to="/composer" className={is("/composer") ? "active" : ""}>Composer</Link>
          <Link to="/skills" className={is("/skills") ? "active" : ""}>Skills</Link>
          <Link to="/settings" className={is("/settings") ? "active" : ""}>Settings</Link>
        </nav>
        {/* per-song workspace nav portals in here while a song is open */}
        <div id="song-nav-slot" className="song-nav" />
        <div className="spacer" />
        <div className="foot">
          {inTauri ? "tauri • claude engine" : "browser • mock data"}
          <br />
          v0.1.0 · GPL-3.0
        </div>
      </aside>
      <main className="main">
        <Outlet />
      </main>
      <GlobalTerminal open={chatOpen} onToggle={() => setChatOpen((o) => !o)} songId={songId} />
    </div>
  );
}

const rootRoute = createRootRoute({ component: Shell });
const indexRoute = createRoute({ getParentRoute: () => rootRoute, path: "/", component: Library });
const presetsRoute = createRoute({ getParentRoute: () => rootRoute, path: "/presets", component: Presets });
const songRoute = createRoute({ getParentRoute: () => rootRoute, path: "/song/$id", component: SongWorkspace });
const skillsRoute = createRoute({ getParentRoute: () => rootRoute, path: "/skills", component: Skills });
const builderRoute = createRoute({ getParentRoute: () => rootRoute, path: "/builder", component: Builder });
const composerRoute = createRoute({ getParentRoute: () => rootRoute, path: "/composer", component: ComposerRoute });
const outlinesRoute = createRoute({ getParentRoute: () => rootRoute, path: "/outlines", component: Outlines });
const settingsRoute = createRoute({ getParentRoute: () => rootRoute, path: "/settings", component: SettingsPage });

const routeTree = rootRoute.addChildren([indexRoute, presetsRoute, songRoute, skillsRoute, builderRoute, composerRoute, outlinesRoute, settingsRoute]);
const router = createRouter({ routeTree });
declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}

const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } } });

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <FieldDrawerProvider>
        <RouterProvider router={router} />
      </FieldDrawerProvider>
    </QueryClientProvider>
  </React.StrictMode>,
);
