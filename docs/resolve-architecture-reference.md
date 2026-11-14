# DaVinci Resolve - Clean-Room Architecture Reference

Reference for designing **Picasso Suite**. Built only from public, documented sources shipped with
DaVinci Resolve 21.1 (Windows):

- Developer SDKs in `C:\ProgramData\Blackmagic Design\DaVinci Resolve\Support\Developer\`
  (Scripting, Workflow Integrations, OpenFX, CodecPlugin, DaVinciCTL, Fusion Fuse, Fusion Templates,
  OGraf HTML Templates, LUT)
- The user manual `Documents\DaVinci Resolve.pdf` (4,351 pages)
- File names in the install folder (which third-party libraries ship)

No Resolve binary was disassembled, decompiled or inspected. The licence (§8a) forbids that,
and names AI agents ("Automated Controllers") specifically. Everything below describes **observable
behaviour and published interfaces**. Write Picasso Suite's own implementation from this; do not copy
Blackmagic's sample code (the SDK samples carry their own licences) or assets.

---

## 1. Big picture

Resolve is one application made of **"pages"**: workspaces that edit the same project data
at different stages of post-production.

| Page | Role | Primary data it edits |
|---|---|---|
| Media | Ingest, browse storage, clip metadata, sync audio | Media pool, clip attributes |
| Cut | Fast editing for short-form work (simplified UI) | Timeline |
| Edit | Full NLE: tracks, trimming, effects, titles, multicam | Timeline, timeline items |
| Fusion | Node-based compositing per clip | Fusion composition attached to a timeline item |
| Color | Node-based grading | Grade node graph per clip / group / timeline |
| Fairlight | DAW: mixing, buses, automation, ADR | Audio tracks, buses, mixer state |
| Deliver | Render queue, export formats | Render jobs and presets |
| Photo | Stills editing (new in 21) | Photo albums |

**Key design idea:** pages are *views and editors over one shared project model*, not separate
programs. Every page reads and writes the same Project → MediaPool → Timeline → TimelineItem graph.
The scripting API's object model (§3) is the clearest public view of that model.

### Process / module layout (from shipped file names only)

| Concern | What ships | Implication |
|---|---|---|
| UI toolkit | Qt 5 Widgets + QML/Quick (`Qt5*.dll`, `QtQuick*`) | Native C++ desktop UI; some panels in QML |
| Main app | `Resolve.exe` (~600 MB, monolithic) + `FusionPage.dll`, `FairlightPage.dll`, `BMDDavUI.dll` | Most pages compiled in; two big pages as separate modules |
| Compositor | `fusionsystem`, `fusiongraphics`, `fusionoperators`, `fusioncontrols`, `fusionscript` + `Plugins\*.plugin` | Fusion is its own engine with a plugin system, embedded in Resolve |
| Scripting | LuaJIT (`lua5.1.dll`, `fuscript.exe`), embedded Python 3.14 (`ResolvePython\`) | Scripting runs inside Resolve and also out of process over IPC |
| Plugins out-of-process | `OFXLoader.exe`, `VstScanner.exe` | Third-party plugins are scanned/loaded in helper processes so crashes stay isolated |
| Web | Chromium Embedded Framework (`Plugins\ChromiumEmbeddedFramework`) | HTML graphics (OGraf) rendered offscreen |
| GPU compute | CUDA, OpenCL, Metal (Mac), DirectML, ArrayFire (`libaf`) | One effect is implemented per GPU backend |
| AI | PyTorch (`torch*`), ONNX Runtime, OpenVINO, cuDNN, llama.cpp/ggml (`llama.dll`, `mtmd.dll`) | On-device ML models (masking, transcription, speech, search) |
| Media I/O | FFmpeg 6, Blackmagic RAW, ARRI SDK, ProRes RAW, Sony, Canon, DNx, Kakadu (JPEG 2000/DCP), JPEG XS, JPEG XL, WebP, Intel VPL | Pluggable decoders, mostly vendor SDKs |
| Colour/VFX formats | OpenColorIO 2.5, OpenEXR, OpenImageIO, OpenVDB, USD, Alembic, FBX | Industry-standard open libraries |
| Database | PostgreSQL client (`libpq`), SQLite, Qt SQL | Projects live in a database, not loose files (§2) |
| Audio DSP | Intel IPP/MKL, a spatial-audio framework, Dolby Atmos, Audio Vivid, PortAudio, libsndfile, SoX resampler | |
| Graph layout | Graphviz (`cgraph`, `gvc`) | Probably automatic node-graph layout |
| Logging | log4cxx | |

---

## 2. Project storage model

- **Project libraries** (formerly "databases") hold many projects. There are three types:
  - **Local**: on disk, in a "Resolve Disk Database" folder.
  - **Network**: a PostgreSQL server, which the bundled "Project Server" app manages.
  - **Cloud**: Blackmagic Cloud.
- Projects are rows in the library, not documents on disk. Portable forms are:
  - **`.drp`**: export/import of a single project.
  - **`.dra`**: an archive that includes the media.
  - **`.drt`**: a timeline on its own.
  - **`.drx`**: a grade.
  - **`.picef`**: a bundle of Fusion templates (a renamed zip).
- **Saving:**
  - "Live Save" writes every change incrementally, so there is no explicit save step.
  - Separately, periodic full-project backups are kept on a grandfather-father-son rotation. Stills and LUTs are left out of backups to save space.
  - Timeline backup snapshots can be compared.
- **Collaboration** (network and cloud libraries):
  - Multi-user editing uses **bin locking**. A bin is locked while a user has it selected, and released when they deselect it.
  - Colour and Fusion use **clip locking** in the same way. The Color and Fusion pages keep separate locks, so a colourist and a compositor can work on the same clip at once.
  - Changes from other users appear when the user refreshes. There is also built-in chat.
- **Media is never embedded.** Clips reference files by path and can be relinked. Optimized media,
  proxies and render-cache files are separate sidecar media that the app manages.

**Takeaway for Picasso Suite:** use a database-backed project model with incremental autosave and
fine-grained locks. That one decision unlocks crash safety, versioning and multi-user work. An
embedded SQLite file per library plus optional Postgres is a reasonable equivalent.

---

## 3. The data model (from the scripting API)

The scripting API (`Scripting/DaVinciResolveScript.pyi`, 2,848 lines) exposes the object graph. The
same API is available from Lua, Python and JavaScript (Workflow Integrations).

```
Resolve (app)
├── ProjectManager ── project libraries, folders, projects, cloud projects
│   └── Project
│       ├── settings (≈300 keys: resolution, fps, colour science, caching, monitoring, HDR, audio…)
│       ├── MediaPool
│       │   ├── Folder (tree) ── MediaPoolItem (clip)
│       │   │                     ├── metadata, third-party metadata, markers, flags, clip colour
│       │   │                     ├── clip properties, audio mapping (JSON), proxy/full-res links
│       │   │                     └── transcription, AI analysis results
│       │   └── Timelines
│       ├── Timeline
│       │   ├── settings (can override project settings when useCustomSettings=1)
│       │   ├── tracks: video | audio | subtitle (name, enable, lock, sub-type)
│       │   │   └── TimelineItem (clip / title / generator / transition)
│       │   │        ├── properties: transform, crop, composite, retime, scaling, audio
│       │   │        ├── Fusion comps (0..n per item), colour versions, takes
│       │   │        ├── grade node Graph, ColorGroup membership
│       │   │        └── markers, flags, fades, speed, linked items
│       │   ├── markers, in/out, start timecode
│       │   └── timeline node Graph (timeline-level grade)
│       ├── Gallery ── still albums, PowerGrade albums ── GalleryStill
│       ├── ColorGroup ── pre-clip and post-clip Graphs
│       └── render jobs, render presets, render formats/codecs
├── MediaStorage ── mounted volumes, file browsing, clone/copy with checksums
└── Fusion ── full Fusion scripting object model
```

### Notable modelling choices visible in the API

- **Everything has a stable unique ID** (`GetUniqueId()` on Project, MediaPool, MediaPoolItem,
  Timeline, TimelineItem and Folder). Jobs are addressed by ID, not by index; index-based calls were removed.
- **Settings are flat string key/value maps.** `GetSettings()`/`SetSettings(dict)` return a whole snapshot,
  and keys map directly onto labels in the UI. Project and timeline settings share keys, and a timeline
  overrides the project only when it opts in. `SetSettings` applies keys one at a time and reports which ones failed.
  `TimelineItem.SetProperties` validates every key first and then applies all or none.
- **Timeline item transform properties** (one flat set): Pan, Tilt, ZoomX/Y (+gang), RotationAngle,
  AnchorPointX/Y, Pitch, Yaw, FlipX/Y, Crop L/R/T/B + softness, DynamicZoom + ease, CompositeMode
  (32 blend modes), Opacity, lens Distortion, RetimeProcess (nearest / frame blend / optical flow),
  MotionEstimation, Scaling (crop/fit/fill/stretch), ResizeFilter (16 kernels), audio volume/pan/pitch,
  voice isolation and dialogue leveler.
- **Markers** carry a colour (16 colours), a name, a note, a duration and an opaque `customData` string.
  The custom data lets integrations attach their own payloads.
- **Frames vs timecode:** item positions are frames (`GetStart`, `GetEnd`, `GetLeftOffset`), and the
  playhead is timecode. Drop-frame timecode is set by appending "DF" to the frame-rate string.
- **Audio channel mapping** is JSON. It records embedded channels, linked external audio files with
  sample offsets, and, per track, a format type (mono…7.1, adaptive_1..36) with a list of
  1-based source channels per output channel, where 0 means unconnected.
- **Grades are versioned.** Each item has local and remote colour versions, a node graph of 1-based
  nodes, LUT-per-node, CDL-per-node, per-node cache mode, and DRX grade import.
- **Interchange formats** (Timeline.Export): AAF, EDL (+CDL/SDL), FCP 7 XML, FCPXML 1.8–1.10, OTIO,
  ALE, CSV/TSV, Dolby Vision XML, HDR10 metadata, plus the native DRT.
- **Long-running AI jobs** (transcription, scene-cut detection, smart reframe, subtitles from audio,
  slate detection) are asynchronous. Each has a `Get…Status()` poller.

**Takeaway:** the API is effectively a ready-made spec for a project schema. Mirroring its object
names and property keys in Picasso Suite would also make it easier to write a Resolve
import/export bridge later.

---

## 4. Image processing pipeline

From the manual's "Image Processing Order of Operations" chapter (described here in my own words).
Page order in the UI is **not** processing order: operations from different pages are interleaved
so that each step gets the cleanest possible input.

```
SOURCE
  camera raw decode settings
  → lens correction
  → colour management: input colour space → timeline colour space (or ACES IDT)
  → input LUT → deinterlace → Super Scale (AI upscaling, raises source resolution)
