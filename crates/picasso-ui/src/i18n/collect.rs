//! A tool for the translators, not a check: draws every page and the main windows without a
//! screen and writes the text they paint, one line each, so the translations are made for the
//! text the program really shows.
//!
//! `PICASSO_I18N_COLLECT=scripts/i18n/ui_labels.txt cargo test -p picasso-ui collect_ui_text -- --ignored`

use std::collections::BTreeSet;

use egui::{Context, Pos2, RawInput, Rect, Shape};

use crate::app::{Page, StudioApp};

fn texts(shape: &Shape, out: &mut BTreeSet<String>) {
    match shape {
        Shape::Text(t) => {
            let text = t.galley.job.text.trim();
            if !text.is_empty() {
                out.insert(text.to_string());
            }
        }
        Shape::Vec(shapes) => shapes.iter().for_each(|s| texts(s, out)),
        _ => {}
    }
}

fn frames(app: &mut StudioApp, ctx: &Context, size: (f32, f32), out: &mut BTreeSet<String>) {
    for _ in 0..3 {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(size.0, size.1))),
            ..RawInput::default()
        };
        let mut output = ctx.run_ui(input, |ui| app.ui(ui));
        // Nothing is drawn, so nothing is uploaded.
        output.textures_delta.clear();
        for clipped in &output.shapes {
            texts(&clipped.shape, out);
        }
    }
}

#[test]
#[ignore = "a tool: writes the file named by PICASSO_I18N_COLLECT"]
fn collect_ui_text() {
    let Some(path) = std::env::var_os("PICASSO_I18N_COLLECT") else {
        return;
    };
    let ctx = Context::default();
    let mut app = StudioApp::new(&ctx);
    app.load_demo();
    let mut found = BTreeSet::new();
    // The project manager comes first, then the pages.
    frames(&mut app, &ctx, (1920.0, 1080.0), &mut found);
    app.manager.open = false;
    for page in [
        Page::Media,
        Page::Cut,
        Page::Edit,
        Page::Color,
        Page::Capture,
        Page::Effects,
        Page::Audio,
        Page::Deliver,
    ] {
        app.page = page;
        frames(&mut app, &ctx, (1920.0, 1080.0), &mut found);
    }
    // The pages with a clip chosen and the inspector open.
    let first = app.project().tracks.iter().flat_map(|t| t.clips.first()).next().map(|c| c.id);
    app.selection = first;
    app.inspector_open = true;
    for page in [Page::Cut, Page::Edit, Page::Color, Page::Effects, Page::Audio] {
        app.page = page;
        frames(&mut app, &ctx, (1920.0, 1080.0), &mut found);
    }
    // The Deliver page's other tabs and the sections that open.
    app.page = Page::Deliver;
    app.deliver_ui.advanced_open = true;
    app.deliver_ui.subtitle_open = true;
    frames(&mut app, &ctx, (1920.0, 1080.0), &mut found);
    app.deliver_ui.tab = crate::deliver::Tab::Audio;
    frames(&mut app, &ctx, (1920.0, 1080.0), &mut found);
    app.deliver_ui.tab = crate::deliver::Tab::File;
    frames(&mut app, &ctx, (1920.0, 1080.0), &mut found);
    // The windows that open over the pages.
    app.show_about = true;
    app.show_settings = true;
    app.show_shortcuts = true;
    app.extra.preferences = true;
    app.extra.media_management = true;
    frames(&mut app, &ctx, (1920.0, 1080.0), &mut found);
    let text = found.into_iter().collect::<Vec<_>>().join("\n");
    std::fs::write(path, text).expect("the collected text is written");
}

/// What the first frame paints, with the language `language` in use.
fn painted_in(language: crate::i18n::Language) -> BTreeSet<String> {
    let ctx = Context::default();
    let mut app = StudioApp::new(&ctx);
    app.load_demo();
    crate::i18n::set(language);
    crate::i18n::sync_fonts(&ctx);
    let mut found = BTreeSet::new();
    // The project manager is the first thing a program with saved projects shows; one with none
    // (a clean computer, the test runners) opens on the editor, so it is opened here.
    app.manager.open = true;
    frames(&mut app, &ctx, (1920.0, 1080.0), &mut found);
    app.manager.open = false;
    app.page = Page::Media;
    frames(&mut app, &ctx, (1920.0, 1080.0), &mut found);
    crate::i18n::set(crate::i18n::Language::English);
    found
}

