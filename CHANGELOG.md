# Changelog

## 0.6.3

Picasso Suite 0.6.3 makes **File > Media Management...** cover the whole media pool, and fixes two Linux problems in the window library: files dragged from the file manager on a Wayland desktop that were lost without a word, and a crash when the X server could not list the monitors. Both Linux fixes are checked by tests and by building for Linux, but have not been tried on a Wayland desktop or on the Intel graphics that crashed. The project file format is the same as in 0.6.2.

### ✨ New
- ✅ **Optimised Media covers the whole media pool:** **File > Media Management... > Optimised Media** lists every video of the media pool, used on a timeline or not (a file imported twice once), as Resolve does; **Optimise All** makes a copy of each, **Delete Copies** deletes this project's copies of them (the other projects' copies stay) and **Delete All** deletes every project's. The viewer already read a copy for any video of the pool; only the list and the buttons were limited to what the timelines use. Archive Project still copies only the files the timelines use. Asked by GANESH NIKAM.

### 🐛 Fixed
- ✅ **macOS: the Apple silicon app is built on macOS 15:** GitHub retired the macOS 14 machines it was built on (they refused jobs from October and are gone on 2 November 2026), so it is built on macOS 15 like the Intel app, whose libraries already needed macOS 15; the Apple silicon app may need macOS 15 or newer now too.
- ✅ **Linux, Wayland: files dragged from the file manager are no longer lost silently:** the window opens through XWayland on a Wayland desktop, and winit's X11 drag and drop gave up on the whole drag for one thing it could not read, or dropped nothing when the drop came before the file manager's list of files did (slower for a large batch). The program now uses winit 0.30.13 with that part changed (`third_party/winit`, see `PICASSO.md` there): each file in the list stands alone (a name that is not UTF-8, `file://localhost/`, a NUL at the end, a file that is gone no longer spoil the others), a drop that comes before the list waits for it, and every way a drag can still fail is a warning in the session log (`PICASSO_LOG=debug` for more), with the reason. **File > Media Management...** has a note about it on Wayland. The change is checked by tests of the list reading and by compiling for Linux; it has not been tried on a Wayland desktop. Reported by GANESH NIKAM.
- ✅ **Linux: the program no longer closes when the X server cannot list the monitors:** a `RandrBadCrtc` answer (seen on Intel graphics, even at the start) made winit panic with "Failed to get monitor list"; the four places that did it now log a warning and carry on (`third_party/winit`, see `PICASSO.md`). Reported by GANESH NIKAM.
- ✅ **Linux, Wayland: a note on lost drops:** **File > Media Management...** says on a Wayland desktop that files dragged from the file manager may not arrive, and points at **File > Import** and a session on Xorg.
- ✅ **Tests that make a video or a tone no longer fail at random:** the tests that write a file with ffmpeg wrote the same file in the temp folder and ran at the same time, so one could read it while another rewrote it; each test has a file of its own now.

## 0.6.2

Picasso Suite 0.6.2 is the biggest update since 0.6.0: the program can be shown in 39 languages (**Help > Language**, kept for the next start), videos can have small Optimised Media copies for smooth scrubbing, work is saved by autosave and can be recovered, missing media can be relinked and a project archived, and the updater is safer. It also fixes exports that read back one frame short and menus that did not open on two pages. The project file format is the same as in 0.6.1.

### ✨ New
- ✅ **Auto Align Clips > Based on Waveform:** select two clips or more (a picture and its linked sound count once) and the same sound lands on the same timeline frame: the earliest clip stays and the others are placed from how their sound lines up with its, comparing up to a minute of each clip from the frame it shows (so slipped clips line up by what they show). It works for sound recorded by different microphones at different levels, a camera against a recorder, say. If a clip shares no clear sound with the first, nothing moves and the message says which clip. Reversed, frozen and re-timed clips are refused; it is one undo step.
- ✅ **Autosave and recovery:** while a project has changes that were not saved, a copy of it is written every minute beside the project file (`shot.picasso.autosave`, with the two before it), or in the data folder for a project that was never saved. Opening a project whose copy is newer asks "Recover unsaved changes?"; **Recover** opens it with the project's own file kept as the place it saves to, and **Discard** deletes the copy and leaves the project file alone. A save, or Exit without saving, deletes the copies. A crash or a power cut now costs at most a minute of work.
- ✅ **Relink missing media:** right-click a clip in the Media Pool and choose **Relink Missing Media...** (or use **File > Media Management...**) to choose the folder the files moved to. Files are matched by name, and only when exactly one file under the folder has that name: a name that appears twice is reported, never guessed, because relinking to the wrong take is worse than not relinking. A relink is one undo step, and the new file is what plays.
- ✅ **Archive a project:** **File > Media Management... > Archive Project...** copies the files the timelines use into a folder (`<chosen>/<project>/media/`) and writes a copy of the project there that points at them, so the folder can be moved, handed over or backed up as one thing. The project you are editing is not changed. Files that are missing are reported and the rest are still archived.
- ✅ **Help > Language:** the menus and the pages can be shown in Español, Português (de Portugal), Français, Deutsch, Italiano, Nederlands, Polski, Türkçe, Svenska, Norsk, Русский, Українська, Ελληνικά, Dansk, Suomi, Bahasa Indonesia, Čeština, Slovenčina, Română, Magyar, Hrvatski, Bosanski, Српски, Català, Eesti, Latviešu, Lietuvių, Íslenska, Lëtzebuergesch, Shqip, Filipino, Tiếng Việt, 한국어, বাংলা, ไทย, 中文, 日本語, العربية and हिन्दी; English (Default) is the one chosen until another is. The choice is kept in `preferences.txt` and is back at the next start. Every language has every menu entry and the text the pages show (about 190 pieces of text each: the Media, Cut, Edit, Fusion, Color, Fairlight and Deliver pages, the Render Settings tabs, the Inspector and Mixer headings). A translation never makes the text wider than the English: a word that is too long for its place is a short form (`Arch.` for File in Spanish, `Export. rápida` for Quick Export) that is still clear, and text is never drawn smaller than the text around it. The update window and the frame-rate question ("Change Project Frame Rate?") are translated too, sentences with a version or a frame rate in them included (`{}` in the tables marks the piece). A text with no translation yet (the other dialogs, a tooltip, a status message) shows in English. Chinese, Japanese and Arabic come with fonts of their own (Noto Sans SC, JP and Arabic, cut down to the characters those languages need, about 3 MB together); Japanese uses half-width katakana (`ﾌｧｲﾙ`) where the full-width word would not fit, and Arabic is joined and set in reading order when the table is made, because the text drawing sets letters one by one from the left (the layout of the pages is not mirrored, and a sentence that wraps over several lines in Arabic reads in the wrong line order). Hindi, Bengali and Thai are shaped at build time too (conjuncts, vowel signs and marks are set by HarfBuzz and each word becomes one glyph of a font made for it, `scripts/i18n/devanagari.py`); Korean uses its own font (the 2,350 common syllables and the ones the tables use). `scripts/i18n/nums.py`, `derive.py` and `addlang.py` make adding a language a matter of one numbered file.
- ✅ **Optimised Media:** small H.264 copies of the video files a timeline uses (at most 1280 pixels on the long side, a key frame every 8 pictures, no B-frames), made once and kept in the data folder for every project, so scrubbing and playing 4K and long-GOP clips is smooth. **Playback > Use Optimised Media** turns them on and off (the choice is kept; an OPTIMISED badge shows in the viewer while the copies are read), and **File > Media Management... > Optimised Media** lists the files, makes the copies (**Optimise All**) and deletes them. A copy is used only while its file has the same size and date; exports read the originals unless **Use proxy media** is ticked on the Deliver page. Thanks to GANESH NIKAM.
- ✅ **Update window and frame-rate question translated:** the update window and "Change Project Frame Rate?" (with sentences that hold a version or a frame rate) are in all 39 languages.