FUSION
  → Fusion page composition (per clip)
TIMELINE (per clip)
  → edit sizing → input sizing → Fusion effects
  → group pre-clip grade → Cut/Edit-page OFX & ResolveFX → clip grade → group post-clip grade
  → speed effects → track compositing (alpha, composite mode, opacity)
  → timeline grade
AFTER THE TIMELINE
  → subtitles → output sizing → output blanking → output LUT
  → colour management: timeline → output colour space (or ACES RRT/ODT)
  → data burn-in → (display LUT for the viewer only) → render/output
```

Within **one Color-page corrector node** there is also a fixed internal order: motion blur → noise
reduction → plugins → auto colour/shot match → recovery/detail/boost → temperature → HDR wheels →
lift/gamma/gain → contrast/pivot → curves → log wheels → hue/sat → LUT → HSL curves → warper →
channel mixer → soft clip → defocus → node sizing. If a user needs a different order, they split the
work across several nodes.

### Engine facts worth copying

- **All image processing runs in 32-bit float per channel.** Fusion Studio can be set to lower bit
  depths, but Resolve always uses float. Values below 0 and above 1 are preserved (HDR, super-whites).
- **Resolution independence.** The timeline resolution is separate from source resolutions and
  from the output resolution. Every sizing stage (edit, input, node, output) is a transform
  that gets concatenated, so pixels are resampled once, with a chosen filter, not at every step.
  Deliver can render at source resolution, or disable sizing so it can be sent back in an XML.
- **Colour management (RCM).** Every clip carries an input colour space and gamma. The timeline
  has a working colour space and working luminance. Input and output DRT tone mapping, gamut mapping
  and inverse DRT (for SDR→HDR) are separate stages. Grading tools can be "colour-space aware". ACES (cc/cct, v1 and v2)
  is an alternative path. Colour management can be bypassed per clip.
- **Data levels.** Processing is always full range internally. Each clip is tagged video- or full-range,
  and conversion happens at input and output.

### Caching (three independent layers)

Resolve keeps three caches so that a change in one layer doesn't invalidate the others:

1. **Fusion output cache** (pre-grade): stores processor-heavy sources (H.264/HEVC, raw) and clips
   with Fusion effects, before colour.
2. **Node cache:** per-node grade caching. A cached node stores its result and everything upstream of it.
   It triggers automatically on heavy operations (noise reduction, motion blur, plugins). It can be
   forced for a whole clip ("color output cache") or per OFX filter on the Edit page. Group pre- and post-grades can be cached too.
3. **Sequence cache:** timeline-level effects such as transitions, opacity, adjustment layers, composites
   and retimes.

Each layer can run in **Smart** (automatic) or **User** (manual) mode. The cache format can be
ProRes, DNxHR or uncompressed 10-bit or 16-bit float; the "HDR" variants keep out-of-range values.
Deliver can reuse cached frames when rendering. **Optimized media** (transcodes of whole clips, made at
ingest) and **proxies** are separate from the render cache and can be shared between projects.

**Takeaway:** build the renderer as a pull-based DAG of operations with content-hash cache keys at
these three boundaries. That gives you the "edit without re-caching grades, grade without
re-caching edits" behaviour.

---

## 5. Audio engine (Fairlight) signal flow

From the manual's signal-flow diagrams:

- **Track channel strip:**
  - Input → record gain → EQ (4-band plus a 2-band filter) → dynamics (gate, compressor, limiter) → inserts (up to 6 plugins) → fader, pan, mute.
  - **EQ, dynamics and inserts can be put in any order.**
  - Aux sends can be pre- or post-fader, pre- or post-mute, and pre- or post-pan, in any combination.
- **Buses:**
  - Up to 8 Main buses, 24 Sub buses, 24 Aux buses and 24 Multi-track buses, plus a Solo bus.
  - Subs and Auxes can be assigned to Mains.
  - Each bus has its own comp, EQ, limiter, insert plugins and meters.
  - Formats run from mono to 24-channel immersive.
- **Control room and studio monitoring:**
  - Mains, subs, auxes, solo or external inputs.
  - Dim, mute, fold-down/up and format check.
  - Loudness metering.
- **Utilities:** VCA groups (32), AFL/PFL solo, talkback, oscillator and white/pink noise generators.
- **Plugins:** VST3 (with a separate scanner process), AU on Mac, and Fairlight FX built in.

**Takeaway:** keep audio as a separate real-time graph (track strips → bus matrix → monitor
section) that runs on its own audio thread, and drive it from the shared timeline model.

---

## 6. Extension points (public SDKs)

This is the most directly reusable part. Each one is a contract Picasso Suite could adopt as is
(OFX, OGraf, .cube are open standards) or reimplement in an equivalent form.

### 6.1 Scripting API — Lua / Python / JavaScript

- **Where scripts run:**
  - From the Workspace → Console.
  - From the Workspace → Scripts menu, which is filled by scanning `Fusion/Scripts/{Utility,Comp,Tool,Edit,Color,Deliver}`.
  - As render start/end hooks.
  - As Fusion "in-tool" scripts on frame, start or end render.
- **Injected globals:** `bmd`, `resolve`, `fusion`. Render hooks also receive the job ID, status and error.
- **External access:**
  - `DaVinciResolveScript.scriptapp("Resolve")` connects from a separate process.
  - It works locally, or over the network on **TCP port 1144** (registered with IANA). The return channel uses an ephemeral port in 49152–65535.
  - This is gated by a preference: None / Local / Network.
- **Headless mode:** `-nogui` launches the app without a UI while scripting still works. That's useful for render farms.
- **UI for scripts:** `fusion.UIManager` plus `bmd.UIDispatcher` give a declarative layout of Qt widgets.
  - Windows are built from nested dicts (ID, Text, children), and events are wired with `win.On.<ID>.<Event> = handler`.
  - The dispatcher runs its own event loop (`RunLoop`/`ExitLoop`).
  - Widgets include labels, buttons, checkboxes, combos, spin boxes, sliders, line/text edits, colour pickers, tabs and trees.
- **Lua vs Python:** Lua represents lists and dicts as indexed tables. The API surface is otherwise identical.

**Design lesson:** one reflective native object model, bound to several languages,
reachable both in process and over IPC. Free and Studio editions share one API; Studio-only calls just
return False.

### 6.2 Workflow Integrations — Electron plugins

- Each plugin is a folder `com.<company>.<name>/` with `manifest.xml` (Id, Name, Version, Description,
  FilePath → `main.js`), `package.json`, `index.html` and `node_modules`.
- It's installed into `%PROGRAMDATA%\Blackmagic Design\DaVinci Resolve\Support\Workflow Integration Plugins\`.
- Resolve lists these plugins under Workspace → Workflow Integrations. Launching one opens its HTML in its own window.
- The native addon `WorkflowIntegration.node` connects the plugin to Resolve:
  - `Initialize(pluginId)` / `InitializePromise`.
  - `GetResolve()` / `GetResolvePromise()` return the same object model as the scripting API, with a promise-returning variant.
  - `RegisterCallback('RenderStart' | 'RenderStop' | 'ResolveQuit', fn)`, plus `SetAPITimeout` and `CleanUp`.
- Since 19.0.2 the Electron windows run sandboxed with context isolation. The addon is loaded in the preload script and exposed over IPC.
- Python or Lua scripts can be workflow integrations too; they get `resolve` and `project` globals.

**Design lesson:** a way for third parties to embed web-UI panels that drive the same object model,
which is how asset-management and review tools integrate.

### 6.3 OpenFX (image effect plugins)

- This is the standard **OpenFX 1.4** C API, plus the open-source C++ support library (both BSD-licensed).
- Bundles are `Name.ofx.bundle/Contents/Win64/Name.ofx`, installed in `C:\Program Files\Common Files\OFX\Plugins`.
  On Windows ARM, Resolve looks in `Win-arm64x`, then `Win-arm64ec`, then `Win64`.
- **Contexts:**
  - Filter: one input.
  - Transition: two inputs and a required "Transition" double parameter.
  - Generator.
- **GPU rendering:**
  - The host passes device buffers directly: a CUDA pointer, a Metal `MTLBuffer` or an OpenCL `cl_mem`.
  - It also passes its CUDA stream or Metal/OpenCL command queue, so plugins can queue work without forcing a sync. Plugins declare support with `kOfxImageEffectPropCudaStreamSupported`.
- **Resolve-specific extensions** (`ofxImageEffectExt.h`, `ofxParamExt.h`):
  - `kOfxImageEffectPropResolvePage`: which page created the instance (Edit, Color or Fusion).
  - `kOfxImageEffectPropNoSpatialAwareness`: the plugin is a per-pixel function. If true, Resolve includes it when **baking a node graph to a LUT**.
  - `kOfxImageClipPropThumbnail`: the render is only for a thumbnail.
  - `kOfxImageEffectPropSrcFilePath`, `kOfxImageEffectPropSrcFrame`: the source media path and frame.
  - `kOfxImageEffectPropEyeToRender`: left or right eye for stereo.
  - `kOfxParamTypeStrChoice`: string-valued choice parameters, which can be animated.
- **OFX colour management:** the `ofx-native-v1.5_aces-v1.3_ocio-v2.3` config defines named colour spaces, so plugins and host can agree on colour space.
- Samples cover temporal access (`getFramesNeeded`) and random frame access.
- Resolve runs OFX plugins through `OFXLoader.exe`, a separate process.

**Takeaway:** **host OFX in Picasso Suite.** It's an open standard with a large existing plugin
ecosystem; hosting it gets you third-party effects for free and keeps your own effects portable.

### 6.4 Codec / IO plugins (`.dvcp`)

- These are CPU-only **encoder and container** plugins that appear in the Deliver page format and codec lists. Studio only.
- Bundle: `NAME.dvcp.bundle/Contents/{Win64|Win-arm64ec|MacOS|Linux-x86-64}/NAME.dvcp`, installed in
  `%ProgramData%\Blackmagic Design\DaVinci Resolve\Support\IOPlugins`.
- **ABI:** the plugin exports `pluginInit(const APIContext* host, APIContext* plugin)`. Each `APIContext`
  holds `{ version, StatusCode (*pHandleMessage)(MessageID, ...) }`, so all calls go through a
  **single variadic message function** in each direction. Message families:
  - object lifetime: create, retain, release (ref-counted, objects addressed by UUID);
  - property collection: set, get, clear;
  - list: append;
  - buffer: resize, lock, unlock;
  - plugin: start, terminate, list codecs, list containers, get info;
  - codec: init, open, flush, set callback, process data, accept frame (PTS), need next pass;
  - container: init, open, add track, close; track: write.
- **Properties are string-keyed typed values**:
  - fourcc, dimensions, strides, field order, crop;
  - bit depth, colour model, subsampling, data range, HDR primaries and MaxCLL/FALL;
  - time base, PTS/DTS, frame rate (drop or non-drop);
  - audio channel layout;
  - multipass, B-frame reordering, magic cookie (avcC/esds), bitrate.
  - The **render-settings UI is also described by properties** (`uiType`, label, min/max/default/step, value lists, a "trigger update" flag), so plugins never touch the host's widget toolkit.

**Design lesson:** a message-passing C ABI with ref-counted opaque objects and string-keyed
properties is easy to version and keeps ABI breakage away. The plugin also describes its own
settings UI as data. This is a good pattern for Picasso Suite's own plugin APIs.

### 6.5 DCTL — DaVinci Color Transform Language

- A C-like **per-pixel shader language** that Resolve compiles at runtime for CUDA, OpenCL or Metal.
  Files are `.dctl`. `.dctle` files are encrypted and carry an expiry date.
- **Entry points** (exact signatures are required):
  - `float3 transform(w, h, x, y, r, g, b)`: pixel-value form, equivalent to a LUT.
  - `float3/float4 transform(..., __TEXTURE__ texR, texG, texB[, texA])`: texture form, which can sample neighbours with `_tex2D`.
  - `float4 transition(w, h, x, y, fromTex×4, toTex×4)` with a global `TRANSITION_PROGRESS` running 0..1.
- **Language features:**
  - Qualifiers `__DEVICE__`, `__CONSTANT__` and `__CONSTANTREF__`.
  - `float2/3/4` vector types, structs and `#include`.
  - Built-ins: `RAND(seed)`, `TIMELINE_FRAME_INDEX`, `DEVICE_IS_CUDA/OPENCL/METAL`, `__RESOLVE_VER_MAJOR__`.
  - LUT helpers: `DEFINE_LUT(name, path)`, an inline `DEFINE_CUBE_LUT`, and `APPLY_LUT`.
  - Alpha mode is declared with tags (straight or premultiplied).
