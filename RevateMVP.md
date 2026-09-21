# Revate — MVP Specification

> A local-first, open-source screen recorder and auto-editor that turns raw
> screen captures into polished, shareable videos — automatically.

Revate records your screen, cursor, keyboard, active windows, and (optionally)
microphone audio. After recording, it produces a clean, engaging video by
applying smart zooms, speed ramps, click/typing sound effects, active-window
chapters, and privacy blurring — all offline, no accounts, no cloud.

---

## 1. Goals & Non-Goals

### Goals (MVP)

- Record screen (full screen or a single window).
- Record cursor position, clicks, and keyboard events with high-res timestamps.
- Record active window focus changes.
- Optionally record microphone audio.
- After recording, automatically apply:
  - Cursor-following auto-zoom on clicks and dwell.
  - Activity-aware speed ramping (skip idle, keep action at 1x).
  - Active-window chapter markers + window-aware zoom targets.
  - Auto-SFX (click + typing) mixed into the final audio.
  - Smart redaction / auto-blur for marked regions.
- Export a single MP4.
- 100% offline. No login, no telemetry, no network calls.

### Non-Goals (MVP)

- Multi-track timeline editor with manual cuts.
- Webcam overlay / face cam.
- Cloud sharing, links, comments.
- Collaboration or accounts.
- Transcription / captions (planned for v2).
- Live streaming.

---

## 2. Target User

- Developers recording coding tutorials.
- Designers recording product walkthroughs.
- Founders recording async demos.
- Anyone who wants a clean recording without opening Premiere.

---

## 3. Core Features

### 3.1 Recording

| Feature                | Description                                    |
| ---------------------- | ---------------------------------------------- |
| Full-screen capture    | Entire display, all monitors optional          |
| Window capture         | User picks a specific window                   |
| Cursor tracking        | Position sampled at ≥ 60 Hz                    |
| Click tracking         | Button down/up with timestamps                 |
| Keyboard tracking      | Key down/up (key codes only, never characters) |
| Active window tracking | Focus changes with window rects                |
| Microphone capture     | Optional, separate track                       |
| System audio capture   | Optional (platform-dependent)                  |
| Pause / resume         | Segments stitched in post                      |

### 3.2 Auto-Zoom

- Zoom triggered by:
  - Clicks (primary trigger).
  - Cursor dwell (staying in a small region for > N ms).
  - Active window changes (optional).
- Zoom targets:
  - Click point + surrounding region.
  - Active window rect.
  - Cursor-following viewport at a fixed zoom.
- Motion smoothing via critically-damped spring.
- Ease-in / ease-out on enter and exit.
- Configurable zoom level (1.5x – 3x) and duration.

### 3.3 Activity-Aware Speed Ramping

- Detect idle periods: no input + no screen change.
- Speed up idle segments (4x – 10x) or cut them entirely.
- Return to 1x on activity.
- Smooth transitions between speeds (short ease).

### 3.4 Active-Window Chapters + Smarter Zoom

- Log focus changes with window title and rect.
- Generate chapter markers on the timeline.
- Zoom to active window instead of cursor when the user is scrolling or reading.

### 3.5 Auto-SFX

- Two bundled samples: `click.wav`, `typing.wav`.
- Place a click sample at every mouse-button event.
- Place a typing sample at every key-down event (ignoring key repeats).
- Randomize pitch/volume slightly for natural feel.
- Duck the original audio slightly during SFX for clarity.
- Mixed into the final export in post-processing.

### 3.6 Smart Redaction / Auto-Blur

- Hotkey during recording marks a screen region as sensitive.
- Region is tracked across frames.
- Blur/pixelate applied in the compositor.
- Retroactive: once marked, all occurrences are blurred.

---

## 4. Tech Stack

