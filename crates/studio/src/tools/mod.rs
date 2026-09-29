pub mod calibrators;
pub mod preview;
pub mod rigs;

/// A side panel section's heading colour: it stands out from the controls
/// under it (and from the blue of a selection and the orange of a warning).
pub fn section_colour(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        egui::Color32::from_rgb(110, 205, 190)
    } else {
        egui::Color32::from_rgb(20, 115, 105)
    }
}

/// A side panel section: its heading, in [`section_colour`], opens and shuts
/// what is under it. Shut until first opened; after that it opens as it was
/// left, run to run (`open_sections` in `settings.toml`). `id` tells it from
/// a same-titled section elsewhere, and a section with the same `id` on two
/// tabs opens and shuts as one. The body's value, when open.
pub fn section<R>(ui: &mut egui::Ui, id: &str, title: impl Into<String>, add: impl FnOnce(&mut egui::Ui) -> R) -> Option<R> {
    let title = section_title(ui, title);
    section_with(ui, id, title, false, false, add)
}

/// A section heading's text: large, in [`section_colour`].
pub fn section_title(ui: &egui::Ui, title: impl Into<String>) -> egui::RichText {
    egui::RichText::new(title).heading().color(section_colour(ui))
}

/// [`section`] with its heading's text as given (a warning's colour), open
/// until shut when `open` (what a click just asked to see).
pub fn section_rich<R>(ui: &mut egui::Ui, id: &str, title: egui::RichText, open: bool, add: impl FnOnce(&mut egui::Ui) -> R) -> Option<R> {
    section_with(ui, id, title, open, false, add)
}

/// The one section: open by default when `open` or when it was left open
/// last run; forced open this frame when `open_now` (a note being pinned).
/// A click on the heading is remembered.
pub fn section_with<R>(
    ui: &mut egui::Ui,
    id: &str,
    title: egui::RichText,
    open: bool,
    open_now: bool,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<R> {
    let salt = ("panel_section", id);
    let key = ui.make_persistent_id(salt);
    let remembered = open_sections::is_open(id);
    let shown = egui::CollapsingHeader::new(title)
        .id_salt(salt)
        .default_open(open || remembered)
        .open(open_now.then_some(true))
        .show(ui, add);
    if shown.header_response.clicked() {
        let now = egui::collapsing_header::CollapsingState::load(ui.ctx(), key).is_some_and(|s| s.is_open());
        open_sections::set(id, now);
    }
    shown.body_returned
}

/// Which sections were left open: read from `settings.toml` once, written
/// back on every heading click.
mod open_sections {
    use std::collections::BTreeSet;
    use std::sync::{Mutex, OnceLock};

    fn set_of() -> &'static Mutex<BTreeSet<String>> {
        static OPEN: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();
        OPEN.get_or_init(|| {
            let (settings, _) = crate::settings::Settings::load(&crate::settings::home());
            Mutex::new(settings.open_sections.into_iter().collect())
        })
    }

    /// `GS_STUDIO_OPEN=<id,...>` (or `all`): sections open for this run,
    /// never saved -- for screenshots.
    fn opened_for_the_run(id: &str) -> bool {
        static RUN: OnceLock<Vec<String>> = OnceLock::new();
        let run = RUN.get_or_init(|| {
            std::env::var("GS_STUDIO_OPEN").map(|v| v.split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect()).unwrap_or_default()
        });
        run.iter().any(|s| s == id || s == "all")
    }

    pub fn is_open(id: &str) -> bool {
        opened_for_the_run(id) || set_of().lock().unwrap_or_else(|e| e.into_inner()).contains(id)
    }

    pub fn set(id: &str, open: bool) {
        let list: Vec<String> = {
            let mut set = set_of().lock().unwrap_or_else(|e| e.into_inner());
            let changed = if open { set.insert(id.to_owned()) } else { set.remove(id) };
            if !changed {
                return;
            }
            set.iter().cloned().collect()
        };
        if let Some(e) = crate::settings::remember(|s| s.open_sections = list) {
            eprintln!("gs-studio: {e}");
        }
    }
}

/// A group's label inside a section (Niffy's LEGS, PELVIS): small capitals
/// in a quieter shade of the section colour, a little space above.
pub fn group(ui: &mut egui::Ui, title: &str) {
    ui.add_space(4.0);
    ui.label(group_text(ui, title));
}

/// [`group`]'s text, for a label at the start of a row.
pub fn group_text(ui: &egui::Ui, title: &str) -> egui::RichText {
    egui::RichText::new(title.to_uppercase()).small().strong().color(section_colour(ui).gamma_multiply(0.8))
}

/// A number that may be unset: shows `default` greyed until touched, and
/// right-click puts it back to the default. True when it changed.
pub fn opt_drag(ui: &mut egui::Ui, value: &mut Option<f32>, default: f32, range: std::ops::RangeInclusive<f32>, speed: f64) -> bool {
    let before = *value;
    let mut v = value.unwrap_or(default);
    let unset = value.is_none();
    let response = ui
        .scope(|ui| {
            if unset {
                ui.visuals_mut().override_text_color = Some(ui.visuals().weak_text_color());
            }
            ui.add(egui::DragValue::new(&mut v).range(range).speed(speed).fixed_decimals(2))
        })
        .inner;
    if response.changed() {
        *value = Some(v);
    }
    let hint = if unset { format!("{default:.2}, the default: drag to set") } else { format!("Set; the default is {default:.2} (right-click)") };
    let response = response.on_hover_text(hint);
    if !unset {
        response.context_menu(|ui| {
            if ui.button("Back to the default").clicked() {
                *value = None;
                ui.close();
            }
        });
    }
    *value != before
}