- **Declarative UI** with `DEFINE_UI_PARAMS(var, label, type, …)`:
  - float and int sliders (with step), value box, checkbox, combo (an enum plus labels) and colour picker;
  - up to 64 of each type and 5 colour pickers;
  - tooltips via `DEFINE_UI_TOOLTIP`.
- **Ways to use a DCTL:**
  - as a LUT on a node;
  - through the "DCTL" ResolveFX plugin, which builds the UI from the declarations;
  - through the DCTL transition plugin;
  - as a custom ACES IDT/ODT (`DEFINE_ACES_PARAM` / `DEFINE_ACES_V2_PARAM`, either parametric or hand-written).
- Edits to the file are picked up live by pressing Reset. A build error shows a dialog with the compiler output.

**Takeaway:** a small portable shader DSL, cross-compiled to every GPU backend, gives users
GPU effects without a C++ SDK. Picasso Suite could do the same with a WGSL/GLSL subset, or by
translating to Slang/SPIR-V.

### 6.6 Fusion Fuses (Lua tool plugins)

- Lua scripts with the `.fuse` extension, JIT-compiled and hot-reloaded inside the app, with no build step.
- Installed in `…\Support\Fusion\Fuses` (resolved through Fusion's "Fuses:" path map).
- **Structure:**
  - `FuRegisterClass(id, CT_Tool, {REGS_Name, REGS_Category, …, REG_OpNoMask, REG_NoBlendCtrls…})`.
  - `Create()` declares inputs and outputs with typed links (`LINKID_DataType = "Image" | "Number" | …`) and control types (`INPID_InputControl = "SliderControl"`, …).
  - `Process(req)` pulls values for the request time, calls image operations and sets the outputs.
- **Library:** colour matrices, image channel operations, a shape library, text, pixel sampling, metadata, GPU paths (DCTL kernels and GLSL viewer LUTs) and modifier fuses.

### 6.7 Fusion macros and templates (`.setting`, `.picef`)

- `.setting` files are **Lua-table text serialisations** of a Fusion node group (`MacroOperator` with
  `InstanceInput`/`InstanceOutput` that expose inner tool controls). The format is human-readable and diffable.
- **Template categories** come from the folder they live in under `Fusion/Templates/Edit/`:
  `Transitions/` (2 image inputs, 1 output), `Titles/`, `Generators/` (0 inputs) and `Effects/` (1 in, 1 out).
- **Anim Curves modifier:**
  - It drives animation from the *duration of the edit* (or the transition curve) instead of fixed keyframes, so a template stretches to fit any clip length.
  - It offers easing presets (sine, quad, cubic … elastic, bounce), plus mirror, invert, scale, offset and time scale/offset.
- **Assets and packaging:**
  - The `Setting:` path map resolves assets relative to the .setting file.
  - An icon is a `.png` with the same name (104×58).
  - `.picef` is a zip bundle of templates, icons and assets.

**Takeaway:** "Templates that know their edit duration" is the key idea that makes compositor
content usable as one-click NLE effects.

### 6.8 OGraf HTML graphics (EBU open standard)

- A graphic is a manifest (`.ograf.json`) plus an ES module that exports a custom `HTMLElement`, plus assets.
  The package is a `.ograf` file or part of a `.picef`.
- Resolve renders it inside Fusion's **OGrafLoader** tool using an offscreen browser (CEF on Windows,
  WebKit on Mac). It **renders non-real-time only**: the host calls `goToTime(ms)` for every frame and
  sends the whole action list with `setActionsSchedule()`.
  - Output must be deterministic for a given time: no timers, no unseeded randomness, no CSS animations left playing.
