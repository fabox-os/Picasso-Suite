//! Edit's docked browser. It lists the built-ins and hands edits to the existing menu actions.

use std::sync::OnceLock;

use egui::{
    vec2, Atom, Button, ComboBox, Context, EventFilter, Key, Label, Layout, Modifiers, Rect,
    RichText, ScrollArea, Stroke, TextEdit, TextStyle, TextWrapMode, Ui, UiBuilder,
};
use picasso_core::{
    effect_looks::LOOKS, AudioEffectKind, EffectKind, GeneratorKind, TransitionKind,
};

use crate::app::StudioApp;
use crate::menu::{Act, Entry, EFFECTS};
use crate::theme;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum Category {
    #[default]
    Transitions,
    Titles,
    Generators,
    Effects,
    Looks,
    Audio,
}

impl Category {
    const ALL: [Self; 6] = [
        Self::Transitions,
        Self::Titles,
        Self::Generators,
        Self::Effects,
        Self::Looks,
        Self::Audio,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Transitions => "Video Transitions",
            Self::Titles => "Titles",
            Self::Generators => "Generators",
            Self::Effects => "Effects",
            Self::Looks => "Looks",
            Self::Audio => "Audio FX",
        }
    }

    fn tag(self) -> &'static str {
        match self {
            Self::Effects => "Picasso FX",
            _ => self.label(),
        }
    }
}

struct Item {
    label: &'static str,
    act: Act,
    tagged: String,
    search_key: String,
}

/// Direct commands which don't have a corresponding kind in a core registry.
fn menu_items(category: Category) -> impl Iterator<Item = (&'static str, Act)> {
    EFFECTS
        .iter()
        .filter_map(move |entry| match entry {
            Entry::Sub { label, items } if *label == category.label() => Some(*items),
            _ => None,
        })
        .flatten()
        .filter_map(|entry| match entry {
            Entry::Item {
                act: Some(Act::TrackEq),
                ..
            } => Some(("Track EQ", Act::TrackEq)),
            Entry::Item {
                act: Some(Act::TrackDynamics),
                ..
            } => Some(("Track Dynamics", Act::TrackDynamics)),
            Entry::Item {
                label,
                act: Some(act),
                ..
            } => Some((*label, *act)),
            _ => None,
        })
}

fn catalog() -> &'static [Vec<Item>; Category::ALL.len()] {
    static CATALOG: OnceLock<[Vec<Item>; Category::ALL.len()]> = OnceLock::new();
    CATALOG.get_or_init(|| {
        Category::ALL.map(|category| {
            let mut items = Vec::new();
            let mut add = |(label, act)| {
                let tagged = format!("{label}  ·  {}", category.tag());
                let search_key =
                    format!("{label} {} {}", category.label(), category.tag()).to_lowercase();
                items.push(Item {
                    label,
                    act,
                    tagged,
                    search_key,
                });
            };
            match category {
                Category::Transitions => TransitionKind::ALL
                    .into_iter()
                    .map(|kind| (kind.label(), Act::AddTransition(kind)))
                    .for_each(add),
                Category::Titles => menu_items(category).for_each(add),
                Category::Generators => {
                    menu_items(category).for_each(&mut add);
                    GeneratorKind::ALL
                        .into_iter()
                        .map(|kind| (kind.label(), Act::AddGenerator(kind)))
                        .for_each(add);
                }
                Category::Effects => EffectKind::ALL
                    .into_iter()
                    .map(|kind| (kind.label(), Act::AddEffect(kind)))
                    .for_each(add),
                Category::Looks => LOOKS
                    .iter()
                    .enumerate()
                    .map(|(i, look)| (look.label, Act::ApplyLook(i)))
                    .for_each(add),
                Category::Audio => {
                    menu_items(category).for_each(&mut add);
                    AudioEffectKind::ALL
                        .into_iter()
                        .map(|kind| (kind.label(), Act::AddAudioEffect(kind)))
                        .for_each(add);
                }
            }
            items
        })
    })
}

#[derive(Default)]
pub(crate) struct EffectsLibrary {
    category: Category,
    query: String,
    filtered_query: String,
    matches: Vec<&'static Item>,
}

pub(crate) fn has_category_pane(width: f32) -> bool {
    width >= 128.0
}