#[test]
fn the_pages_are_painted_in_the_language_chosen() {
    use crate::i18n::{Language, CHANGING};
    let _only = CHANGING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let english = painted_in(Language::English);
    assert!(english.contains("Projects"), "{english:?}");
    assert!(english.contains("Media Storage"), "{english:?}");

    let spanish = painted_in(Language::Spanish);
    // The text with a translation is painted in it, and not in English.
    assert!(spanish.contains("Proyectos"), "{spanish:?}");
    assert!(spanish.contains("Almacenamiento"), "{spanish:?}");
    assert!(!spanish.contains("Projects"));
    assert!(!spanish.contains("Media Storage"));
    // What has no translation (a name) is as it was.
    assert!(spanish.contains("Picasso Suite"));

    // Back in English, nothing stays translated: the text is only changed on the way to the screen.
    let again = painted_in(Language::English);
    assert!(again.contains("Projects") && !again.contains("Proyectos"));
}

/// Every piece of text a frame paints with the box it takes, in the order painted.
fn boxes(shape: &Shape, out: &mut Vec<(String, Rect)>) {
    match shape {
        Shape::Text(t) => {
            let text = t.galley.job.text.trim();
            if !text.is_empty() {
                out.push((text.to_string(), t.visual_bounding_rect()));
            }
        }
        Shape::Vec(shapes) => shapes.iter().for_each(|s| boxes(s, out)),
        _ => {}
    }
}

/// What each page and window paints in `language`: its texts with their boxes.
fn painted_boxes(language: crate::i18n::Language) -> Vec<(String, Vec<(String, Rect)>)> {
    painted_boxes_of(language, true)
}

/// The same, for the demo project or for an empty one (whose pools say they are empty).
fn painted_boxes_of(language: crate::i18n::Language, demo: bool) -> Vec<(String, Vec<(String, Rect)>)> {
    let ctx = Context::default();
    let mut app = StudioApp::new(&ctx);
    if demo {
        app.load_demo();
    }
    crate::i18n::set(language);
    crate::i18n::sync_fonts(&ctx);
    let mut all = Vec::new();
    let mut take = |name: &str, app: &mut StudioApp| {
        let mut found = Vec::new();
        for _ in 0..3 {
            found.clear();
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1920.0, 1080.0))),
                ..RawInput::default()
            };
            let mut output = ctx.run_ui(input, |ui| app.ui(ui));
            output.textures_delta.clear();
            for clipped in &output.shapes {
                boxes(&clipped.shape, &mut found);
            }
        }
        all.push((name.to_string(), found));
    };
    take("project manager", &mut app);
    app.manager.open = false;
    for page in [
        Page::Media,
        Page::Cut,
        Page::Edit,
        Page::Color,
        Page::Capture,
        Page::Effects,
        Page::Audio,
        Page::Deliver,
    ] {
        app.page = page;
        take(&format!("{page:?}"), &mut app);
    }
    // The pages with a clip chosen and the inspector open (the demo project has clips).
    if demo {
        let first = app.project().tracks.iter().flat_map(|t| t.clips.first()).next().map(|c| c.id);
        app.selection = first;
        app.inspector_open = true;
        for page in [Page::Cut, Page::Edit, Page::Color, Page::Effects, Page::Audio] {
            app.page = page;
            take(&format!("{page:?}, clip chosen"), &mut app);
        }
    }
    // The Deliver page's other tabs and the sections that open.
    app.page = Page::Deliver;
    app.deliver_ui.advanced_open = true;
    app.deliver_ui.subtitle_open = true;
    take("Deliver, advanced", &mut app);
    app.deliver_ui.tab = crate::deliver::Tab::Audio;
    take("Deliver, audio", &mut app);
    app.deliver_ui.tab = crate::deliver::Tab::File;
    take("Deliver, file", &mut app);
    // (Not the windows: they open over each other, and over the menu behind, in English too.)
    crate::i18n::set(crate::i18n::Language::English);
    all
}

/// Two texts of one row that run into each other.
fn collisions(boxes: &[(String, Rect)]) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    for (i, (_, a)) in boxes.iter().enumerate() {
        for (j, (_, b)) in boxes.iter().enumerate().skip(i + 1) {
            let both = a.intersect(*b);
            if both.width() > 2.0 && both.height() > 0.5 * a.height().min(b.height()) {
                found.push((i, j));
            }
        }
    }
    found
}