- **Mapping to the Inspector:** the JSON-Schema (GDD) properties become native controls. Colours become colour-managed pickers.
- **Limits:** 20 parameters, 10 custom actions and 20 scheduled actions per clip.
- **Default clip length** is the larger of `v_bmd.duration` and the play+stop action durations; with neither declared it is 10 s.
  An automatic stop action follows the end of the clip.
- Includes a linter (`lint-ograf.py`) and a packager (`prepare-picef.py`).

**Takeaway:** Picasso Suite can **adopt OGraf directly** as its motion-graphics template format. It's
an open spec, and rendering HTML through a deterministic offscreen browser with seek-per-frame is a
proven approach.

### 6.9 LUTs (`.cube`)

- 1D LUTs of up to 65,536 entries and 3D LUTs of size N³ (R varies fastest), with an optional 1D shaper before them.
- `LUT_1D/3D_INPUT_RANGE`, `TITLE`, and `LUT_IN_VIDEO_RANGE` / `LUT_OUT_VIDEO_RANGE` flags.
- Resolve works internally in data (full) range and converts when those flags are set. 3D interpolation is trilinear or tetrahedral, set per project.
- Export sizes are 17/33/65-point cubes and Panasonic VLUT.

### 6.10 Transport control TCP protocol

- Port **9060**, enabled with `System.Remote.Control = 1` in the advanced preferences. No TLS.
- **Wire format:**
  - Commands are bare lowercase ASCII strings followed by a little-endian payload.
  - Strings are sent as an int32 length followed by the bytes.
  - Each reply is a status byte (0x00 OK / 0xFF fail) followed by its payload.
  - If either side stalls for more than 5 s, the other may drop the connection.