### 🐛 Fixed
- ✅ **Optimised Media: Cancel, and variable frame rate:** **File > Media Management... > Optimised Media** has a **Cancel** button that stops the copy being made (its partial file is removed) and forgets the ones waiting; the copies already made stay. A file whose frame rate is not fixed (a phone or screen recording) is no longer given a constant-rate copy, whose pictures could be a frame off: it is read as it is and listed as "Read as it is".
- ✅ **Edit page Inspector scroll:** the Inspector's scroll area had the same id as another list on the page, so dragging its bar moved the other list and the mouse wheel did not scroll the Inspector. It has its own id now, and a test checks that no page draws two widgets with one id. Reported by a tester on CachyOS.
- ✅ **Edit page Effects button:** it no longer jumps to the Fusion page; it opens the Inspector, where a clip's effects are added and ordered.
- ✅ **Exported files no longer read back one frame short:** the encoder gave the last video packet no duration, so 24 frames at 24 fps had a length of 23/24 s; every packet now carries one frame. Thanks to GANESH NIKAM.
- ✅ **Recovering a copy that cannot be read:** Autosave no longer says it recovered a copy that is not a project (the project was marked changed with changes that never existed); it names the file and points at Discard. Thanks to GANESH NIKAM.
- ✅ **Optimised media, made safer:** the index of copies can only name files inside the copies folder (an edited index could make Delete Copies remove another file), two paths that hash alike never share a copy (the hash is 64 bits and a taken name is not reused), a file name with a tab or a line break keeps no copy, and copies that were never finished (the program closed while one was being made) are swept at the next start when they are an hour old.
- ✅ **Updater hardened:** the update is downloaded over HTTPS only (redirects included), a package name from GitHub must be a plain file name, and the folder it is downloaded to has a name nobody can guess and is private to the user (a `.deb` or `.rpm` is installed with `pkexec` after the download, so another user could have swapped the file in a shared `/tmp`).
- ✅ **Help menu:** the first menus (File to Timeline, or all of them on the Media page) did not open on the Media and Deliver pages, in any language; the menu row is now above the pages' layers. A long submenu such as Help > Language is a short window that scrolls.

## 0.6.1

Picasso Suite 0.6.1 plays smoother on Windows and in 4K and at 60 fps, adds Auto Align Clips by timecode and the six Hue/Sat/Lum curves on the Color page, and fixes what the pull requests since 0.6.0 left open. Much of it is GANESH NIKAM's work. Packages for Windows, macOS and Linux (x86_64 and ARM64).