#[test]
fn no_language_puts_a_text_on_top_of_another_that_english_did_not() {
    use crate::i18n::{Language, CHANGING};
    let _only = CHANGING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut report = String::new();
    // With the demo project, and with an empty one (whose pools show their hints).
    for demo in [true, false] {
        let english = painted_boxes_of(Language::English, demo);
        for language in Language::ALL.into_iter().skip(1) {
            let shown = painted_boxes_of(language, demo);
            for ((page, then), (_, now)) in english.iter().zip(&shown) {
                if then.len() != now.len() {
                    report.push_str(&format!("{language:?} {page}: {} texts, English {}
", now.len(), then.len()));
                    continue;
                }
                let before: std::collections::HashSet<_> = collisions(then).into_iter().collect();
                for (i, j) in collisions(now) {
                    if !before.contains(&(i, j)) {
                        report.push_str(&format!(
                            "{language:?} {page}: {:?} runs into {:?} (English {:?} / {:?})
",
                            now[i].0, now[j].0, then[i].0, then[j].0
                        ));
                    }
                }
            }
        }
    }
    assert!(report.is_empty(), "texts on top of each other:
{report}");
}

#[test]
fn a_right_aligned_label_keeps_its_right_edge_in_every_language() {
    use crate::i18n::{Language, CHANGING};
    let _only = CHANGING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // On the Deliver page "Graphics card" is a label that ends where the value begins.
    let find = |all: &[(String, Vec<(String, Rect)>)], text: &str| {
        all.iter()
            .filter(|(page, _)| page == "Deliver")
            .flat_map(|(_, boxes)| boxes.iter())
            .find(|(t, _)| t == text)
            .map(|(_, r)| *r)
    };
    let english = painted_boxes(Language::English);
    let was = find(&english, "Graphics card").expect("the label is painted");
    for (language, text) in [
        (Language::French, "Carte graphique"),
        (Language::German, "Grafikkarte"),
        (Language::Spanish, "Tarjeta gráfica"),
        (Language::Portuguese, "Placa gráfica"),
    ] {
        let shown = painted_boxes(language);
        let now = find(&shown, text).unwrap_or_else(|| panic!("{language:?}: {text:?} is painted"));
        assert!(
            (now.right() - was.right()).abs() < 2.0,
            "{language:?}: the label ends at {} and not where the English ended, {}",
            now.right(),
            was.right()
        );
    }
}

#[test]
fn a_hint_in_a_panel_is_no_wider_than_the_english_one_in_any_language() {
    use crate::i18n::{Language, CHANGING};
    let _only = CHANGING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let width_of = |all: &[(String, Vec<(String, Rect)>)], text: &str| {
        all.iter()
            .filter(|(page, _)| page == "Media")
            .flat_map(|(_, boxes)| boxes.iter())
            .find(|(t, _)| t == text)
            .map(|(_, r)| r.width())
    };
    let english = painted_boxes_of(Language::English, false);
    let was = width_of(&english, "Add clips from Media Storage to get started").expect("painted");
    for (language, text) in [
        (Language::Portuguese, "Adicione clips do armazenamento para começar"),
        (Language::Spanish, "Añade clips desde Almacenamiento para empezar"),
        (Language::French, "Ajoutez des clips depuis Stockage"),
        (Language::German, "Clips aus dem Speicher hinzufügen"),
    ] {
        let shown = painted_boxes_of(language, false);
        let now = width_of(&shown, text).unwrap_or_else(|| panic!("{language:?}: {text:?} is painted"));
        assert!(now <= was + 1.5, "{language:?}: the hint is {now} wide, the English {was}");
    }
}

/// Every small thing painted besides text (an icon, an arrow, a line of a divider) with its box, and
/// every text with its box, in the order painted: `(is text, box)`.
fn all_boxes(shape: &Shape, out: &mut Vec<(bool, Rect)>) {
    match shape {
        Shape::Text(t) => {
            if !t.galley.job.text.trim().is_empty() {
                out.push((true, t.visual_bounding_rect()));
            }
        }
        Shape::Vec(shapes) => shapes.iter().for_each(|s| all_boxes(s, out)),
        Shape::Noop => {}
        other => {
            let r = other.visual_bounding_rect();
            // Icons, arrows and dividers are small; a background is not what a text can run into.
            if r.is_positive() && r.width() <= 40.0 && r.height() <= 40.0 {
                out.push((false, r));
            }
        }
    }
}

fn painted_all(language: crate::i18n::Language) -> Vec<(String, Vec<(bool, Rect)>)> {
    let ctx = Context::default();
    let mut app = StudioApp::new(&ctx);
    app.load_demo();
    crate::i18n::set(language);
    crate::i18n::sync_fonts(&ctx);
    app.manager.open = false;
    let first = app.project().tracks.iter().flat_map(|t| t.clips.first()).next().map(|c| c.id);
    let mut all = Vec::new();
    let mut take = |name: String, app: &mut StudioApp| {
        let mut found = Vec::new();
        for _ in 0..3 {
            found.clear();
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1920.0, 1080.0))),
                ..RawInput::default()
            };
            let mut output = ctx.run_ui(input, |ui| app.ui(ui));
            output.textures_delta.clear();
            for clipped in &output.shapes {
                all_boxes(&clipped.shape, &mut found);
            }
        }
        all.push((name, found));
    };
    for (chosen, pages) in [
        (false, vec![Page::Media, Page::Cut, Page::Edit, Page::Color, Page::Capture, Page::Effects, Page::Audio, Page::Deliver]),
        (true, vec![Page::Cut, Page::Edit, Page::Color, Page::Effects, Page::Audio]),
    ] {
        app.selection = if chosen { first } else { None };
        app.inspector_open = chosen;
        for page in pages {
            app.page = page;
            take(format!("{page:?}{}", if chosen { ", clip chosen" } else { "" }), &mut app);
        }
    }
    crate::i18n::set(crate::i18n::Language::English);
    all
}