| Layer           | Choice                                                | Why                                          |
| --------------- | ----------------------------------------------------- | -------------------------------------------- |
| Desktop shell   | **Tauri 2.x**                                         | Small binaries, Rust-native backend, web UI  |
| Frontend        | **TypeScript + React + Vite**                         | Fast DX, huge ecosystem                      |
| Styling         | **Tailwind CSS**                                      | Rapid UI iteration                           |
| Backend         | **Rust**                                              | Performance for capture, encode, compositing |
| Screen capture  | `scap`                                                | Cross-platform, native APIs                  |
| Global input    | `rdev`                                                | Clicks + keys, cross-platform                |
| Cursor position | `global-mousemove` or `rdev`                          | High-frequency polling                       |
| Window tracking | platform crates (`windows`, `core-graphics`, `x11rb`) | Native focus + rects                         |
| Audio capture   | `cpal`                                                | Mic + system audio where supported           |
| Audio file I/O  | `hound`                                               | Simple WAV read/write                        |
| Video encode    | `ffmpeg` (via CLI or `ffmpeg-sidecar`)                | Battle-tested                                |
| GPU compositing | `wgpu` (optional, later)                              | Shaders for blur, spotlight                  |
| Event log       | **JSON Lines** (`.revents`)                           | Append-only, easy to debug                   |
| Project file    | **JSON** (`.revate`)                                  | Human-readable, versioned                    |

> **Note:** MVP uses FFmpeg for encoding and simple CPU compositing. GPU
> compositing with `wgpu` is a v2 target.

---

## 5. System Architecture

```
┌───────────────────────────────────────────────────────────────┐
│                        Tauri App (Revate)                     │
│                                                               │
│  ┌─────────────────────────┐    ┌──────────────────────────┐  │
│  │   Frontend (TypeScript) │◄──►│   Backend (Rust)         │  │
│  │                         │    │                          │  │
│  │  - Recorder UI          │    │  - Capture               │  │
│  │  - Editor UI            │    │  - Input tracker         │  │
│  │  - Timeline             │    │  - Window tracker        │  │
│  │  - Settings             │    │  - Audio capture         │  │
│  │  - State (Zustand)      │    │  - Event log             │  │
│  │                         │    │  - Zoom engine           │  │
│  │  Tauri commands (IPC)   │    │  - Speed ramper          │  │
│  └─────────────────────────┘    │  - Redaction engine      │  │
│                                 │  - SFX generator         │  │
│                                 │  - Compositor            │  │
│                                 │  - Encoder (FFmpeg)      │  │
│                                 │  - Project I/O           │  │
│                                 └──────────────────────────┘  │
└───────────────────────────────────────────────────────────────┘
```

### 5.1 Recording Pipeline

```
   ┌────────────┐   ┌────────────┐   ┌─────────────┐   ┌──────────────┐
   │ Screen     │   │ Input      │   │ Window      │   │ Audio        │
   │ Capture    │   │ Tracker    │   │ Tracker     │   │ Capture      │
   │ (raw.mp4)  │   │ (events)   │   │ (events)    │   │ (audio.wav)  │
   └─────┬──────┘   └─────┬──────┘   └──────┬──────┘   └──────┬───────┘
         │                │                 │                 │
         └────────────────┴────────┬────────┴─────────────────┘
                                   ▼
                        ┌─────────────────────┐
                        │  Session Manager    │
                        │  - timestamps       │
                        │  - clock sync       │
                        │  - event log writer │
                        └─────────┬───────────┘
                                  ▼
                        ┌─────────────────────┐
                        │  Project Folder     │
                        │  /raw.mp4           │
                        │  /audio.wav         │
                        │  /events.revents    │
                        │  /session.revate    │
                        └─────────────────────┘
```

### 5.2 Editing / Export Pipeline

