//! Content Browser: project folders, thumbnails, drag & drop, import/create.

use super::{Editor, LogLevel};
use crate::assets::{self, AssetKind};
use crate::i18n::t;
use crate::theme::{self, ACCENT};
use egui::{Color32, RichText};
use std::path::{Path, PathBuf};

impl Editor {
    fn thumb(&mut self, ctx: &egui::Context, p: &Path) -> Option<egui::TextureHandle> {
        if let Some(t) = self.thumbs.get(p) {
            return Some(t.clone());
        }
        let img = image::open(p).ok()?.thumbnail(96, 96).to_rgba8();
        let size = [img.width() as usize, img.height() as usize];
        let tex = ctx.load_texture(
            p.display().to_string(),
            egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()),
            egui::TextureOptions::LINEAR,
        );
        self.thumbs.insert(p.to_path_buf(), tex.clone());
        Some(tex)
    }

    pub fn content_ui(&mut self, ui: &mut egui::Ui) {
        let root = self.project_root();
        if !root.exists() {
            ui.label(
                RichText::new(t(
                    "Save the scene to create its project folder.",
                    "Proje klasörü için sahneyi kaydet.",
                ))
                .weak(),
            );
            if ui.button(t("💾 Save", "💾 Kaydet")).clicked() {
                self.save();
            }
            return;
        }
        let dir = root.join(&self.cb_dir);
        // Toolbar.
        ui.horizontal(|ui| {
            ui.menu_button(RichText::new(t("+ Create", "+ Oluştur")).strong(), |ui| {
                if ui.button(t("📁 Folder", "📁 Klasör")).clicked() {
                    self.cb_new = Some((0, t("New Folder", "Yeni Klasör").into()));
                    ui.close_menu();
                }
                if ui.button(t("📜 Rhai Script", "📜 Rhai Betiği")).clicked() {
                    self.cb_new = Some((1, "NewScript".into()));
                    ui.close_menu();
                }
            });
            if ui
                .button(t("⬇ Import from device…", "⬇ Cihazdan içe aktar…"))
                .on_hover_text(t(
                    "Textures (png/jpg), models (glTF/GLB), audio (wav/ogg/mp3/flac), animations, scripts — or drag files onto the window.",
                    "Doku (png/jpg), model (glTF/GLB), ses (wav/ogg/mp3/flac), animasyon, betik — ya da dosyaları pencereye sürükle.",
                ))
                .clicked()
            {
                let picked = rfd::FileDialog::new()
                    .set_title(t("Import assets", "Varlık içe aktar"))
                    .add_filter(t("All assets", "Tüm varlıklar"), &["png", "jpg", "jpeg", "gltf", "glb", "wav", "ogg", "mp3", "flac", "json", "rhai"])
                    .add_filter(t("Textures", "Dokular"), &["png", "jpg", "jpeg"])
                    .add_filter(t("3D models", "3D modeller"), &["gltf", "glb"])
                    .add_filter(t("Audio", "Ses"), &["wav", "ogg", "mp3", "flac"])
                    .pick_files();
                match picked {
                    Some(files) => self.import_files(&files),
                    None => self.cb_import = Some(String::new()),
                }
            }
            if ui
                .button("⟳")
                .on_hover_text(t("Refresh", "Yenile"))
                .clicked()
            {
                self.thumbs.clear();
                if let Some(r) = &self.renderer {
                    r.lock().unwrap().invalidate_textures();
                }
            }
            ui.separator();
            // Breadcrumb.
            if ui.link(RichText::new("Assets").strong()).clicked() {
                self.cb_dir = PathBuf::new();
            }
            let mut acc = PathBuf::new();
            for c in self.cb_dir.clone().components() {
                acc.push(c);
                ui.label("›");
                if ui.link(c.as_os_str().to_string_lossy()).clicked() {
                    self.cb_dir = acc.clone();
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.cb_search)
                        .hint_text(t("🔍 Search assets", "🔍 Varlık ara"))
                        .desired_width(160.0),
                );
            });
        });
        // Create / import inline forms.
        if let Some((kind, mut name)) = self.cb_new.take() {
            let mut keep = true;
            ui.horizontal(|ui| {
                ui.label(if kind == 0 {
                    t("Folder name", "Klasör adı")
                } else {
                    t("Script name", "Betik adı")
                });
                let r = ui.text_edit_singleline(&mut name);
                if ui.button(t("Create", "Oluştur")).clicked()
                    || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                {
                    let safe = super::sanitize(&name);
                    let res = if kind == 0 {
                        std::fs::create_dir_all(dir.join(&safe))
                    } else {
                        std::fs::write(
                            dir.join(format!("{}.rhai", safe.replace(' ', "_"))),
                            "fn start() {\n}\n\nfn update(dt) {\n}\n",
                        )
                    };
                    if let Err(e) = res {
                        self.log(LogLevel::Error, e.to_string());
                    }
                    keep = false;
                }
                if ui.button("✖").clicked() {
                    keep = false;
                }
            });
            if keep {
                self.cb_new = Some((kind, name));
            }
        }
        if let Some(mut src) = self.cb_import.take() {
            let mut keep = true;
            ui.horizontal(|ui| {
                ui.label(t("File to import", "İçe aktarılacak dosya"));
                ui.add(
                    egui::TextEdit::singleline(&mut src)
                        .hint_text("C:\\…\\texture.png")
                        .desired_width(300.0),
                );
                if ui.button(t("Import", "Aktar")).clicked() {
                    let p = PathBuf::from(src.trim().trim_matches('"'));
                    match p.file_name() {
                        Some(n) => match std::fs::copy(&p, dir.join(n)) {
                            Ok(_) => self.log(
                                LogLevel::Info,
                                format!("{} {}", t("Imported", "Aktarıldı:"), n.to_string_lossy()),
                            ),
                            Err(e) => self.log(LogLevel::Error, e.to_string()),
                        },
                        None => self.log(LogLevel::Error, "invalid path".into()),
                    }
                    keep = false;
                }
                if ui.button("✖").clicked() {
                    keep = false;
                }
            });
            if keep {
                self.cb_import = Some(src);
            }
        }
        ui.separator();

        let ctx = ui.ctx().clone();
        egui::SidePanel::left("cb_tree")
            .resizable(true)
            .default_width(150.0)
            .frame(egui::Frame::NONE)
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        let sel = self.cb_dir.as_os_str().is_empty();
                        if ui.selectable_label(sel, "📁 Assets").clicked() {
                            self.cb_dir = PathBuf::new();
                        }
                        self.folder_tree(ui, &root, &root, 1);
                    });
            });

        let q = self.cb_search.to_lowercase();
        let items: Vec<PathBuf> = if q.is_empty() {
            assets::list_dir(&dir)
        } else {
            let mut all = vec![];
            collect(&root, &mut all, 4);
            all.into_iter()
                .filter(|p| {
                    p.file_name()
                        .map_or(false, |n| n.to_string_lossy().to_lowercase().contains(&q))
                })
                .collect()
        };
        let mut action: Option<(u8, PathBuf)> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
                    for p in items {
                        let kind = AssetKind::of(&p);
                        let name = p
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let selected = self.cb_selected.as_ref() == Some(&p);
                        let thumb = if kind == AssetKind::Texture {
                            self.thumb(&ctx, &p)
                        } else {
                            None
                        };
                        let resp = {
                            {
                                let (rect, resp) = ui.allocate_exact_size(
                                    egui::vec2(86.0, 100.0),
                                    egui::Sense::click_and_drag(),
                                );
                                if resp.dragged() {
                                    resp.dnd_set_drag_payload(p.clone());
                                }
                                let bg = if selected {
                                    ACCENT.linear_multiply(0.35)
                                } else {
                                    theme::BG2
                                };
                                ui.painter().rect_filled(rect, 6.0, bg);
                                let icon_rect = egui::Rect::from_min_size(
                                    rect.min + egui::vec2(11.0, 6.0),
                                    egui::vec2(64.0, 64.0),
                                );
                                if let Some(tex) = &thumb {
                                    ui.painter().image(
                                        tex.id(),
                                        icon_rect,
                                        egui::Rect::from_min_max(
                                            egui::pos2(0.0, 0.0),
                                            egui::pos2(1.0, 1.0),
                                        ),
                                        Color32::WHITE,
                                    );
                                } else {
                                    let c = match kind {
                                        AssetKind::Folder => Color32::from_rgb(214, 170, 82),
                                        AssetKind::Script => Color32::from_rgb(120, 190, 240),
                                        AssetKind::Scene => ACCENT,
                                        AssetKind::Prefab => Color32::from_rgb(130, 210, 160),
                                        AssetKind::Model => Color32::from_rgb(190, 160, 230),
                                        AssetKind::Audio => Color32::from_rgb(240, 200, 90),
                                        AssetKind::Animation => Color32::from_rgb(120, 220, 210),
                                        _ => theme::BONE,
                                    };
                                    ui.painter().text(
                                        icon_rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        kind.icon(),
                                        egui::FontId::proportional(38.0),
                                        c,
                                    );
                                }
                                let label: String = if name.chars().count() > 13 {
                                    format!("{}…", name.chars().take(12).collect::<String>())
                                } else {
                                    name.clone()
                                };
                                ui.painter().text(
                                    rect.center_bottom() - egui::vec2(0.0, 14.0),
                                    egui::Align2::CENTER_CENTER,
                                    label,
                                    egui::FontId::proportional(11.0),
                                    theme::BONE,
                                );
                                resp
                            }
                        }
                        .on_hover_text(&name);
                        if resp.clicked() {
                            self.cb_selected = Some(p.clone());
                        }
                        if resp.double_clicked() {
                            action = Some((0, p.clone()));
                        }
                        resp.context_menu(|ui| {
                            match kind {
                                AssetKind::Texture => {
                                    if ui
                                        .button(t(
                                            "Apply to selected object",
                                            "Seçili nesneye uygula",
                                        ))
                                        .clicked()
                                    {
                                        action = Some((1, p.clone()));
                                        ui.close_menu();
                                    }
                                }
                                AssetKind::Script => {
                                    if ui
                                        .button(t(
                                            "Attach to selected object",
                                            "Seçili nesneye ekle",
                                        ))
                                        .clicked()
                                    {
                                        action = Some((1, p.clone()));
                                        ui.close_menu();
                                    }
                                }
                                AssetKind::Prefab => {
                                    if ui.button(t("Instantiate", "Sahneye ekle")).clicked() {
                                        action = Some((0, p.clone()));
                                        ui.close_menu();
                                    }
                                }
                                AssetKind::Scene | AssetKind::Folder => {
                                    if ui.button(t("Open", "Aç")).clicked() {
                                        action = Some((0, p.clone()));
                                        ui.close_menu();
                                    }
                                }
                                _ => {}
                            }
                            if ui.button(t("Copy path", "Yolu kopyala")).clicked() {
                                ui.ctx().copy_text(p.display().to_string());
                                ui.close_menu();
                            }
                            if ui
                                .button(RichText::new(t("Delete", "Sil")).color(Color32::LIGHT_RED))
                                .clicked()
                            {
                                action = Some((9, p.clone()));
                                ui.close_menu();
                            }
                        });
                    }
                });
            });
        if let Some((a, p)) = action {
            let kind = AssetKind::of(&p);
            match (a, kind) {
                (0, AssetKind::Folder) => {
                    self.cb_dir = p
                        .strip_prefix(&root)
                        .map(|x| x.to_path_buf())
                        .unwrap_or_default()
                }
                (0, AssetKind::Scene) => self.open_scene(p),
                (0, AssetKind::Prefab) => self.instantiate_prefab(&p),
                (0, AssetKind::Audio) => {
                    if let Err(e) = self.audio.play(&p, 1.0, None) {
                        self.log(LogLevel::Warn, e);
                    }
                }
                (1, AssetKind::Audio) => {
                    let rel = p
                        .strip_prefix(&root)
                        .map(|x| x.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_default();
                    if let Some(id) = self.selected {
                        self.push_undo();
                        if let Some(e) = self.scene.get_mut(id) {
                            e.audio = Some(crate::scene::AudioSource {
                                clip: rel,
                                ..Default::default()
                            });
                        }
                    }
                }
                (0, AssetKind::Animation) | (1, AssetKind::Animation) => {
                    let rel = p
                        .strip_prefix(&root)
                        .map(|x| x.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_default();
                    if a == 1 {
                        if let Some(id) = self.selected {
                            self.push_undo();
                            if let Some(e) = self.scene.get_mut(id) {
                                e.animator = Some(crate::scene::Animator {
                                    clip: rel,
                                    ..Default::default()
                                });
                            }
                        }
                    } else if let Ok(c) = crate::animation::Clip::load(&p) {
                        self.anim.clip = c;
                        self.anim.path = Some(rel);
                        self.open_tab = Some(super::Tab::Animation);
                    }
                }
                (0, AssetKind::Model) | (1, AssetKind::Model) => {
                    let rel = p
                        .strip_prefix(&root)
                        .map(|x| x.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_default();
                    let name = p
                        .file_stem()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    self.add_model(&rel, &name, None, None);
                }
                (0, AssetKind::Script) | (1, AssetKind::Script) | (1, AssetKind::Texture) => {
                    self.apply_asset(&p)
                }
                (0, AssetKind::Texture) => self.apply_asset(&p),
                (9, _) => {
                    let r = if p.is_dir() {
                        std::fs::remove_dir_all(&p)
                    } else {
                        std::fs::remove_file(&p)
                    };
                    if let Err(e) = r {
                        self.log(LogLevel::Error, e.to_string());
                    }
                    self.thumbs.remove(&p);
                }
                _ => {}
            }
        }
    }

    /// Import files from the device into the project (dialog or OS drag & drop).
    pub fn import_files(&mut self, files: &[PathBuf]) {
        let root = self.project_root();
        crate::assets::ensure_project(&root);
        for r in crate::assets::import(&root, files) {
            match r {
                Ok(rel) => self.log(
                    LogLevel::Info,
                    format!("{} {rel}", t("Imported", "İçe aktarıldı:")),
                ),
                Err(e) => self.log(LogLevel::Error, e),
            }
        }
        self.thumbs.clear();
        if let Some(r) = &self.renderer {
            r.lock().unwrap().invalidate_textures();
        }
        self.open_tab = Some(super::Tab::Content);
    }

    fn apply_asset(&mut self, p: &Path) {
        let Some(id) = self.selected else {
            self.log(
                LogLevel::Warn,
                t("Select an object first.", "Önce bir nesne seç.").into(),
            );
            return;
        };
        let root = self.project_root();
        self.push_undo();
        match AssetKind::of(p) {
            AssetKind::Texture => {
                let rel = p
                    .strip_prefix(&root)
                    .map(|x| x.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_else(|_| p.display().to_string());
                if let Some(e) = self.scene.get_mut(id) {
                    e.material.texture = rel;
                    e.color = [1.0, 1.0, 1.0];
                }
            }
            AssetKind::Script => {
                if let (Ok(code), Some(e)) = (std::fs::read_to_string(p), self.scene.get_mut(id)) {
                    e.script = code;
                }
            }
            _ => {}
        }
    }

    fn folder_tree(&mut self, ui: &mut egui::Ui, dir: &Path, root: &Path, depth: usize) {
        for p in assets::list_dir(dir).into_iter().filter(|p| p.is_dir()) {
            let rel = p
                .strip_prefix(root)
                .map(|x| x.to_path_buf())
                .unwrap_or_default();
            let sel = self.cb_dir == rel;
            ui.horizontal(|ui| {
                ui.add_space(depth as f32 * 10.0);
                let name = p.file_name().unwrap().to_string_lossy().to_string();
                if ui.selectable_label(sel, format!("📁 {name}")).clicked() {
                    self.cb_dir = rel.clone();
                }
            });
            if depth < 4 {
                self.folder_tree(ui, &p, root, depth + 1);
            }
        }
    }
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>, depth: u32) {
    for p in assets::list_dir(dir) {
        if p.is_dir() && depth > 0 {
            collect(&p, out, depth - 1);
        }
        out.push(p);
    }
}
