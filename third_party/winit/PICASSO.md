# winit 0.30.13, as changed for Picasso Suite

This is winit 0.30.13 from crates.io (Apache-2.0, see `LICENSE`) with its docs, examples and tests
left out, and **one change, in the X11 drag and drop** (`src/platform_impl/linux/x11/`). Picasso
Suite opens its window through XWayland on a Wayland desktop, because winit 0.30 takes no files
dropped on a native Wayland window; but a drag from the file manager through XWayland could arrive
as nothing at all, with no word of why. The root `Cargo.toml` points `[patch.crates-io]` here.

What is changed (every change is marked "Picasso Suite change" in the code):

- `uri_list.rs` (new): the `text/uri-list` is read line by line. Upstream read all of it or none: one
  line it could not use (a name that is not UTF-8, `file://localhost/...`, a NUL at the end, a
  path that is gone) made the whole drop silent. Now the usable lines are kept and the others are
  logged. It has its own tests.
- `dnd.rs`: `parse_data` uses it; a path that cannot be resolved but exists is kept; `pending_drop`.
- `event_processor.rs`:
  - a drop that comes **before the list of files has arrived** (the compositor reads it from the
    file manager, which takes longer for a large batch) asks for the list again, with the time of the
    drop, and is answered when it comes (`XdndFinished`), instead of dropping nothing;
  - a late second answer for the list is not taken as a new hover;
  - every way the drag could fail is logged as a `tracing` warning (the session log has them):
    the list of types could not be read, the list of files could not be read, the source did not
    give it, no usable line. A refused drag of text is logged with `PICASSO_LOG=debug`.

A second change, in the monitor list (`event_processor.rs`, `window.rs`): when the X server answers
"bad CRTC" (`RandrBadCrtc`, seen on Intel graphics while the monitors are changing) upstream
panicked with `Failed to get monitor list`, which closed the program, even at the start. The four
places that did that now log a warning and carry on: the scale factor is kept, the monitor list is
empty or missing for that call, and it is asked for again the next time.

A drag that worked before takes the same path as before (same events, same order).

Remove this folder and the `[patch.crates-io]` entry when eframe uses a winit that takes drops on
Wayland itself (winit 0.31).