- **Commands:**
  - `connect`.
  - `goto` with hh, mm, ss, ff as 4 bytes.
  - `play` with a float: 1 is play, 0 is stop, −1 is reverse, 2 is 2×.
  - `gettc`.
  - `getframerate`.

### 6.11 Other surfaces (manual only, not SDKs)

- **Control panels:** Blackmagic panels, plus Tangent, Euphonix/Avid and JL Cooper. Each has its own
  daemon process.
- **Resolve Live:** on-set grading of a live input. **Remote grading / Remote Monitor**.
- **Blackmagic Cloud:** project server, storage, presentations, Live Sync.
- **VFX Connect:** a round trip to Fusion Studio.
- **Round-trip delivery:** when rendering individual clips for an XML or AAF round trip, some effects
  are baked in and others are passed through as metadata. The manual has a table of which ones.

---

## 7. Suggested mapping for Picasso Suite

| Resolve concept | Suggested Picasso Suite equivalent |
|---|---|
| Pages over one project model | Workspaces/views over one shared, observable document model |
| Project library DB + Live Save | SQLite (local) / Postgres (shared) with an op-log and incremental autosave |
| Bin and clip locking | Per-entity soft locks plus a presence system |
| Stable unique IDs everywhere | UUIDv7 for every entity |
| Flat settings key/value with overrides | Typed settings schema with project → timeline → clip inheritance |
| 32-bit float, resolution-independent DAG | Pull-based render graph in float32, with concatenated transforms |
| Three-layer cache | Content-hash cache at source, node and sequence boundaries |
| RCM / ACES | OpenColorIO 2.x config-driven pipeline |
| OFX hosting | Host OpenFX 1.4/1.5 in an isolated process |
| DCTL | Small shader DSL → SPIR-V/MSL/HLSL (e.g. via Slang) with declarative UI params |
| Fuses / scripting | Embedded Lua or Python plus a reflective object API, also reachable over IPC |
| Workflow Integrations | Web-panel plugin host (WebView2/CEF) with a sandboxed bridge |
| `.setting` / `.picef` templates | Text-serialised node groups plus a zip bundle; duration-relative animation |
| OGraf | Adopt OGraf as is |
| `.cube` LUTs | Support .cube (plus CLF via OCIO) |
| Codec plugin message ABI | C ABI using a message dispatcher, ref-counted handles and property bags |
| Fairlight strip/bus model | Real-time audio graph: strip → bus matrix → monitor section |
| Interchange | OTIO first (open), then FCPXML, AAF and EDL |