#[test]
fn no_language_runs_a_text_into_an_icon_that_english_did_not() {
    use crate::i18n::{Language, CHANGING};
    let _only = CHANGING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let english = painted_all(Language::English);
    let mut report = String::new();
    for language in Language::ALL.into_iter().skip(1) {
        let shown = painted_all(language);
        for ((page, then), (_, now)) in english.iter().zip(&shown) {
            if then.len() != now.len() {
                report.push_str(&format!("{language:?} {page}: {} shapes, English {}
", now.len(), then.len()));
                continue;
            }
            let hits = |boxes: &[(bool, Rect)]| {
                let mut hit = Vec::new();
                for (i, (is_text, a)) in boxes.iter().enumerate() {
                    if !*is_text {
                        continue;
                    }
                    for (j, (other_text, b)) in boxes.iter().enumerate() {
                        let both = a.intersect(*b);
                        if !*other_text && both.width() > 2.0 && both.height() > 0.5 * b.height().min(a.height()) {
                            hit.push((i, j));
                        }
                    }
                }
                hit
            };
            let before: std::collections::HashSet<_> = hits(then).into_iter().collect();
            for (i, j) in hits(now) {
                if !before.contains(&(i, j)) {
                    report.push_str(&format!("{language:?} {page}: text #{i} at {:?} runs into an icon at {:?}
", now[i].1, now[j].1));
                }
            }
        }
    }
    assert!(report.is_empty(), "text on icons:
{report}");
}

#[test]
fn every_character_of_every_language_has_a_glyph_in_the_fonts_bundled() {
    use crate::i18n::{CHANGING, Language};
    let _only = CHANGING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut missing = String::new();
    for language in Language::ALL {
        let ctx = Context::default();
        ctx.set_fonts(crate::theme::fonts_for(language));
        let font = egui::FontId::proportional(14.0);
        let mut absent: BTreeSet<char> = BTreeSet::new();
        let mut check = |ctx: &Context, text: &str| {
            ctx.fonts_mut(|fonts| {
                for c in text.chars().filter(|c| !c.is_control()) {
                    if !fonts.has_glyph(&font, c) {
                        absent.insert(c);
                    }
                }
            });
        };
        // Two frames: the fonts are given to the context at the start of the first.
        for _ in 0..2 {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0))),
                ..RawInput::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                for other in Language::ALL {
                    check(ui.ctx(), other.name());
                    if let Some((menus, entries, ui_text)) = language.data() {
                        if other == language {
                            for m in menus {
                                check(ui.ctx(), m);
                            }
                            for (_, shown) in entries.iter().chain(ui_text) {
                                check(ui.ctx(), shown);
                            }
                        }
                    }
                }
            });
            output.textures_delta.clear();
        }
        if !absent.is_empty() {
            missing.push_str(&format!("{language:?}: {absent:?}\n"));
        }
    }
    assert!(missing.is_empty(), "characters no bundled font draws:\n{missing}");
}

