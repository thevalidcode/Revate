import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";

import DisplayPicker from "./pages/DisplayPicker";
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

export default function App() {
  const picker = pickerIndex();

  if (picker !== null) {
    return <DisplayPicker index={picker} />;
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