```
   ┌─────────────┐   ┌─────────────┐   ┌──────────────┐
   │ raw.mp4     │   │ events      │   │ audio.wav    │
   └──────┬──────┘   └──────┬──────┘   └──────┬───────┘
          │                 │                 │
          ▼                 ▼                 ▼
   ┌─────────────────────────────────────────────────┐
   │              Analysis Layer                     │
   │  - Zoom planner                                 │
   │  - Speed-ramp planner                           │
   │  - Chapter planner                              │
   │  - Redaction planner                            │
   └──────────────────────┬──────────────────────────┘
                          ▼
   ┌─────────────────────────────────────────────────┐
   │             Render / Composite Layer            │
   │  - Apply zoom transform per frame               │
   │  - Apply speed remap                            │
   │  - Apply blur / redaction                       │
   │  - Render cursor + spotlight                    │
   └──────────────────────┬──────────────────────────┘
                          ▼
   ┌─────────────────────────────────────────────────┐
   │              Audio Layer                        │
   │  - Generate SFX track from events               │
   │  - Mix SFX + original audio                     │
   └──────────────────────┬──────────────────────────┘
                          ▼
   ┌─────────────────────────────────────────────────┐
   │              Encoder (FFmpeg)                   │
   │  - Video + audio mux                            │
   │  - H.264 / H.265 output                         │
   └──────────────────────┬──────────────────────────┘
                          ▼
                    final.mp4
```

---

## 6. Module Breakdown (Rust Backend)

### `capture/`

- `screen.rs` — init and drive `scap`; produce raw frames or MP4.
- `window_enum.rs` — list capturable windows.
- `session.rs` — orchestrate capture lifecycle (start, pause, stop).

### `input/`

- `listener.rs` — spawn `rdev` listener thread; emit events via channel.
- `cursor.rs` — poll cursor position at high frequency.
- `keyboard.rs` — filter key repeats, build modifier state.
- `event.rs` — event structs with timestamps.

### `window_track/`

- `mod.rs` — cross-platform trait `WindowTracker`.
- `windows.rs`, `macos.rs`, `linux.rs` — platform implementations.
- `focus.rs` — poll foreground window every N ms.

### `audio/`

- `mic.rs` — `cpal` input stream → WAV.
- `system.rs` — platform-specific system audio (macOS, Windows).
- `mixer.rs` — combine tracks.

### `events/`

- `log.rs` — append-only JSON Lines writer.
- `reader.rs` — streaming reader for the editor.
- `schema.rs` — event type definitions + versioning.

### `zoom/`

- `planner.rs` — scan events → list of zoom segments.
- `spring.rs` — critically-damped spring for cursor following.
- `easing.rs` — ease curves.
- `viewport.rs` — compute viewport rect per frame.

### `speed/`

- `idle.rs` — detect idle windows from events + frame diff.
- `planner.rs` — build speed-ramp segments.
- `remap.rs` — map output time → source time.

### `chapters/`

- `planner.rs` — group window focus changes into chapters.
- `labels.rs` — derive chapter titles from window titles.

### `redaction/`

- `marker.rs` — hotkey-marked regions during recording.
- `tracker.rs` — follow region across frames.
- `blur.rs` — apply blur/pixelate.

### `sfx/`

- `samples.rs` — load bundled WAVs.
- `scheduler.rs` — place samples on timeline.
- `synth.rs` — build the SFX audio buffer.
- `mix.rs` — duck + mix with original.

### `compositor/`

- `frame.rs` — frame representation.
- `renderer.rs` — apply transforms (zoom, blur, cursor).
- `cursor_overlay.rs` — redraw cursor with shape.
- `spotlight.rs` — optional focus dimming.

### `encode/`

- `ffmpeg.rs` — wrap FFmpeg CLI or sidecar.
- `progress.rs` — parse FFmpeg progress output.

### `project/`

- `format.rs` — `.revate` project file schema.
- `io.rs` — read/write.
- `migrate.rs` — schema version migrations.

### `commands/`

- `recording.rs` — Tauri commands for start/stop/pause.
- `editing.rs` — commands for zoom/speed/redaction params.
- `export.rs` — kick off export + progress events.
- `files.rs` — file dialogs, project listing.

### `utils/`

- `clock.rs` — shared monotonic clock.
- `paths.rs` — app data dirs.
- `logging.rs` — structured logs.
- `errors.rs` — error types.

---

## 7. Data Formats

### 7.1 Event Log — `.revents` (JSON Lines)

One JSON object per line. Append-only. Cheap to write, cheap to stream.