/// What a click on a note asked for.
pub enum NoteAct {
    /// Put the view back to where it was seen.
    Show,
    Done(bool),
    Delete,
    /// Post this reply under it.
    Reply(String),
}

/// One note as a list shows it, whatever it is pinned to.
pub struct NoteView<'a> {
    pub id: u32,
    pub text: &'a str,
    pub by: &'a str,
    pub made: &'a str,
    /// Where and when it was seen: "leg_r 95%, kneel", "Campfire 1, 23:00".
    pub place: String,
    pub done: bool,
    pub replies: &'a [crate::notes::Reply],
    /// The one last shown.
    pub focused: bool,
}

/// A note in a notes list (Niffy's review notes): its number and text (a
/// click shows it), who and when and where under it, Show / Reply / Done /
/// Delete, its replies indented, and the reply box when `draft` is on this
/// note.
pub fn note_ui(ui: &mut egui::Ui, note: NoteView, draft: &mut Option<(u32, String)>) -> Option<NoteAct> {
    let mut act = None;
    let label = format!("#{} {}", note.id, note.text);
    let text = if note.done { egui::RichText::new(label).weak().strikethrough() } else { egui::RichText::new(label) };
    if ui.selectable_label(note.focused, text).on_hover_text("Show it: where and when it was seen").clicked() {
        act = Some(NoteAct::Show);
    }
    ui.horizontal_wrapped(|ui| {
        let mut who = if note.by.is_empty() { String::new() } else { format!("{}, ", note.by) };
        who.push_str(note.made);
        if !note.place.is_empty() {
            who.push_str(&format!(" - {}", note.place));
        }
        ui.weak(who);
        if ui.small_button("Show").on_hover_text("Where and when it was seen, its pin ringed").clicked() {
            act = Some(NoteAct::Show);
        }
        if ui.small_button("Reply").clicked() {
            *draft = Some((note.id, String::new()));
        }
        let mut done = note.done;
        if ui.checkbox(&mut done, "Done").on_hover_text("Dealt with: kept in the list, off the pins").changed() {
            act = Some(NoteAct::Done(done));
        }
        if ui.small_button("Delete").clicked() {
            act = Some(NoteAct::Delete);
        }
    });
    for reply in note.replies {
        ui.horizontal_wrapped(|ui| {
            ui.add_space(16.0);
            if !reply.by.is_empty() {
                ui.strong(&reply.by);
            }
            ui.label(&reply.text);
            ui.weak(&reply.made);
        });
    }
    if let Some((id, text)) = draft.as_mut().filter(|(id, _)| *id == note.id) {
        let (mut send, mut cancel) = (false, false);
        ui.horizontal_wrapped(|ui| {
            ui.add_space(16.0);
            let edit = ui.add(egui::TextEdit::singleline(text).desired_width(220.0).hint_text("reply"));
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            send = (ui.small_button("Send").clicked() || enter) && !text.trim().is_empty();
            cancel = ui.small_button("Cancel").clicked();
        });
        if send {
            act = Some(NoteAct::Reply(text.trim().to_owned()));
            let _ = id;
            *draft = None;
        } else if cancel {
            *draft = None;
        }
    }
    ui.add_space(2.0);
    act
}

#[cfg(test)]
mod tests {
    /// Every non-ASCII character the UI code can put on screen, whether
    /// written literally or as a `\u{..}` escape.
    fn ui_glyphs() -> Vec<(String, char)> {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files: Vec<_> = std::fs::read_dir(src.join("tools")).unwrap().map(|e| e.unwrap().path()).collect();
        files.push(src.join("app.rs"));
        let mut found = Vec::new();
        for path in files {
            let text = std::fs::read_to_string(&path).unwrap();
            // This module's own tests talk ABOUT glyphs.
            let text = text.split("#[cfg(test)]").next().unwrap();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            for line in text.lines() {
                let code = line.split("//").next().unwrap();
                found.extend(code.chars().filter(|c| !c.is_ascii()).map(|c| (name.clone(), c)));
                for escape in code.split("\\u{").skip(1) {
                    let hex = escape.split('}').next().unwrap();
                    let c = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32).unwrap();
                    found.push((name.clone(), c));
                }
            }
        }
        found
    }

    /// Non-ASCII glyphs a human has SEEN render in this app's (egui default)
    /// fonts. U+25CF / U+25CB are not here: as the rig editor's joint
    /// markers they drew as empty boxes, and are painted shapes now.
    /// `has_glyph` cannot stand in for eyes -- it answers true for those two.
    const SEEN_TO_RENDER: [char; 2] = ['\u{26A0}', '\u{2691}'];

    #[test]
    fn ui_code_only_uses_glyphs_known_to_render() {
        for (file, c) in ui_glyphs() {
            assert!(
                SEEN_TO_RENDER.contains(&c),
                "{file}: U+{:04X} is not known to render in the default fonts -- use ASCII or paint a shape",
                c as u32
            );
        }
    }
}