#[test]
fn every_menu_of_the_title_row_opens_on_every_page() {
    use crate::i18n::{CHANGING, Language};
    let _only = CHANGING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let pages = [Page::Media, Page::Cut, Page::Edit, Page::Capture, Page::Color, Page::Effects, Page::Audio, Page::Deliver];
    for language in [Language::English, Language::Spanish] {
        for page in pages {
            let ctx = Context::default();
            let mut app = StudioApp::new(&ctx);
            app.manager.open = false;
            app.load_demo();
            app.page = page;
            crate::i18n::set(language);
            crate::i18n::sync_fonts(&ctx);
            let size = egui::vec2(1920.0, 1080.0);
            let mut t = 0.0f64;
            let mut run = |app: &mut StudioApp, events: Vec<egui::Event>| {
                t += 0.016;
                let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), events, time: Some(t), ..RawInput::default() };
                let mut out = ctx.run_ui(input, |ui| app.ui(ui));
                out.textures_delta.clear();
            };
            for _ in 0..4 { run(&mut app, vec![]); }
            let mut bad = vec![];
            for (n, x) in [216.0f32, 267.0, 322.0, 384.0, 480.0, 535.0, 601.0, 665.0, 763.0, 842.0, 911.0, 1003.0, 1122.0].into_iter().enumerate() {
                let p = Pos2::new(x + 14.0, 19.0);
                let before = ctx.memory(|m| m.layer_ids().count());
                for _ in 0..2 { run(&mut app, vec![egui::Event::PointerMoved(p)]); }
                run(&mut app, vec![egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }]);
                run(&mut app, vec![egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }]);
                for _ in 0..2 { run(&mut app, vec![]); }
                let after = ctx.memory(|m| m.layer_ids().count());
                if after <= before { bad.push(n); }
                // click away to close
                let away = Pos2::new(1500.0, 600.0);
                run(&mut app, vec![egui::Event::PointerMoved(away)]);
                run(&mut app, vec![egui::Event::PointerButton { pos: away, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }]);
                run(&mut app, vec![egui::Event::PointerButton { pos: away, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }]);
                for _ in 0..2 { run(&mut app, vec![]); }
            }
            assert!(bad.is_empty(), "{language:?} {page:?}: menus {bad:?} do not open");
        }
    }
    crate::i18n::set(Language::English);
}

#[test]
fn a_label_with_a_version_in_it_is_painted_in_the_language_chosen() {
    use crate::i18n::{CHANGING, Language};
    let _only = CHANGING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let ctx = Context::default();
    crate::i18n::set(Language::Spanish);
    let mut found = BTreeSet::new();
    for _ in 0..2 {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0))),
            ..RawInput::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            ui.label("Picasso Suite 0.7.0 is available");
            ui.label("Update now");
            crate::i18n::translate_frame(ui.ctx());
        });
        output.textures_delta.clear();
        for clipped in &output.shapes {
            texts(&clipped.shape, &mut found);
        }
    }
    crate::i18n::set(Language::English);
    assert!(found.contains("Picasso Suite 0.7.0 está disponible"), "{found:?}");
    assert!(found.contains("Actualizar ahora"), "{found:?}");
}

#[test]
/// Two widgets with one id share what egui keeps for it: dragging one scroll bar moved another
/// list, and the wheel did not reach the right one (the Inspector of the Edit page and the pool).
fn no_page_uses_a_widget_id_twice() {
    let mut found = BTreeSet::new();
    for demo in [true, false] {
        for inspector in [false, true] {
            for page in [Page::Media, Page::Cut, Page::Edit, Page::Color, Page::Capture, Page::Effects, Page::Audio, Page::Deliver] {
                let ctx = Context::default();
                let mut app = StudioApp::new(&ctx);
                app.manager.open = false;
                if demo { app.load_demo(); }
                app.page = page;
                let first = app.project().tracks.iter().flat_map(|t| t.clips.first()).next().map(|c| c.id);
                app.selection = first;
                app.inspector_open = inspector;
                for _ in 0..4 {
                    let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1920.0, 1080.0))), ..RawInput::default() };
                    let mut out = ctx.run_ui(input, |ui| app.ui(ui));
                    out.textures_delta.clear();
                    let mut all = BTreeSet::new();
                    for c in &out.shapes { texts(&c.shape, &mut all); }
                    for t in all { if t.contains("Second use") { found.insert(format!("{page:?} demo={demo} inspector={inspector}: {t}")); } }
                }
            }
        }
    }
    assert!(found.is_empty(), "ids used twice:
{}", found.iter().cloned().collect::<Vec<_>>().join("
"));
}
