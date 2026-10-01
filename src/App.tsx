import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";

import DisplayPicker from "./pages/DisplayPicker";
import Editor from "./pages/Editor";
import Recorder from "./pages/Recorder";
import Settings from "./pages/Settings";

/**
 * `?picker=N` is appended by the Rust `open_display_picker` command when it
 * creates the per-monitor overlay windows. Those windows render *only* the
 * picker — no router and no app chrome — while the main window renders the app.
 */
function pickerIndex(): number | null {
  const raw = new URLSearchParams(window.location.search).get("picker");
  if (raw === null) return null;
  const value = Number.parseInt(raw, 10);
  return Number.isNaN(value) ? null : value;
}

/**
 * `?editor=1&session=<id>` is appended by `open_editor` when it hands a finished
 * recording over to a second window. Like the picker, the editor renders on its
 * own — it has no routes to navigate between.
 */
function editorSession(): string | null {
  const params = new URLSearchParams(window.location.search);
  if (params.get("editor") !== "1") return null;
  return params.get("session") ?? "";
}

export default function App() {
  const picker = pickerIndex();
  const session = editorSession();

  if (picker !== null) {
    return <DisplayPicker index={picker} />;
  }

  if (session !== null) {
    // The editor still renders inside a router so AppShell's header controls
    // (which use `useNavigate`) keep working.
    return (
      <BrowserRouter>
        <Editor sessionId={session} />
      </BrowserRouter>
    );
  }

  return (
    <BrowserRouter>
      <Routes>
        <Route path="/" element={<Navigate to="/recorder" replace />} />
        <Route path="/recorder" element={<Recorder />} />
        <Route path="/settings" element={<Settings />} />
        <Route path="*" element={<Navigate to="/recorder" replace />} />
      </Routes>
    </BrowserRouter>
  );
}