```jsonl
{"t":0.000,"type":"session_start","data":{"version":1,"screen":{"w":2560,"h":1440}}}
{"t":0.512,"type":"cursor_move","data":{"x":812,"y":540}}
{"t":0.847,"type":"mouse_down","data":{"x":900,"y":620,"button":"left"}}
{"t":0.912,"type":"mouse_up","data":{"x":900,"y":620,"button":"left"}}
{"t":1.104,"type":"key_down","data":{"code":"KeyP","mods":["Meta","Shift"]}}
{"t":1.152,"type":"key_up","data":{"code":"KeyP"}}
{"t":2.340,"type":"window_focus","data":{"id":"0x1a2b","title":"Visual Studio Code","rect":{"x":0,"y":0,"w":1920,"h":1080}}}
{"t":5.900,"type":"redaction_mark","data":{"rect":{"x":400,"y":300,"w":200,"h":40},"label":"api-key"}}
{"t":30.100,"type":"session_stop","data":{}}
```

### 7.2 Project File — `.revate` (JSON)

```json
{
  "version": 1,
  "name": "demo-2025-01-15",
  "sources": {
    "video": "raw.mp4",
    "audio": "audio.wav",
    "events": "events.revents"
  },
  "settings": {
    "zoom": {
      "enabled": true,
      "level": 2.0,
      "followCursor": true,
      "clickZoom": true,
      "dwellMs": 800
    },
    "speed": {
      "enabled": true,
      "idleThresholdSec": 2.0,
      "idleSpeed": 6.0,
      "cutInsteadOfSpeed": false
    },
    "sfx": {
      "enabled": true,
      "clickVolume": 0.5,
      "typingVolume": 0.4,
      "randomize": true
    },
    "redaction": {
      "regions": []
    },
    "chapters": { "enabled": true }
  }
}
```

---

## 8. Project Folder Layout (on disk)

```
<AppData>/Revate/projects/<project-id>/
├── raw.mp4
├── audio.wav
├── events.revents
├── session.revate
├── thumbnails/
│   └── 0001.jpg ...
├── cache/
│   └── analysis.json
└── exports/
    └── final.mp4
```

---

## 9. Repo Layout

```
revate/
├── RevateMVP.md
├── README.md
├── LICENSE                      (MIT or Apache-2.0)
├── .gitignore
├── .editorconfig
├── package.json
├── pnpm-lock.yaml
├── tsconfig.json
├── vite.config.ts
├── tailwind.config.ts
├── postcss.config.js
├── index.html
│
├── assets/
│   └── sfx/
│       ├── click.wav
│       ├── typing.wav
│       └── LICENSE.md
│
├── public/
│   └── icons/
│
├── src/                         # Frontend
│   ├── main.tsx
│   ├── App.tsx
│   ├── styles/
│   │   └── globals.css
│   ├── components/
│   │   ├── recorder/
│   │   ├── editor/
│   │   ├── timeline/
│   │   ├── settings/
│   │   └── ui/
│   ├── pages/
│   │   ├── Home.tsx
│   │   ├── Recorder.tsx
│   │   ├── Editor.tsx
│   │   └── Settings.tsx
│   ├── hooks/
│   ├── lib/
│   │   ├── tauri.ts
│   │   └── format.ts
│   ├── state/
│   │   └── store.ts
│   └── types/
│       └── events.ts
│
└── src-tauri/                   # Rust backend
    ├── Cargo.toml
    ├── build.rs
    ├── tauri.conf.json
    ├── icons/
    └── src/
        ├── main.rs
        ├── lib.rs
        ├── capture/
        ├── input/
        ├── window_track/
        ├── audio/
        ├── events/
        ├── zoom/
        ├── speed/
        ├── chapters/
        ├── redaction/
        ├── sfx/
        ├── compositor/
        ├── encode/
        ├── project/
        ├── commands/
        └── utils/
```

---

## 10. Cross-Cutting Concerns & Challenges

### 10.1 Clock Synchronization

- Video frames, input events, window events, and audio must share a **single monotonic clock**.
- Strategy: one `Instant::now()` captured at session start; all components measure
  elapsed time from it. Store as `f64` seconds.
- Audio uses sample counts; convert to seconds using `SAMPLE_RATE`.
- Drift is real; periodically re-anchor long recordings.