### ✨ New
- ✅ **Audio > Auto Align Clips > Based on Timecode:** select two or more clips and choose *Audio > Auto Align Clips > Based on Timecode*. The clips move so that the same source timecode falls on the same timeline frame: the clip with the earliest timecode stays where it is, and the others line up with it. A clip's first shown frame counts from the file's start timecode plus its source in, so slipped clips still line up. The start timecode is read from the video stream's metadata (the `tmcd` track of a QuickTime file) or the file's metadata, kept on the media item, and counted the way drop-frame files do at every tenth minute. Thanks to GANESH NIKAM for the pull request.
- ✅ **Color page hue/sat/lum curves:** the six Hue vs Hue, Hue vs Sat, Hue vs Lum, Lum vs Sat, Sat vs Sat and Sat vs Lum curve buttons in the Curves panel now work. Drag on the graph to bend hue, saturation or luminance by how far the curve is from straight; the histogram behind it keys the same value, and the curves are saved per node.
- ✅ **New Effects Clip:** a transparent clip whose color grade and effects apply to the composite of all tracks below it, like DaVinci Resolve's Effects Clip. Add one from the Timeline menu (`Alt+Shift+E`) or the Clip toolbar; it lands on a free video track above the clips at the playhead. Drag on its grade in the Color page or add effects in the Inspector to process everything underneath.
- ✅ **Exit Picasso Suite?:** closing the window (the close button, File > Quit, Alt+F4 or the taskbar) with changes that were not saved asks "Do you really want to exit, without saving?" in the same box as "Change Project Frame Rate?". **Exit without saving** closes the program; the red **Save Project** saves as File > Save Project does (to the project's file, or after asking where on the computer to put it) and closes once the project is written; if the save is cancelled or fails the program stays open, and Escape leaves the box. A project with nothing to save closes at once.

### 🛠️ Fixed
- ✅ **Smoother playback at 60 fps and in 4K, above all on Windows:** the decoder copied each whole decoded picture (12 MB at 4K) twice, once to keep it and again on every request, and wrote every converted picture into a new allocation; it now keeps the picture it decoded, converts into one reused buffer, and a picture asked for again (24 fps footage in a 60 fps project asks for each picture two or three times) is not converted again. On a Ryzen 9 5950X, 4K 60 fps pictures take 7 ms instead of 15 ms, and 1080p 24 fps footage in a 60 fps project half as long.
- ✅ **The graphics card is used only where it is faster:** copying pictures off the card is slow with Direct3D 11 (10-bit 4K HEVC took 21 ms a picture on an RX 7800 XT against 9 ms on the processor, and jumping to another place took three times as long). While a file plays, the first pictures are timed on the card, and when they take more than 6 ms the processor is tried and the faster way is kept for that file for the rest of the run. System Preferences > Decode Options still turns the card off or on.
- ✅ **Audio > Auto Align Clips > Based on Timecode, drop-frame:** the drop-frame rule counts two (or four at 60 fps) skipped frame numbers every minute but every tenth, the way broadcast files do, so aligning clips keeps their frames.
- ✅ **Color page scopes no longer block the interface:** the histogram, waveform, vectorscope and parade are measured on a background thread, so the 6 ms a frame of pixel reads no longer stalls clicks, drags and scrolling.
- ✅ **Color page scopes and curve histogram:** a scope or the histogram behind the curves could stay empty or out of date, because each of the two threw away the other's finished measurement; they are kept now, and the page redraws as soon as one is ready.
- ✅ **Hue vs Lum, Hue vs Sat and Hue vs Hue leave greys alone:** a grey pixel has no hue and was treated as red, so a bump at red also changed greys, white and black. Hue curves now count for less the less colour a pixel has.
- ✅ **Hue/sat/lum curves are about twice as fast:** they are sampled once for a picture instead of being worked out again for every pixel.
- ✅ **Auto align by timecode, other frame rates:** a start timecode counted at the file's own frame rate (25 fps footage in a 24 fps project) is turned into the project's rate on import instead of being refused or read wrongly.
- ✅ **Auto align by timecode, bad files:** a timecode with an absurd hour count is refused instead of overflowing.
- ✅ **Auto align by timecode, linked sound:** a clip's linked sound moves with it, so lining pictures up by timecode no longer leaves their sound behind. A reversed clip lines up by the frame it shows first.
- ✅ **New Effects Clip, one shortcut and a clearer export note:** `Alt+Shift+E` was given to two menu entries (Timeline and Clip); it is on the Timeline entry only. Exporting XML or AAF says that an effects clip's grade and effects are not carried, as no other editor is given them.

### 🔒 Security
- ✅ **Checked, nothing found to fix:** media paths still refuse `http:`, `concat:`, `subfile` and network paths in every decode and audio open, the updater downloads only from this project's GitHub releases, the render queue's script setting is not saved in project files, and Resolve zips, projects and images are read within size and count limits. The one gap found, a timecode with an absurd hour count, is closed (see Fixed).

### 🚧 In progress
- 🚧 **Tested in other editors:** Media Composer, Pro Tools, Premiere and Final Cut Pro 7 themselves still need people to try the AAF and XML files; send a report if one refuses a file.
- 🚧 **Intel VA-API decoding on Linux:** waiting for confirmation on the Intel machine that reported it (issue [#21](https://github.com/fabox-os/Picasso-Suite/issues/21)).
- 🚧 **Flatpak Check for Updates test:** the release container has no `flatpak-session-helper`; it stays non-blocking until a release has passed it.
- 🚧 **Playback on other computers:** the 0.6.1 timings are from a Ryzen 9 5950X with a Radeon RX 7800 XT; reports from other Windows computers, above all with NVIDIA or Intel cards, are welcome.

### ⚠️ Known limitations
- ❌ **Auto align by waveform** is not made yet; only timecode is.
- ❌ **A file switching from the graphics card to the processor** stutters once, for 0.1 to 0.3 seconds, the first time it plays in a run.
- ❌ **An effects clip's grade and effects are not carried** to XML or AAF (the export says so); OpenTimelineIO keeps the clip.
- ❌ **Exit Picasso Suite?** is asked only when there are unsaved changes; a project with nothing to save closes at once.
- ❌ AAF speed changes and markers, nested sequences from other editors, the IMF and Pro Tools presets, a signed macOS app and the newer Flatpak runtime are still to do (see 0.6.0).

### 🙏 Thanks
- Thanks to GANESH NIKAM ([@GMNEdits](https://github.com/GMNEdits)) for:
  - ✅ Audio > Auto Align Clips > Based on Timecode ([#27](https://github.com/fabox-os/Picasso-Suite/pull/27))
  - ✅ The six Hue/Sat/Lum curves on the Color page, with the clippy cleanups and the Color page scopes measured on a background thread ([#28](https://github.com/fabox-os/Picasso-Suite/pull/28))
  - ✅ New Effects Clip, and auto align taking a clip's linked sound with it ([#29](https://github.com/fabox-os/Picasso-Suite/pull/29))

## 0.6.0

Picasso Suite 0.6.0 brings timelines in from other editors as well as out: Final Cut Pro 7 / Premiere / Resolve XML and Avid AAF files open on a new timeline, with their clips matched to the media. Paste Mix to Current Track, audio scrubbing and Bounce Selected Tracks work, the Edit page's track buttons take clicks and its tracks scroll, and the graphics check says whether video decodes on the graphics card. Much of it is GANESH NIKAM's work (pull requests #19 to #26). Packages for Windows, macOS and Linux (x86_64 and ARM64).

### ✨ New
- ✅ **XML and AAF import:** File > Import > Timeline (or dropping the file on the window) reads a Final Cut Pro 7 / Premiere XML sequence (Resolve exports the same) or an AAF composition (Avid Media Composer, Pro Tools, Premiere, Resolve) onto a new timeline, as one undo, the way OpenTimelineIO files are: clips are matched to the media pool by path, then by name, files not in the pool are imported, and files not on this computer are kept offline. XML brings gaps, dissolves, speed changes, reverse, freeze frames, disabled clips, turned off tracks and markers; AAF brings each clip through its master clip to its file's address, fillers and dissolves on their cut points. A stereo pair is one sound clip, and other frame rates are converted. An `.xml` file that is not a sequence is still read as a Resolve XML export, for its media.
- ✅ **Edit > Paste Mix to Current Track:** pastes only the sound of the copied clips onto the current sound track (the one the selected clip is on, or the first unlocked one), the first clip at the playhead and the others at their offsets, as one undo. A video brings its sound and not its picture; a video whose sound is a linked clip of its own brings that clip only, so it is not heard twice. Thanks to GANESH NIKAM.
- ✅ **Clip > Enable / Disable Audio Scrubbing:** with it on, dragging the playhead by a ruler (Edit, both Cut timelines, Fairlight) plays a tenth of a second of the mix at each new place, through the same mix as playback (volume, mute, solo, levels). It is on by default, kept in the preferences, and also in System Preferences > Playback. A scrub is heard but not counted by the loudness meter. Thanks to GANESH NIKAM.
- ✅ **Audio > Bounce Selected Tracks:** bounces only the tracks the selected clips are on (a video track's sound included) to a new sound track, like Bounce Mix to Track; the other tracks and any solo are left out of it. Thanks to GANESH NIKAM.
- ✅ **Edit timeline, track scrolling:** tracks keep their size instead of being squeezed until they cannot be read; when they do not all fit, the mouse wheel scrolls them (Shift + wheel pans) and a bar at the right edge shows where they are. When they fit, the wheel pans the timeline as before. Thanks to GANESH NIKAM.
- ✅ **Graphics check:** `picasso-suite --graphics-check` also says whether video decodes on the graphics card (D3D11, VA-API, CUDA or VideoToolbox), and when it does not, why: on Linux, a card whose VA-API driver is not installed is named, with the package to install.

### 🛠️ Fixed
- ✅ **Imported timelines heard twice:** a video and its sound imported from OpenTimelineIO came in unlinked, so the picture played its own sound as well as the sound clip; a picture and the sound clips of the same file at the same place are now linked when a timeline is imported (from XML and AAF too), as the timeline links them.
- ✅ **Edit page track headers:** the lock, mute and solo buttons never took a click (the column's right-click area was on top of them); they work now, solo lights amber, and a right click anywhere in the header still opens the Add / Delete Track menu. The names and buttons are bigger, so they can be read and hit in a small window. Thanks to GANESH NIKAM.
- ✅ **No false graphics card errors at start (Linux):** the program no longer prints "Cannot load libcuda.so.1" and "Failed to initialise VAAPI connection" when it looks for a decoder the computer does not have: CUDA is only tried with NVIDIA's driver, and the attempts are logged at debug level instead (`PICASSO_LOG=debug`). Thanks to GANESH NIKAM.
- ✅ **Intel graphics on Linux:** the .deb recommends `va-driver-all` (every VA-API driver, Intel's included) as well as `mesa-va-drivers`, which covers AMD but not Intel, and the .rpm recommends Intel's driver under Fedora's and openSUSE's names, so a new install decodes on an Intel card too. An install that has no VA-API driver for its card decodes on the processor; the graphics check now says so.
- ✅ **Color page:** the mini timeline's ruler no longer shows the same time ten times on an empty or very short timeline. Thanks to GANESH NIKAM.
- ✅ **Fixes to the above before release:** the scroll bar could panic in a very short panel, and showed up with nothing to scroll when the tracks fit but started low; the scrub's sound was counted by the loudness meter the next time the timeline played; a ruler drag let go over another panel could keep scrubbing; the decoder check could leave FFmpeg's messages silenced for the whole run; and the bounce's message says again to mute the tracks it came from.

### 🔄 Changed
- ✅ **Exports checked by other programs:** CI opens the AAF this program writes with pyaaf2 (the library OpenTimelineIO's AAF adapter uses), which follows every clip to its file, and reads the XML with Python's XML parser, checking it the way Final Cut Pro 7 and Premiere read it (`scripts/check_interchange.py`). The AAF reader is tested on an Avid-style edit written by pyaaf2 (`crates/picasso-aaf/tests/data`).
- 🚧 **Flatpak update test:** the release workflow's Check for Updates test inside the Flatpak failed because the build container's session bus had no service for org.freedesktop.Flatpak; a service was added for 0.6.0, and the test still failed there: the container has no `flatpak-session-helper` to start. It stays non-blocking until a release has passed it.
- ✅ **Development:** the workflows use the Node.js 24 versions of GitHub's checkout, cache and artifact actions, which removes the Node.js 20 warnings. Thanks to GANESH NIKAM for the first part.

### 🚧 In progress
- 🚧 **Tested in other editors:** every AAF and XML export is now opened by other programs in CI (pyaaf2 and an XML parser), and the AAF reader reads an Avid-style file written by pyaaf2; Media Composer, Pro Tools, Premiere and Final Cut Pro 7 themselves still need people to try them. Please send a report if one refuses a file.
- 🚧 **Intel video decoding on Linux:** the packages now bring Intel's VA-API driver; waiting for confirmation on the Intel machine that reported it (issue #21).
- 🚧 **Flatpak Check for Updates test:** failed again in the 0.6.0 release build (the container has no `flatpak-session-helper`); it stays non-blocking until a release has passed it.

### ⚠️ Known limitations
- ❌ AAF: speed changes, reverse and freeze are written at normal speed and read without them; markers in an AAF file are not read; big-endian AAF files (some old Macs) are not read.
- ❌ A nested sequence (a compound clip from another editor) is left out of OpenTimelineIO, XML and AAF imports, keeping its time as a gap; the app says how many.
- ❌ The IMF Generic and Pro Tools presets are still greyed out (they need an MXF writer and AAF with the sound in it).
- ❌ The macOS app is not signed or notarised (right-click > Open the first time).
- ❌ The Flatpak is built on the freedesktop 24.08 runtime, which has reached its end of life; a newer one is next.

### 🙏 Thanks
- **GANESH NIKAM (@GMNEdits)** for most of this release:
  - ✅ Paste Mix to Current Track (#20)
  - ✅ Audio scrubbing (#22)
  - ✅ Bounce Selected Tracks (#23)
  - ✅ Clickable track header buttons and track scrolling on the Edit page (#24)
  - ✅ No false graphics card errors at start (#25)
  - ✅ The Node.js 24 GitHub actions (#26)
  - ✅ The Color page ruler fix (#19)
  - ✅ The Intel UHD graphics report (#21), which led to the graphics check's video decoding line and the Intel VA-API driver in the packages

## 0.5.0

Picasso Suite 0.5.0 opens the timeline to other editors: it writes Final Cut Pro 7 / Premiere XML and AAF for Avid Media Composer and Pro Tools, and reads OpenTimelineIO timelines from Resolve, Premiere, Avid and others. The Deliver page's Final Cut Pro 7, Premiere XML and AVID AAF presets work, timelines and subtitles dropped on the window open, and Help > Export Log File saves a log of the session to send with a bug report. Packages for Windows, macOS and Linux (x86_64 and ARM64).

### ✨ New
- ✅ **File > Export > Timeline: XML:** the timeline as an `xmeml` (version 5) sequence that Final Cut Pro 7, Premiere Pro and DaVinci Resolve import: every track, each clip pointing at its file, gaps, cross dissolves and dips (centred on the cut), speed changes, reverse and freeze frames (as Time Remap), disabled clips, muted tracks and markers. The Deliver page's **Final Cut Pro 7** and **Premiere XML** presets are no longer greyed out: they write the same file, pointing at the original media (nothing is rendered).
- ✅ **File > Export > Timeline: AAF:** the timeline as an AAF file (AAF Edit Protocol) for Avid Media Composer, Pro Tools and others: a composition with a timecode track starting at 01:00:00:00, V and A tracks of clips, fillers and dissolves, and for each file a master clip and a file source with the file's address, so the editor links to the media where it is. A file keeps the same master clip from one export to the next. The **AVID AAF** preset on the Deliver page writes it too.
- ✅ **OpenTimelineIO import:** File > Import > Timeline (or dropping an `.otio` file on the window) puts the timeline on a new timeline of the project, as one undo. Clips are matched to the media pool by file path, then by name; files that are not in the pool are imported first (read in the background), and files that are not on this computer are added as offline media so the clips keep their place. Gaps, dissolves, speed changes, reverse, freeze frames, disabled clips, muted tracks and markers come across, and times in other frame rates are converted. Solid colours, bars, gradients and text from generators are made here.
- ✅ **Help > Export Log File...:** saves a text file of what happened since Picasso Suite started, to attach to a bug report: the version, the system, the graphics card and driver, what the status line said (projects opened and saved, imports, renders, every error) and the warnings and errors of the program and of the graphics libraries. The log is kept in memory (the last 5000 lines), so nothing is written to disk while the program runs, and nothing is sent anywhere; it names the files that were opened. `PICASSO_LOG=info` (or `debug`) keeps more.
- ✅ **Crash reports:** if the program crashes, the log and the crash (with where it happened) are written to `crash-<time>.txt` in the data folder's `logs` folder (`%APPDATA%\Picasso Suite\logs` on Windows); the last 5 are kept, and the next Export Log File adds the latest one to the file it saves.

### 🛠️ Fixed
- ✅ **Dropped timelines and subtitles:** an `.edl`, `.otio`, `.srt` or `.vtt` file dropped on the window opens like the same file chosen in File > Import; it was read as a picture before and refused.

### 🔄 Changed
- ✅ **macOS universal app:** `picasso-suite-0.5.0-macos-universal.zip` runs natively on Apple silicon and Intel Macs (Intel needs macOS 15 or newer); it is built on both kinds of Mac and joined, and checked on both. The Apple silicon zip is still there. The 0.3.0 and 0.4.0 releases get a universal app too, built from their own code. Check for Updates on an Intel Mac takes the universal one.
- ✅ **File > Export:** the planned "Timeline: AAF, XML..." entry is now two working entries, **Timeline: XML...** and **Timeline: AAF...**; the Open dialog lists OpenTimelineIO files.
- ✅ **Development:** a new `picasso-aaf` crate writes AAF (an AAF file is a Microsoft compound file of objects and properties, through the MIT-licensed `cfb` crate), written from the AMWA's published AAF Object Specification and Low-Level Container Specification; `picasso-core` gains the XML writer (`xmeml`), the OTIO reader (`otio::read_otio`) and an `ImportTimeline` command that builds an imported timeline as one edit, leaving out clips that do not fit.

### ⚠️ Known limitations
- ❌ The AAF file has been read back with pyaaf2 (every clip resolves to its master clip, file source and address) but not yet opened in Media Composer, Pro Tools or Premiere; please send a report if one of them refuses it.
- ❌ AAF: speed changes, reverse and freeze are written at normal speed. Generated media, titles and compound clips have no file and are left out of the XML and AAF (the app says how many).
- ❌ OTIO import: a nested stack (a compound clip from another editor) is left out, keeping its time as a gap. XML and AAF files are not imported yet.
- ❌ The IMF Generic and Pro Tools presets are still greyed out (they need an MXF writer and AAF with the sound in it).
- ❌ Flatpak: Check for Updates installs the new version only where the Flatpak may use the host's `flatpak`; elsewhere it shows the command to run. The 0.5.0 Flatpaks are in the release again (0.4.0 shipped without them, as this check stopped their upload).

## 0.4.0

Picasso Suite 0.4.0 makes playback smoother and steadier: the viewer's picture is put together on a thread of its own, scrubbing camera video is faster, the timelines move at the right speed at every frame rate, and the Edit page's source viewer no longer blinks. It adds saved layouts, working track buttons and height arrows on the Cut page, a mini timeline on the Color page and a Hardware Decoding switch in the Playback menu, with security fixes for project and Resolve files. Packages for Windows, macOS and Linux (x86_64 and ARM64).

### ✨ New
- ✅ **Playback > Hardware Decoding:** turns decoding on the graphics card off (or on) straight from the Playback menu, the same switch as System Preferences > Decode Options. Pictures already made with the card are forgotten, so the change shows at once. If playback shakes, blinks or goes black on your computer, try it off and tell us your graphics card.
- ✅ **Smoother playback:** the viewer's picture is put together on a thread of its own, so the window no longer waits for it. Measured with a 1080p clip playing, the interface's work per frame went from 3.0 to 1.35 ms on the Edit page, 2.2 to 0.66 ms on Cut and 3.9 to 2.3 ms on Color on average; the medians are now 0.6 to 0.9 ms, from 2.2 to 2.6 ms.
- ✅ **Faster scrubbing on long-GOP video** (camera H.264/H.265): a picture being decoded is dropped as soon as the playhead moves on, and the key frame it starts from is shown while the rest is decoded and kept for the next pass. Dragging across 4K video with a key frame every 2 seconds, the exact picture arrives about 200 ms after letting go instead of 370 ms (worst case 270 ms instead of 570 ms).
- ✅ **Cut page:** the height arrows beside the tracks make the picture rows taller or shorter; the height is kept with the panel sizes, in saved layouts too, and Reset UI Layout puts it back.
- ✅ **Color page:** the mini timeline under the clips shows each clip where it sits on up to three picture tracks, with the clip being graded outlined, instead of one solid bar.
- ✅ **Development:** `--play` starts playback once the files given with `--place` are on the timeline (answering the frame-rate question), so `PICASSO_PERF=1` can time playback.
- ✅ **Saved layouts:** Workspace › Layout › Save Layout keeps the sizes of the panels on every page under a name (`layouts/<name>.txt` in the data folder, readable and editable by hand); Load Layout puts one back. Thanks to GANESH NIKAM for the pull request.
- ✅ **Cut page track buttons:** the button row beside the tracks now works. Lock, mute, **solo** and **picture** act on the track of the selected clip (V1 when nothing is selected), undo, and are saved with the project. Solo silences every track that is not soloed, in playback, the mixdown and compound clips; picture off keeps a video track out of the program, the Color and Deliver previews and the subtitles. Older projects open with every track unsoloed and its picture on. Thanks to GANESH NIKAM.

### 🛠️ Fixed
- ✅ **Viewer:** a picture still being put together when a newer one was shown from the render cache no longer replaces it when it arrives (playing a stretch a second time could jump back a frame). It is still kept for later.
- ✅ **DaVinci Resolve files:** one still being read when another project is opened no longer replaces that project when it arrives.
- ✅ **Playback with many stills:** the still images are shared with the viewer's thread instead of copied for every picture it puts together.
- ✅ **Memory:** the key frames kept for scrubbing take at most 32 MB per video (96 before), so a multicam of nine angles stays under 300 MB for them.
- ✅ **Edit page source viewer:** it no longer blinks black while the timeline plays the same clip, and playback no longer stutters for it. Both viewers asked one decoder for the clip, which then sought back and forth between their two moments, and the source viewer drew itself again on every frame. It now has a decoder of its own and draws again only when its own picture changes (Edit page with a 1080p clip: 1.04 to 0.49 ms of work a frame, the slowest frames 3.3 to 0.7 ms). Confirmed on the computer where the choppy, blinking playback was seen.
- ✅ **Long clip names:** the source viewer's header cuts a long name with “…” where it would run over the length beside it.
- ✅ **Timelines at 60 fps:** a second was two and a half times wider in a 60 fps project than in a 24 fps one (600 points on the Cut page's lower timeline instead of 240), so the playhead raced across while the picture and the timecode played at the right speed. The timelines now keep the width of a second at every frame rate.
- ✅ **Cut page:** the lower timeline keeps the playhead in the middle while playing, even after a drag of the playhead whose end it missed (it used to stay still and let the playhead run across).
- ✅ **Long clips:** the film strip and the waveform are drawn only where they are on screen; a 12-minute clip queued over a thousand film-strip tiles and spread its waveform along its whole length on every frame. The waveform is also sharper: one line per point.
- ✅ **Check for Updates on openSUSE:** the new .rpm is installed with `--allow-unsigned-rpm`; zypper refused the release's unsigned package before.
- ✅ **Check for Updates as root:** the .deb or .rpm is installed directly, without `pkexec`, which asked for a password that root does not need (and fails where there is no polkit).
- ✅ **Security:** a project file (`.picasso`) that takes more than 5 seconds to read is refused, so a file built to run a query for ever cannot freeze the app; at most 100,000 media entries are read.
- ✅ **DaVinci Resolve files** (`.drp`, `.drt`, `.drb`, XML) are read on a thread of their own, and the media they list is imported in the background, so a big project no longer freezes the window while it opens.
- ✅ **Timecode:** the readouts on the Cut (TCG), Color and Deliver pages started at 00:00:00:00 while the rulers and the Edit page start at 01:00:00:00; they now agree. Thanks to GANESH NIKAM.
- ✅ **Photo tab:** the second page tab said "Media" like the first; it now says **Photo**. `--page=photo`, `--page=fusion` and `--page=fairlight` open their pages (they were ignored); the old names still work. Thanks to GANESH NIKAM.
- ✅ **Reset UI Layout** now puts the panel dividers back too, not only the floating windows.
- ✅ A damaged layout file is refused with a message instead of loading as the default sizes.
- ✅ A layout is found by its file, so a file renamed by hand still loads; names that make the same file name (`My Layout`, `my-layout`) no longer replace each other; names like `con` or `nul` are not written to Windows devices.
- ✅ The Save and Load Layout windows read the layouts once when they open, not on every frame, and Enter with no name no longer closes Save Layout.

### 🔄 Changed
- ✅ **Releases:** the release workflow now tries Check for Updates where each package really runs (its first run is still to come): the .deb (Ubuntu 22.04 and 24.04, Debian 12) and .rpm (Fedora, openSUSE) are updated to a copy marked newer and the package manager must have it, the macOS app and the Windows folder are replaced with the old copy set aside, and the Flatpak is reinstalled through the host's flatpak, on x86_64 and ARM64.
- ✅ **Development:** `picasso-suite --update-from <package>` installs a release package over this copy with the updater's own code (the download is the only step left out), and `picasso-suite --version` prints the version and, for release builds, the commit. `PICASSO_PERF=1` also gives the time between frames and how many took over 50 ms (`PICASSO_PERF_FRAMES` sets how many frames are timed).
- ✅ **Development:** CI fails when `docs/status.svg` or the README's Status numbers no longer match `Menu-Status.md` and `ROADMAP.md` (run `python3 scripts/status.py`). Thanks to GANESH NIKAM.

### ⚠️ Known limitations
- ❌ Scrubbing video with very few key frames (one every 20 seconds of 4K) still takes seconds for the exact picture; that needs optimised media (proxies).
- ❌ Check for Updates is tested by the release workflow on every package from this release on; before, it was only tried on Windows.
- ❌ Not tried on real NVIDIA, Intel, Apple or ARM graphics cards, nor on a Raspberry Pi; the Color page's scopes still measure on the interface thread.
- ❌ Native Wayland windows take no dropped files (winit 0.30), and the Flathub submission is still to be made.

## 0.3.0

Picasso Suite 0.3.0 adds Linux on ARM64 (.deb, .rpm, AppImage and Flatpak), an AppImage for
x86_64, panels you can resize on every page, a graphics check, and a faster window when
switching pages and importing media. Packages for Windows, macOS and Linux (x86_64 and ARM64).

**✨ New**
- ✅ **Linux on ARM64:** .deb, .rpm and AppImage for 64-bit ARM computers (Raspberry Pi 5, ARM laptops and servers)
- ✅ **Linux Flatpak on ARM64:** `picasso-suite-X-aarch64.flatpak` beside the x86_64 one; Help > Check for Updates installs the one for the computer's processor
- ✅ **Graphics check:** `picasso-suite --graphics-check` lists the graphics cards and interfaces (Vulkan, DirectX 12, Metal, OpenGL), tells which ones draw a test picture and saves the report as `picasso-graphics-report.txt`; there is a "Graphics report" issue form for sending it in
- ✅ **AppImage:** one file that runs on most Linux distributions without installing; Help > Check for Updates replaces it with the new version
- ✅ **Cut page:** files dragged over the window show a drop target on the media pool, and 3GP, 3G2, F4V, VOB, DV, ASF and M2TS video can be imported
- ✅ **Resizable panels:** on every page, drag the dividers between the panels to resize them (Edit: bins, media pool, viewers, timeline; Cut: pool, viewer, timelines; Media: files, queue, viewer, storage, meters, upper and lower rows; Fusion: viewers, node area, inspector; Color: gallery, viewer, nodes, clips, scopes; Fairlight: meters, video, console, track headers, mixer; Deliver: settings, viewer, queue, timeline; Photo: pool, viewer, inspector, album). The divider lights up under the pointer, a double click puts it back, and the sizes are kept for the next start. Panels keep at least two fifths of their usual size; the text, icons and buttons in a narrowed panel get smaller with it instead of running into the next one (toolbars, track headers, transport, empty-panel messages), and the Deliver settings form scrolls sideways in a narrow column. The color wheels keep their size, so the palettes are not resizable
- ✅ **Cut page:** the empty timeline before a track's first clip is grey, as in Resolve, so a clip that starts late is easy to see

**🛠️ Fixed**
- ✅ **Linux:** dragging video files from the file manager into the media pool works on Wayland desktops (Ubuntu's default); the window opens through XWayland, and `PICASSO_WAYLAND=1` keeps a native Wayland window
- ✅ **Linux:** the dock shows Picasso Suite's icon and groups the window with its launcher
- ✅ **Linux:** a per-user Flatpak installs its update per-user, and the command shown when that fails says `--user`
- ✅ **Linux:** programs started from Picasso Suite (the browser, the installer) still see the Wayland desktop; the window asks for X11 itself instead of hiding the Wayland display from everything
- ✅ **Check for Updates:** a download that breaks off leaves no half-file behind
- ✅ **Media pool:** video thumbnails are made without holding up the window while the first frame decodes, by their own decoder, so the viewer no longer leaves them (and the clips on the timeline) black; they are decoded at a quarter of the size and the decoder is closed once the picture is made, so a pool of 4K clips does not keep a full picture and a thread for each
- ✅ **Speed:** importing videos and sound no longer holds up the window (twelve clips took half a second; their files are now read in the background, so no frame took more than 1 ms), opening the Color page with many clips no longer freezes for a moment (it read every video's codec before drawing the first frame), the Deliver page no longer copies the whole project on every frame, and the Media page's file browser and media pool draw only the clips in view, so big projects and folders with thousands of files stay smooth when switching pages
- ✅ **Timeline:** a disabled clip no longer shows through a transition (dissolve or dip) next to it
- ✅ **Timeline:** dragging a clip, trim or roll no longer copies the whole project on every pointer move
- ✅ **Stills:** shrinking a big picture uses less memory, and an image too big to show is refused from its header before it is decoded
- ✅ **Projects:** the pictures kept in a project (stills) load faster
- ✅ **Video:** a frame with no time of its own is placed by its best-effort time instead of at the start of the file

**🔄 Changed**
- ✅ **Linux packages:** `.picasso`, `.drp`, `.drt` and `.drb` files open with Picasso Suite, and video files offer it under "Open With", several at once
- ✅ **Linux packages:** the .deb and .rpm name the window libraries they need, so a minimal system gets them; the .rpm names them in a way both Fedora and openSUSE understand
- ✅ **Flatpak:** footage on USB drives and memory cards (`/media`, `/run/media`) can be imported
- ✅ **Linux:** icons in 16 to 256 pixels, so menus and docks show a sharp icon at every size
- ✅ **Releases:** a release is only published when every package built and the Linux ones installed and started on seven systems, so a failed build can no longer leave a release with fewer packages
- ✅ **Flathub:** a manifest that builds FFmpeg and its codecs from source (`packaging/flathub`), built in CI on x86_64 and ARM64 and checked with Flathub's linter, with notes for the submission, which is not made yet; the screenshots and links in the app's appstream entry are filled in
- ✅ **Releases:** a release candidate (a tag such as `v0.3.0-rc1`) is published as a pre-release for testing; Check for Updates does not offer it
- ✅ **Development:** the tests run on every pull request, and a test checks `Menu-Status.md` against the menus' table
- ✅ **Development:** `PICASSO_PERF=pages picasso-suite [--place] <files>` shows each page in turn and prints how long its frames take, and files given with `--place` now go one after another on the timeline instead of all at the playhead
- ✅ **Status in the README:** a picture and a table of how much is made (menu entries that work, roadmap items done), drawn from `Menu-Status.md` and `ROADMAP.md` by `python3 scripts/status.py`

**⚠️ Known limitations**
- ❌ A native Wayland window takes no dropped files (winit 0.30 has no support and eframe 0.37 uses it), so the window opens through XWayland
- ❌ The ARM64 and Flatpak packages are only started on GitHub's machines (containers with a virtual screen and a software Vulkan), not on real ARM or Linux desktops
- ❌ Vulkan and the other graphics interfaces were only tried on an AMD card; run `picasso-suite --graphics-check` and send the report in
- ❌ Nothing has been tried on a real Raspberry Pi or other ARM computer yet; if you have one, run `picasso-suite --graphics-check` and send the report in
- ❌ The Color page's palettes (the wheels) keep their size, so they cannot be resized, and the Deliver settings form scrolls sideways in a narrow column instead of shrinking
- ❌ The Flathub submission is not made yet

## 0.2.1

Picasso Suite 0.2.1 is the full editor: all eight pages (Media, Photo, Cut, Edit, Fusion, Color,
Fairlight and Deliver), with FFmpeg included in every package.

**✨ New**
- ✅ **Curve editing:** on the Color page, add, drag and remove points on the Y, R, G and B curves to grade the picture
- ✅ **Curve intensity:** each curve's strength can be set from 0 to 100
- ✅ **Soft Clip:** clip and soften black (Low, L.S.) and white (High, H.S.)
- ✅ **Curves are saved** with each node, and a whole drag is one undo step
- ✅ **Check for Updates:** Help > Check for Updates finds the latest release and installs it on Windows, macOS, Flatpak, .deb and .rpm
- ✅ **Safe Windows updates:** if an update fails partway, your version is restored
- ✅ **More codecs on the Deliver page:** AV1 (MP4), VP9 (WebM), Avid DNxHR LB, SQ, HQ, HQX and 444 (QuickTime), lossless FFV1 with FLAC sound (Matroska) and MP3 audio
- ✅ **10-bit video:** H.264 High 10, H.265 Main 10, AV1 and VP9 profile 2, from the Encoding Profile list
- ✅ **Graphics card encoders:** Encoder can be NVIDIA, Intel Quick Sync, AMD, Apple VideoToolbox or VA-API for H.264, H.265 and AV1; the ones this computer has no card for are greyed out
- ✅ **Hardware decoding:** H.264, H.265, AV1, VP9, MPEG-2 and VC-1 are decoded on the graphics card (D3D11 on Windows, VideoToolbox on macOS, VA-API or CUDA on Linux), with a switch in System Preferences > Decode Options; a clip the card can't decode falls back to the processor
- ✅ **Keyboard Customization:** click any command's keys and press a new one; a key another command has is moved after asking; Reset and Reset All
- ✅ **OpenTimelineIO export:** File > Export > Timeline: OpenTimelineIO, for Resolve, Premiere, Avid and others
- ✅ System Preferences and your keys are saved, and stay after a restart

**🔄 Changed**
- ✅ Renders start at **60 fps** by default
- ✅ The camera tab on the page bar is now called **Media**

**🛠️ Fixed**
- 🛠️ Media and Deliver no longer flash black when you open them
- 🛠️ Dragging a clip from the media pool onto the Cut page timeline works again
- 🛠️ Dragging the playhead no longer makes the viewer flash black

**⚡ Performance**
- 🛠️ Playback is about **2x lighter**: Cut 4.8 → 2.4 ms, Edit 6.6 → 3.1 ms, Color 8.6 → ~4 ms per frame
- 🛠️ Videos are decoded at the viewer's size, so pictures show faster
- 🛠️ The Color page scopes update 20 times a second and no longer slow down playback
- 🛠️ Color grading is up to **14x faster** (wheels + curves: 6.5 → 0.45 ms per picture)

**⚠️ Known limitations**
- ❌ The Hue vs Hue, Hue vs Sat, Hue vs Lum, Lum vs Sat, Sat vs Sat and Sat vs Lum curves aren't done yet
- ❌ Check for Updates has only been tested on Windows so far
- ❌ The graphics card encoders were only tried with an AMD card on Windows; NVIDIA, Intel, Apple and VA-API are untested
- ❌ The cards make no 10-bit H.264 (it is offered with the Native encoder only)
- ❌ The macOS package is for Apple silicon only
- ❌ Projects from 0.2.0 and earlier (`.pcsp`) do not open in 0.2.1, which saves `.picasso` projects

## 0.2.0 hotfix

### New
- [x] **Preferences:** **File ▸ Preferences…** (`Ctrl/Cmd+,`) with a **Decode Options** section, arranged as in Resolve's Preferences ▸ System ▸ Decode Options. Contributed by @GMNEdits.
- [x] **Hardware-decode toggle:** turn hardware decoding on or off for the whole app, for when a GPU driver decodes a clip badly (green frames, corruption, media going offline). It was only possible with the `PICASSO_NO_HWACCEL` environment variable before.
- [x] The preview and source viewers restart their decoders when the toggle changes, so it applies without restarting the app. Export, thumbnails and overviews follow the setting when they are created.
- [x] The setting is saved to `preferences.json` in the per-user config folder next to `keybinds.json` (`%APPDATA%\PicassoSuite`, `~/Library/Application Support/PicassoSuite`, `~/.config/PicassoSuite`), and is not stored in the project.

### Changed
- [x] Hardware decoding stays on by default, as before; nothing is written until the toggle is used.
- [x] `PICASSO_NO_HWACCEL=1` still turns hardware decoding off for a single run, whatever the preference says.
- [x] The README documents the Preferences window, the `Ctrl/Cmd+,` shortcut and `preferences.json`.

### Thanks
- Thanks to @GMNEdits for the Preferences window and the hardware-decode toggle.

## 0.2.0

### New
- [x] **Sound during playback:** the timeline's audio plays in sync with the viewer at normal speed, with each clip's volume.
- [x] **Edit page:** the media pool, source and timeline viewers and track headers laid out as in DaVinci Resolve at 1080p.
- [x] **Color page:** Curves panel with a histogram that follows playback; wheels, Soft Clip and scopes arranged as in Resolve.
- [x] **Deliver page:** Custom Export preset list with icons and submenus; the render settings fit without scrolling.
- [x] **Frame-rate prompt:** when the first imported video has another frame rate, you choose whether to change the project's.
- [x] **Delete track:** right-click a track header to delete it (the last track of each kind stays).
- [x] New projects start with one picture track (V1) and one sound track (A1), as in Resolve.
- [x] Linux: the Flatpak may use the sound server (`--socket=pulseaudio`).

### Changed
- [x] Every page is laid out to Resolve's 1080p size; the layout scales with the window.
- [x] Selection arrow and loop icon redrawn to match Resolve.
- [x] Cut page timeline, overview and detail views as in Resolve.
- [x] Release builds use full link-time optimisation and drop debug symbols.
- [x] Page bar and top toolbars labelled as in Resolve.

### Fixed
- [x] The Selection tool's red arrow was misshapen; it is a proper cursor arrow now. The speaker icon too.
- [x] The 1080p pages keep a safe minimum body height, so a very short window cannot give them negative sizes.

### Removed
- [x] The unused `tracing` dependency of `picasso-media`.

### Known limitations
- [ ] Curve editing in the Color page's Curves panel is not yet possible (the histogram and controls are shown).
- [ ] Edit and Soft Clip effects are shown but do nothing yet.
- [ ] Deliver's interchange presets (Premiere, Avid) are listed but not built.
- [ ] Cut page track buttons are shown but do nothing yet.
- [ ] The Photo page still shows the Capture tools.
- [ ] Sound plays at normal speed only; fast-forward and reverse stay silent.
- [ ] macOS package does not include FFmpeg yet.

## 0.1.0 hotfix

### Fixed
- ✅ UI stalls while the About window was open: it started `ffprobe` and `ffmpeg` on every frame (about 115 ms each). The FFmpeg version is now read once at startup.
- ✅ The window title was sent to the operating system on every frame; now only when it changes.
- ✅ **Windows: FFmpeg is now included.** The zip has `ffmpeg.exe` and `ffprobe.exe` next to `picasso-suite.exe`, so the app works right after unzipping (FFmpeg 8.1, BtbN's GPL build, checked against its SHA-256).
- ✅ The "FFmpeg is missing" message now says that two programs are needed (`ffmpeg` and `ffprobe`), names the one that is missing, and says exactly where to put them on each system.
- ✅ Flatpak: building FFmpeg from source inside the sandbox kept failing; the Flatpak now ships a prebuilt static FFmpeg 8.1 (BtbN's GPL build, with x264/x265), checked against its published SHA-256.

### Added
- ✅ **Rebindable keyboard shortcuts** (Help → Keyboard Shortcuts): click a key and press the new combination. If another command already uses it you get a warning and can replace it or cancel. Reset per command or all; saved to `keybinds.json` in the user's config folder.
- ✅ `PICASSO_PERF=1` prints frame times and closes, for checking performance.

### Docs
- ✅ README: install table per operating system, build-from-source section, screenshots.
- ✅ Roadmap grouped by phases; changelog with checkboxes.

## 0.1.0 — first release

### Application
- ✅ Native Rust application (egui/eframe, wgpu) with a borderless window and DaVinci Resolve-style
  chrome: menu bar, interface toolbar, page bar, and a design-grid scaler that keeps every
  proportion fixed at any window size.
- ✅ **Cut page** (default): media pool, viewer with dB meter, transport row with TCG timecode,
  overview timeline with draggable playhead and 10-second ruler labels, zoomable detail
  timeline with a fixed centre playhead and drag-to-scrub.
- ✅ **Edit page** and **Deliver page** with a reusable timeline widget.
- ✅ One command table drives menus, shortcut labels and key handling (a test asserts no key is
  bound twice).
- ✅ Original vector icon set drawn in code; Noto Sans bundled (SIL OFL).

### Editing
- ✅ Multi-track video and audio timeline; place/overwrite, blade, delete and ripple delete, move,
  trim, snapping, linked picture/sound groups.
- ✅ Per-clip zoom, position, opacity and volume.
- ✅ Snapshot-based undo/redo.
- ✅ Timelines start at 01:00:00:00, like Resolve.

### Media (via FFmpeg, on every OS)
- ✅ Probe and frame-accurate decode of every format FFmpeg reads: H.264, H.265/HEVC, ProRes,
  DNxHR, AV1, VP9, MPEG-2, image sequences (EXR, DPX, TIFF, PNG…), stills, and audio formats.
- ✅ Thumbnails and waveforms for the timeline.
- ✅ Float32 compositor with a decoder pool.

### Export
- ✅ H.264 and H.265 (8- and 10-bit) with x264/x265 and hardware encoders: NVENC, Quick Sync,
  AMF, VA-API (Linux), VideoToolbox (macOS), whichever your FFmpeg build provides.
- ✅ MP4/MOV/MKV containers; ProRes, DNxHR, AV1, VP9, FFV1, PNG sequences, WAV, MP3.
- ✅ Audio mixed across segments with an `amix` graph.

### Projects and interchange
- ✅ `.pcsp` project files (JSON).
- ✅ **`.drp` import**: timeline, tracks, clips, media paths, resolution and frame rate; verified
  against the shipped `Fuse.drp`. Media paths from other operating systems are relinked.
- ✅ OpenTimelineIO export.

### Command line
- ✅ `picasso`: `probe`, `presets`, `new`, `import-drp`, `otio`, `render`.

### Packaging
- ✅ Windows zip, Linux `.deb`, `.rpm` and Flatpak, macOS universal (Intel + Apple silicon) app,
  all built by the release workflow.

### Known limitations
- ❌ No real-time audio playback yet.
- ❌ Edit and Deliver pages are not yet restyled to match the Cut page.
- ❌ FFmpeg (`ffmpeg`/`ffprobe`) must be installed, except in the Flatpak, which bundles it.
- ❌ Only plain-XML parts of `.drp` files are imported.
- ❌ Not affiliated with Blackmagic Design.