/// Use an independent cursor and a hard clip so the dock cannot grow into adjacent panels.
fn pane(ui: &mut Ui, rect: Rect, id: &'static str) -> Ui {
    let mut child = ui.new_child(
        UiBuilder::new()
            .id_salt(id)
            .max_rect(rect)
            .layout(Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(ui.clip_rect().intersect(rect));
    child
}

impl EffectsLibrary {
    pub(crate) fn header(&mut self, ui: &mut Ui, rect: Rect) {
        pane(ui, rect.shrink2(vec2(8.0, 1.0)), "edit_effects_header").with_layout(
            Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add(Label::new(RichText::new("Effects Library").small().strong()).truncate());
                ui.push_id("search", |ui| {
                    let id = ui.make_persistent_id("field");
                    // Keep Escape in the field until we clear and unfocus it. Like Media Pool search,
                    // TextEdit focus also blocks the app's global shortcuts.
                    if ui.memory(|m| m.has_focus(id))
                        && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
                    {
                        self.query.clear();
                        ui.memory_mut(|m| m.surrender_focus(id));
                    }
                    let width = ui.available_width() - 34.0;
                    ui.add(
                        TextEdit::singleline(&mut self.query)
                            .id(id)
                            .hint_text("Search")
                            .font(TextStyle::Small)
                            .margin(vec2(5.0, 2.0))
                            .desired_width(width)
                            .event_filter(EventFilter {
                                horizontal_arrows: true,
                                vertical_arrows: true,
                                escape: true,
                                ..EventFilter::default()
                            }),
                    );
                    if !self.query.is_empty()
                        && ui.small_button("×").on_hover_text("Clear search").clicked()
                    {
                        self.query.clear();
                        ui.memory_mut(|m| m.request_focus(id));
                    }
                });
            },
        );
    }

    /// Refilter only on edits, not on hover, scroll, or idle frames.
    fn update_matches(&mut self) -> bool {
        if self.filtered_query == self.query {
            return false;
        }
        self.filtered_query.clone_from(&self.query);
        self.matches.clear();
        let query = self.query.trim().to_lowercase();
        if !query.is_empty() {
            self.matches
                .extend(catalog().iter().flatten().filter(|item| {
                    query
                        .split_whitespace()
                        .all(|word| item.search_key.contains(word))
                }));
        }
        true
    }

    fn categories(&mut self, ui: &mut Ui) {
        for category in Category::ALL {
            ui.push_id(category, |ui| {
                let button = Button::selectable(
                    self.category == category,
                    (RichText::new(category.label()).small(), Atom::grow()),
                )
                .wrap_mode(TextWrapMode::Truncate);
                if ui.add_sized([ui.available_width(), 28.0], button).clicked() {
                    self.category = category;
                }
            });
        }
    }

    fn body(&mut self, ui: &mut Ui, mut rect: Rect, split_x: f32) -> Option<Act> {
        let old_category = self.category;
        let split = has_category_pane(split_x - rect.left());
        if split {
            let mut categories = rect;
            categories.max.x = split_x;
            ui.painter()
                .rect_filled(categories, 0.0, theme::TOOLBAR1_BG);
            ui.painter().line_segment(
                [categories.right_top(), categories.right_bottom()],
                Stroke::new(1.0, theme::DIVIDER),
            );
            pane(ui, categories.shrink(4.0), "edit_effects_categories")
                .push_id("categories", |ui| self.categories(ui));
            rect.min.x = split_x;
        }
        let mut items = pane(ui, rect.shrink(6.0), "edit_effects_items");
        if !split {
            items.push_id("category_picker", |ui| {
                ComboBox::from_id_salt("category")
                    .selected_text(self.category.label())
                    .width(ui.available_width())
                    .wrap_mode(TextWrapMode::Truncate)
                    .show_ui(ui, |ui| self.categories(ui));
            });
        }

        let updated = self.update_matches();
        let searching = !self.query.trim().is_empty();
        let category = &catalog()[self.category as usize];
        let count = if searching {
            self.matches.len()
        } else {
            category.len()
        };
        if count == 0 {
            items.weak("No effects match");
            return None;
        }
        if searching {
            items.weak("Search results");
        } else if split || self.category == Category::Effects {
            items.strong(self.category.tag());
        }
        items
            .push_id("item_scroll", |ui| {
                let mut scroll = ScrollArea::vertical().auto_shrink([false, false]);
                if updated || old_category != self.category {
                    scroll = scroll.vertical_scroll_offset(0.0);
                }
                scroll
                    .show_rows(ui, 28.0, count, |ui, rows| {
                        let mut chosen = None;
                        for index in rows {
                            let item = if searching {
                                self.matches[index]
                            } else {
                                &category[index]
                            };
                            let label = if searching {
                                item.tagged.as_str()
                            } else {
                                item.label
                            };
                            ui.push_id(item.act, |ui| {
                                let button = Button::new((label, Atom::grow()))
                                    .wrap_mode(TextWrapMode::Truncate)
                                    .frame_when_inactive(false);
                                let response = ui.add_sized([ui.available_width(), 28.0], button);
                                let response = if !label.is_empty() {
                                    response.on_hover_text(label)
                                } else {
                                    response
                                };
                                if response.clicked() {
                                    chosen = Some(item.act);
                                }
                            });
                        }
                        chosen
                    })
                    .inner
            })
            .inner
    }
}

impl StudioApp {
    pub(crate) fn edit_effects_library(
        &mut self,
        ui: &mut Ui,
        ctx: &Context,
        rect: Rect,
        split: f32,
    ) {
        if let Some(act) = self.effects_library.body(ui, rect, split) {
            // Menu commands may reveal the Inspector. Applying from the dock leaves the
            // right-hand panel alone, while keeping the same command, undo and status handling.
            let inspector_open = self.inspector_open;
            self.do_act(ctx, act);
            self.inspector_open = inspector_open;
        }
    }
}

#[cfg(test)]
mod edit_page_tests {
    use super::*;
    use crate::app::Page;
    use egui::{Event, FullOutput, Id, PointerButton, Pos2, RawInput, Shape};
    use picasso_core::{Command, MediaSource, Project, TrackKind};

    struct Editor {
        app: StudioApp,
        ctx: Context,
        time: f64,
        scale: f32,
        root: Id,
    }

    fn english() -> std::sync::MutexGuard<'static, ()> {
        let guard = crate::i18n::CHANGING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        crate::i18n::set(crate::i18n::Language::English);
        guard
    }

    impl Editor {
        fn new(demo: bool) -> Self {
            let ctx = Context::default();
            ctx.options_mut(|o| o.warn_on_id_clash = true);
            let mut app = StudioApp::new(&ctx);
            app.manager.open = false;
            app.page = Page::Edit;
            if demo {
                app.load_demo();
                app.selection = app
                    .project()
                    .tracks
                    .iter()
                    .find_map(|t| t.clips.first().map(|c| c.id));
            }
            Self {
                app,
                ctx,
                time: 0.0,
                scale: 1.0,
                root: Id::NULL,
            }
        }

        fn frame(&mut self, events: Vec<Event>) -> FullOutput {
            self.time += 0.016;
            let mut input = RawInput {
                screen_rect: Some(Rect::from_min_size(
                    Pos2::ZERO,
                    vec2(crate::chrome::DESIGN_WIDTH, 1080.0),
                )),
                events,
                time: Some(self.time),
                ..RawInput::default()
            };
            if let Some(viewport) = input.viewports.get_mut(&input.viewport_id) {
                viewport.native_pixels_per_point = Some(self.scale);
            }
            let mut output = self.ctx.run_ui(input, |ui| {
                self.root = ui.id();
                self.app.ui(ui);
            });
            output.textures_delta.clear();
            output
        }

        fn click(&mut self, pos: Pos2) -> FullOutput {
            self.frame(vec![Event::PointerMoved(pos)]);
            for pressed in [true, false] {
                self.frame(vec![Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                }]);
            }
            self.frame(vec![])
        }

        fn choose(&mut self, label: &str) -> Result<(), String> {
            self.app.effects_library.query = label.into();
            let frame = self.frame(vec![]);
            let pos = find(&frame, |text, _| {
                text.split_once("  ·  ")
                    .is_some_and(|(name, _)| name == label)
            })
            .ok_or_else(|| format!("no library row for {label}"))?;
            self.click(pos);
            Ok(())
        }
    }

    /// Painted text and its real bounds and scissor, including shapes nested by egui.
    fn visit_text(frame: &FullOutput, mut visit: impl FnMut(&str, Rect, Rect)) {
        fn visit_shape(shape: &Shape, clip: Rect, visit: &mut impl FnMut(&str, Rect, Rect)) {
            match shape {
                Shape::Text(t) => visit(&t.galley.job.text, t.visual_bounding_rect(), clip),
                Shape::Vec(shapes) => {
                    for shape in shapes {
                        visit_shape(shape, clip, visit);
                    }
                }
                _ => {}
            }
        }
        for clipped in &frame.shapes {
            visit_shape(&clipped.shape, clipped.clip_rect, &mut visit);
        }
    }

    fn find(frame: &FullOutput, mut matches: impl FnMut(&str, Rect) -> bool) -> Option<Pos2> {
        let mut found = None;
        visit_text(frame, |text, bounds, clip| {
            let visible = bounds.intersect(clip);
            if found.is_none() && visible.is_positive() && matches(text, bounds) {
                found = Some(visible.center());
            }
        });
        found
    }

    fn key(key: Key) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    #[test]
    fn the_idle_dock_only_paints_visible_rows() {
        let ctx = Context::default();
        theme::install(&ctx);
        let mut library = EffectsLibrary {
            category: Category::Effects,
            ..EffectsLibrary::default()
        };
        let header = Rect::from_min_max(egui::pos2(0.0, 80.0), egui::pos2(446.0, 104.0));
        let body = Rect::from_min_max(egui::pos2(0.0, 105.0), egui::pos2(446.0, 1030.0));
        let mut frame = || {
            let mut output = ctx.run_ui(RawInput::default(), |ui| {
                library.header(ui, header);
                let _ = library.body(ui, body, 143.0);
            });
            output.textures_delta.clear();
            output
        };
        let _ = frame();
        let output = frame();
        let effects = &catalog()[Category::Effects as usize];
        let mut painted = 0;
        visit_text(&output, |text, _, _| {
            painted += usize::from(effects.iter().any(|item| item.label == text));
        });
        assert!((1..40).contains(&painted), "{painted} visible effect rows");

        let start = std::time::Instant::now();
        for _ in 0..100 {
            let _ = frame();
        }
        eprintln!(
            "Average idle Effects Library frame: {:?}",
            start.elapsed() / 100
        );
    }

    #[test]
    fn the_real_toolbar_toggles_the_edit_left_dock() -> Result<(), &'static str> {
        let _english = english();
        for inspector in [false, true] {
            let mut editor = Editor::new(false);
            editor.app.inspector_open = inspector;
            let frame = editor.frame(vec![]);
            assert!(!editor.app.effects_library_open);
            let pool_search = editor.root.with(("edit_tb2", 543_i32, 120_i32));
            assert!(editor.ctx.read_response(pool_search).is_some());
            let toolbar = |label: &'static str| {
                move |text: &str, rect: Rect| {
                    text == label && (40.0..79.0).contains(&rect.center().y)
                }
            };
            let effects =
                find(&frame, toolbar("Effects")).ok_or("missing Effects toolbar button")?;
            let pool =
                find(&frame, toolbar("Media Pool")).ok_or("missing Media Pool toolbar button")?;
            let open = editor.click(effects);
            assert!(editor.app.effects_library_open);
            assert!(find(&open, |text, _| text == "Effects Library").is_some());
            assert_eq!(editor.app.page, Page::Edit);
            assert_eq!(editor.app.inspector_open, inspector);
            editor.click(effects);
            assert!(!editor.app.effects_library_open);
            editor.click(effects);
            assert!(editor.app.effects_library_open);
            let closed = editor.click(pool);
            assert!(!editor.app.effects_library_open);
            assert!(find(&closed, |text, _| text == "M...").is_some());
            assert_eq!(editor.app.page, Page::Edit);
            assert_eq!(editor.app.inspector_open, inspector);
            editor.click(effects);
            editor.app.do_act(&editor.ctx, Act::ResetLayout);
            assert!(!editor.app.effects_library_open);
            assert_eq!(editor.app.inspector_open, inspector);
        }
        Ok(())
    }

    #[test]
    fn the_dock_is_clipped_and_has_no_orphaned_pool_controls_or_duplicate_ids(
    ) -> Result<(), &'static str> {
        let _english = english();
        for inspector in [false, true] {
            for narrow in [false, true] {
                let mut editor = Editor::new(true);
                editor.app.effects_library_open = true;
                editor.app.inspector_open = inspector;
                if narrow {
                    editor.scale = 0.4;
                    let panels = &mut editor.app.extra.prefs.panels;
                    panels.set_edit_column(0, 0.0);
                    panels.set_edit_column(1, 0.0);
                }
                let right =
                    f32::from(editor.app.extra.prefs.panels.edit_columns[1]) * crate::chrome::R;
                let check = |frame: &FullOutput| {
                    visit_text(frame, |text, bounds, clip| {
                        assert!(
                            !text.contains("Second use") && !text.contains("First use"),
                            "inspector={inspector} narrow={narrow}: {text}"
                        );
                        if bounds.center().y > 83.0 && bounds.left() < right {
                            assert!(!matches!(text, "Bins" | "Master" | "Smart Bins" | "M..."));
                        }
                        if matches!(
                            text,
                            "Effects Library" | "Search" | "Video Transitions" | "Cross Dissolve"
                        ) {
                            assert!(clip.right() <= right + 0.5, "{text} scissor: {clip:?}");
                            assert!(bounds.right() <= right + 0.5, "{text} bounds: {bounds:?}");
                        }
                    });
                };
                let mut frame = editor.frame(vec![]);
                for _ in 0..3 {
                    check(&frame);
                    for label in [
                        "Effects Library",
                        "Search",
                        "Video Transitions",
                        "Cross Dissolve",
                    ] {
                        assert!(find(&frame, |text, _| text == label).is_some(), "{label}");
                    }
                    assert_eq!(
                        find(&frame, |text, _| text == "Generators").is_none(),
                        narrow
                    );
                    frame = editor.frame(vec![]);
                }
                for x in [26, 248, 461, 543, 601, 659] {
                    let pool_control = editor.root.with(("edit_tb2", x, 120_i32));
                    assert!(editor.ctx.read_response(pool_control).is_none());
                }
                assert!(editor
                    .ctx
                    .read_response(Id::new(("panel divider", "edit pool")))
                    .is_some());
                if narrow {
                    assert!((editor.ctx.pixels_per_point() - 0.4).abs() < 0.01);
                    let picker = find(&frame, |text, _| text == "Video Transitions")
                        .ok_or("missing compact category picker")?;
                    frame = editor.click(picker);
                }
                let generators = find(&frame, |text, _| text == "Generators")
                    .ok_or("missing Generators category")?;
                let generators = editor.click(generators);
                check(&generators);
                assert!(find(&generators, |text, _| text == "Solid Color").is_some());
            }
        }
        Ok(())
    }

    #[test]
    fn search_filters_across_categories_and_owns_the_keyboard() -> Result<(), &'static str> {
        let _english = english();
        let mut editor = Editor::new(true);
        editor.app.effects_library_open = true;
        let frame = editor.frame(vec![]);
        let search = find(&frame, |text, _| text == "Search").ok_or("missing Search field")?;
        editor.click(search);
        assert!(editor.ctx.egui_wants_keyboard_input());

        let before = editor.app.session.revision();
        let playhead = editor.app.playhead;
        editor.frame(
            [
                Key::Space,
                Key::J,
                Key::K,
                Key::L,
                Key::D,
                Key::B,
                Key::Delete,
                Key::Backspace,
            ]
            .map(key)
            .into(),
        );
        assert!(!editor.app.playing);
        assert_eq!(editor.app.playhead, playhead);
        assert_eq!(
            editor.app.session.revision(),
            before,
            "keyboard shortcuts mutated the project"
        );
        let filtered = editor.frame(vec![Event::Text("bLuR".into())]);
        assert_eq!(editor.app.effects_library.query, "bLuR");
        for result in [
            "Blur Dissolve  ·  Video Transitions",
            "Gaussian Blur  ·  Picasso FX",
        ] {
            assert!(
                find(&filtered, |text, _| text == result).is_some(),
                "{result}"
            );
        }
        assert!(find(&filtered, |text, _| text == "Cross Dissolve").is_none());

        let clear = find(&filtered, |text, _| text == "×").ok_or("missing Clear button")?;
        let cleared = editor.click(clear);
        assert!(editor.app.effects_library.query.is_empty());
        assert!(find(&cleared, |text, _| text == "Cross Dissolve").is_some());
        editor.frame(vec![Event::Text("echo".into())]);
        editor.frame(vec![key(Key::Escape)]);
        assert!(editor.app.effects_library.query.is_empty());
        assert!(!editor.ctx.egui_wants_keyboard_input());
        Ok(())
    }

    #[test]
    fn applying_each_kind_from_the_dock_uses_one_undo_step(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let _english = english();
        let mut editor = Editor::new(true);
        editor.app.effects_library_open = true;
        editor.app.run(Command::AddTrack {
            kind: TrackKind::Audio,
        });
        let selected = editor.app.selection.ok_or("missing demo selection")?;
        let cut = editor
            .app
            .nearest_cut(TrackKind::Video, false)
            .ok_or("missing demo cut")?;
        for (label, action) in [
            (
                "Cross Dissolve",
                Act::AddTransition(TransitionKind::CrossDissolve),
            ),
            ("Gaussian Blur", Act::AddEffect(EffectKind::GaussianBlur)),
            ("Neon Night", Act::ApplyLook(2)),
            ("Text+", Act::AddTextPlus),
            (
                "Four-Color Gradient",
                Act::AddGenerator(GeneratorKind::FourColorGradient),
            ),
            ("Reverb", Act::AddAudioEffect(AudioEffectKind::Reverb)),
        ] {
            editor.app.selection = Some(selected);
            let before = editor.app.project().to_json()?;
            let step = editor.app.session.revision();
            editor.choose(label)?;
            assert_eq!(
                editor.app.session.revision(),
                step + 1,
                "{label}: {}",
                editor.app.status.text
            );
            assert_eq!(editor.app.page, Page::Edit);
            assert!(!editor.app.inspector_open);
            let project = editor.app.project();
            let effects = &project
                .clip(selected)
                .ok_or("selected clip disappeared")?
                .1
                .effects;
            match action {
                Act::AddTransition(kind) => assert_eq!(
                    project
                        .clip(cut)
                        .and_then(|(_, clip)| clip.transition.map(|t| t.kind)),
                    Some(kind)
                ),
                Act::AddEffect(kind) => assert_eq!(effects.first().map(|e| e.kind), Some(kind)),
                Act::ApplyLook(index) => assert_eq!(effects, &LOOKS[index].effect_list()),
                Act::AddTextPlus => {
                    let clip = editor
                        .app
                        .selection
                        .and_then(|id| project.clip(id))
                        .ok_or("missing title")?
                        .1;
                    assert!(matches!(
                        project.media(clip.media).map(|m| &m.source),
                        Some(MediaSource::Title(_))
                    ));
                }
                Act::AddGenerator(kind) => assert!(matches!(
                    project.media.last().map(|m| &m.source),
                    Some(MediaSource::Generator { pattern, .. }) if *pattern == kind
                )),
                Act::AddAudioEffect(kind) => assert!(project.tracks.iter().any(|t| {
                    t.kind == TrackKind::Audio
                        && t.fx.chain.iter().any(|effect| effect.kind == kind)
                })),
                _ => return Err(format!("unexpected test action for {label}").into()),
            }
            editor.app.undo();
            assert_eq!(editor.app.project().to_json()?, before, "{label}");
        }
        Ok(())
    }

    #[test]
    fn the_dock_uses_normal_fallbacks_when_there_is_no_selection() -> Result<(), String> {
        let _english = english();
        let mut editor = Editor::new(false);
        editor
            .app
            .replace_session(Project::new("No clips or sound tracks"), None);
        editor.app.effects_library_open = true;
        for (label, hint) in [
            ("Cross Dissolve", "there is no cut"),
            ("Gaussian Blur", "select a clip first"),
            ("Neon Night", "select a clip first"),
            ("Reverb", "there is no sound track"),
        ] {
            editor.choose(label)?;
            assert!(
                editor.app.status.text.contains(hint),
                "{label}: {}",
                editor.app.status.text
            );
            assert_eq!(editor.app.session.revision(), 0);
        }
        for label in ["Four-Color Gradient", "Text+"] {
            assert!(editor.app.selection.is_none());
            editor.choose(label)?;
        }
        assert_eq!(editor.app.session.revision(), 2);
        Ok(())
    }
}