## 8. Open, non-Blackmagic libraries you can legally use

Resolve itself relies on these, and they're the obvious building blocks: Qt (LGPL/commercial),
FFmpeg (LGPL), OpenColorIO, OpenImageIO, OpenEXR, OpenVDB, OpenUSD, Alembic, OpenFX, OpenTimelineIO,
TBB, OpenCV, ONNX Runtime, llama.cpp, PortAudio, libsndfile, SoX resampler, Lua/LuaJIT, CEF or
WebView2. Check each library's licence against Picasso Suite's distribution model.

## 9. Source index

| Topic | Location |
|---|---|
| Scripting README / API stub | `Developer\Scripting\README.md`, `DaVinciResolveScript.pyi`, `CHANGELOG.md` |
| Workflow Integrations | `Developer\Workflow Integrations\README.txt`, `Examples\SamplePlugin\` |
| OpenFX | `Developer\OpenFX\README.txt`, `OpenFX-1.4\include\ofxImageEffectExt.h`, `ofxParamExt.h` |
| Codec plugins | `Developer\CodecPlugin\README.txt`, `…\include\IOPluginDefs.h`, `IOPluginProps.h` |
| DCTL | `Developer\DaVinciCTL\README.txt` |
| Fuses | `Developer\Fusion Fuse\Fuse Read Me.txt`, `Fusion Fuse SDK.pdf` |
| Fusion templates | `Developer\Fusion Templates\README.txt` |
| OGraf | `Developer\OGraf HTML Templates\Documentation\01–09` |
| LUT format | `Developer\LUT\README.txt` |
| Order of operations | Manual ch. 144, pp. 3406–3408 |
| Render cache / proxies | Manual ch. 8, pp. 191–218 |
| Colour management | Manual pp. 219–251 |
| Sizing pipeline | Manual pp. 283–297 |
| Fusion float processing | Manual pp. 1640–1650 |
| Fairlight signal flow | Manual ch. 183, pp. 4062–4063 |
| Delivery effects processing | Manual ch. 185, pp. 4089–4093 |
| Project libraries / collaboration | Manual pp. 72–93, 4222–4255 |
| Transport TCP protocol | Manual ch. 203, pp. 4333–4335 |
