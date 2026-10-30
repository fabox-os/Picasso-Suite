# Flathub

`io.github.fabox_os.PicassoSuite.yml` is the manifest Flathub would build: FFmpeg 7.1 and the codec
libraries it uses (x264, x265 with 8/10/12 bits, libvpx, libaom, SVT-AV1, Opus, LAME, NVENC and AMF
headers, VA-API) are built from source, then the program with its Rust crates from
`cargo-sources.json`. The Flatpak in the releases (`../linux/flatpak.yml`) packs the files CI built
instead and is not what Flathub builds.

**Status:** it builds. `.github/workflows/flathub.yml` builds it in CI (run it by hand or push a
`flathub-*` branch) for x86_64 and aarch64 in about 20 minutes and runs Flathub's linter. Nobody has
submitted it: a submission is a pull request to `flathub/flathub` from a GitHub account, which only
the project's owner can make.

What the linter said (read it in the run's annotations):

* **`appid-url-not-reachable`** (an error). For an id `io.github.<user>.<name>` Flathub looks for
  `https://github.com/<user>/<name>`, lower case. Here that is `github.com/fabox-os/picassosuite`,
  which does not exist: the repository is `Picasso-Suite`. The id Flathub would accept for this
  repository is `io.github.fabox_os.Picasso_Suite`, but that would make the Flathub app a different
  app from the Flatpak in the releases (`io.github.fabox_os.PicassoSuite`), so those installs would
  not update to it. Decision: keep the id and ask for an exception, text under "Asking for the id
  exception" below.
* `finish-args-contains-both-x11-and-wayland`: expected, see below.
* `runtime-update-available-to-org.freedesktop.Platform-26.08`: the manifest uses 25.08, which is
  supported; moving to 26.08 means the `llvm22` extension (the bindings are tried with 20).
* `appstream-external-screenshot-url`, `appstream-screenshots-not-mirrored-in-ostree`: Flathub's
  builders mirror the screenshots themselves; a local build always reports these.

## Asking for the id exception

Paste this in the submission pull request when the reviewers raise `appid-url-not-reachable` (or
in a pull request to `flathub-infra/flatpak-builder-lint`, `exceptions.json`, under
`io.github.fabox_os.PicassoSuite`):

> **App id: `io.github.fabox_os.PicassoSuite` (`appid-url-not-reachable`)**
>
> The linter looks for `https://github.com/fabox-os/picassosuite`. The project is
> `https://github.com/fabox-os/Picasso-Suite`: the repository name has a hyphen, so the id derived
> from it would be `io.github.fabox_os.Picasso_Suite`.
>
> I would like to keep `io.github.fabox_os.PicassoSuite`, for two reasons:
>
> * It is the id of the Flatpak bundles already published on the project's releases page (desktop
>   file, icons, mime file, window class and the app's own Check for Updates all use it). A different
>   id on Flathub would be a different application: people who installed the bundle would not be
>   moved to the Flathub one, and would have two copies with separate settings.
> * The id is still in a namespace I control: `io.github.fabox_os.*` is the `fabox-os` GitHub
>   account that owns the repository, so ownership can be verified through the GitHub login as for
>   any `io.github.*` app.
>
> If the id has to match the repository's name for the app to be accepted, tell me and I will rename
> it before the first Flathub release; only the id changes.

If the exception is refused, the choices are the rename (`io.github.fabox_os.Picasso_Suite`, done in
the desktop file, metainfo, icons, mime file, both manifests, the workflows and the updater) or
renaming the GitHub repository to `PicassoSuite`, which makes the current id match (GitHub redirects
the old address; not tried).

## Submitting

1. Make `cargo-sources.json` for the tag you submit (Flathub builds offline):
   ```
   pip install aiohttp PyYAML tomlkit
   curl -O https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py
   python3 flatpak-cargo-generator.py Cargo.lock -o cargo-sources.json
   ```
2. Copy the manifest and `cargo-sources.json` to a new branch of a fork of `flathub/flathub`
   (`io.github.fabox_os.PicassoSuite.yml` in its root) and change the `picasso-suite` module's source
   from `type: dir` to the release tag:
   ```yaml
   - type: git
     url: https://github.com/fabox-os/Picasso-Suite.git
     tag: v0.6.3
     commit: <the tag's full commit id>
   ```
   Open the pull request against the `new-pr` branch, following
   <https://docs.flathub.org/docs/for-app-authors/submission>.
3. The appstream file (`../linux/io.github.fabox_os.PicassoSuite.metainfo.xml`) is read from the
   source tree. Flathub checks its screenshots are reachable; they are the files in
   `docs/screenshots/0.2.0` on the `main` branch. Verified-app status needs the website
   `fabox-os.github.io` or the GitHub login `fabox-os` to prove the id `io.github.fabox_os.*`.

## Things a reviewer will ask about

* **Both `--socket=x11` and `--socket=wayland`.** The linter flags the pair
  (`finish-args-contains-both-x11-and-wayland`). They are needed because winit 0.30, the window
  library, does not take files dropped on a Wayland window, so the program opens through XWayland
  on a Wayland desktop (and a native Wayland window, `PICASSO_WAYLAND=1`, has no drag and drop).
  The exception to ask for, in a pull request to `flathub-infra/flatpak-builder-lint`
  (`exceptions.json`, under `io.github.fabox_os.PicassoSuite`):
  > `finish-args-contains-both-x11-and-wayland`: the editor imports media by dragging files from
  > the file manager onto its window; winit does not support file drops on Wayland yet
  > (rust-windowing/winit#4773), so the window is opened through XWayland. It uses X11 only as the
  > transport for those drops and falls back to Wayland where there is no X11. To be removed
  > when winit supports drops on Wayland.
* **`--filesystem=home`.** Projects and footage are anywhere in the home folder, and many users
  have them on other drives (`/media`, `/run/media`). If the reviewers prefer, `xdg-videos`,
  `xdg-pictures`, `xdg-music` and `xdg-documents` plus the file chooser portal are the narrower
  choice, but then footage elsewhere cannot be opened from the command line or dropped.
* **GPL codecs.** The app is GPL-3.0-or-later, so FFmpeg is built with `--enable-gpl` and x264 and
  x265. Patent-encumbered codecs (H.264, H.265) are built in; whether Flathub accepts that is
  its reviewers' call. If it does not, the same manifest works with `--enable-libx264 --enable-libx265`
  removed (the H.264 and H.265 presets then show as unavailable).
* **No Quick Sync.** `libvpl` is not built, so Intel's Quick Sync encoder is not in this build (it is
  in the release packages). VA-API (Intel, AMD), NVENC and AMF are.
* **Updates.** The build has `/app/share/picasso-suite/updates-by-flathub`: Help > Check for Updates
  then only opens the release page instead of installing a bundle with the host's `flatpak`, and the
  manifest does not ask for `--talk-name=org.freedesktop.Flatpak` (the release Flatpak does).

## Not tried

CI builds it, but nobody has started this build on a real desktop yet: the VA-API, NVENC and AMF
encoders, the 10-bit H.265 export and the window itself are only checked as far as the release
packages are.