### 10.2 Capture Performance

- Screen capture at 1080p60 is CPU/GPU heavy. Use hardware encoders
  (`h264_nvenc`, `h264_videotoolbox`, `h264_qsv`) via FFmpeg.
- Don't block the capture thread with file I/O; use a bounded channel + writer thread.

### 10.3 Input Privacy

- Log **key codes only** (e.g. `"KeyP"`), never characters.
- Never transmit anything; logs stay local.
- Provide an obvious "recording" indicator.

### 10.4 Cross-Platform Window Tracking

- macOS: needs Accessibility + Screen Recording permissions.
- Windows: `GetForegroundWindow` + `DwmGetWindowAttribute` for true rect.
- Linux: X11 easy; Wayland depends on compositor (`wlr-foreign-toplevel`).
- Abstract behind a trait; ship with a no-op fallback.

### 10.5 Auto-Zoom Quality

- Naive zoom looks jittery. Use a **critically-damped spring** for camera position.
- Merge zoom segments that overlap or are close in time.
- Ignore tiny cursor movements (dead zone).
- Tune enter/exit durations empirically.

### 10.6 Speed Ramping

- Screen-change detection: downscale frames to 64×64 grayscale, compute mean
  absolute difference. Below threshold for N seconds → idle.
- Must not cut mid-sentence in audio; check for audio activity too.
- Time remap must stay monotonic and align audio + video.

### 10.7 SFX Naturalness

- Randomize pitch ±5% and volume ±2 dB per sample.
- Ignore key repeats (key down while already down).
- Different samples for different key classes (letter, space, enter, backspace) — v2.

### 10.8 Redaction Tracking

- Simple case: static region (e.g. a terminal panel). Rect constant.
- Hard case: region moves (scrolling). Use template matching or optical flow.
- MVP: static regions + a "lock to window" option.

### 10.9 Export Performance

- Full re-encode is slow. Prefer stream copy when no video transform applies.
- Show progress via FFmpeg stderr parsing.
- Run export in a background thread with cancel support.

### 10.10 Error Handling & UX

- Never lose a recording: write raw files incrementally; if the app crashes,
  the session folder should still be recoverable.
- Validate project files on load; show clear errors.
- Recording indicator must always be visible.

### 10.11 Permissions Onboarding

- First run: guided screen explaining macOS/Windows/Linux permission needs.
- Detect missing permissions and deep-link to system settings.

### 10.12 File Size & Storage

- Raw recordings get big fast. Show estimated size before recording.
- Auto-cleanup of cache/thumbnails older than N days.

### 10.13 Testing Without a Screen

- CI can't record a real screen. Abstract capture behind a trait and provide
  a `MockCapture` that reads a folder of PNGs.
- Same for input and window tracking.

---

## 11. MVP Milestones

| #   | Milestone  | Deliverable                                            |
| --- | ---------- | ------------------------------------------------------ |
| 0   | Scaffold   | Repo structure, Tauri app boots, empty modules         |
| 1   | Capture    | Full-screen recording → raw.mp4                        |
| 2   | Events     | Cursor + clicks + keys + window focus → events.revents |
| 3   | Project    | `.revate` file written on stop; editor loads it        |
| 4   | Auto-Zoom  | Zoom applied in export based on events                 |
| 5   | SFX        | Click + typing sounds mixed into export                |
| 6   | Speed Ramp | Idle segments sped up                                  |
| 7   | Chapters   | Window changes → chapters on timeline                  |
| 8   | Redaction  | Hotkey-marked regions blurred                          |
| 9   | Polish     | Permissions onboarding, progress UI, error recovery    |

---

## 12. Open Questions

- Which frontend state library? (Zustand recommended.)
- Bundle FFmpeg as sidecar or require system install? (Sidecar recommended.)
- Ship system-audio capture on Linux in v1? (Probably defer.)
- Redaction: how far to go with motion tracking in MVP?
- SFX licensing: use CC0 samples, credit in `assets/sfx/LICENSE.md`.

---

## 13. License

MIT (or Apache-2.0). All bundled assets must be compatible.
