//! Asset Store: free CC0 assets pulled straight into the project.
//!
//! * Poly Haven (<https://polyhaven.com>, API `api.polyhaven.com`): textures and glTF models.
//! * ambientCG (<https://ambientcg.com>, API v2): PBR materials (color map is imported).
//! * Links to other free libraries without an API (Kenney, Quaternius, OpenGameArt, …).
//!
//! All network work runs on background threads; the same blocking helpers are used by the
//! agent tools `asset_search` / `asset_import`.

use crate::i18n::t;
use crate::theme::{self, ACCENT};
use egui::{Color32, RichText};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
pub enum Source {
    PolyHaven,
    AmbientCg,
}

impl Source {
    pub fn parse(s: &str) -> Source {
        if s.to_lowercase().contains("ambient") {
            Source::AmbientCg
        } else {
            Source::PolyHaven
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Source::PolyHaven => "Poly Haven",
            Source::AmbientCg => "ambientCG",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize)]
pub enum Kind {
    Textures,
    Models,
}

impl Kind {
    pub fn parse(s: &str) -> Kind {
        if s.to_lowercase().starts_with("model") {
            Kind::Models
        } else {
            Kind::Textures
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Item {
    pub source: Source,
    pub kind: Kind,
    pub id: String,
    pub name: String,
    pub thumb: String,
    pub tags: Vec<String>,
    pub popularity: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Imported {
    /// Project-relative path (texture image or .gltf file).
    pub path: String,
    pub kind: Kind,
    /// Real-world size for models (becomes the entity scale).
    pub extents: Option<[f32; 3]>,
    pub color: Option<[f32; 3]>,
}

/// Poly Haven API base (overridable with FORGE_POLYHAVEN_API for mirrors/testing).
fn ph_api() -> String {
    std::env::var("FORGE_POLYHAVEN_API").unwrap_or_else(|_| "https://api.polyhaven.com".into())
}

const UA: &str = concat!(
    "Forge/",
    env!("CARGO_PKG_VERSION"),
    " (github.com/burkiuze/Forge)"
);

fn get_bytes(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let r = crate::ai::client::builder_for(url)
        .timeout_connect(std::time::Duration::from_secs(15))
        .timeout_read(std::time::Duration::from_secs(90))
        .redirects(8)
        .build()
        .get(url)
        .set("User-Agent", UA)
        .call()
        .map_err(|e| format!("{url}: {e}"))?;
    let mut buf = vec![];
    r.into_reader()
        .take(limit)
        .read_to_end(&mut buf)
        .map_err(|e| e.to_string())?;
    Ok(buf)
}

fn get_json(url: &str) -> Result<Value, String> {
    serde_json::from_slice(&get_bytes(url, 64 << 20)?).map_err(|e| e.to_string())
}

// ------------------------------------------------------------ search

/// Poly Haven returns the whole catalog for a type; we filter locally.
pub fn polyhaven_list(kind: Kind) -> Result<Vec<Item>, String> {
    let t = if kind == Kind::Models {
        "models"
    } else {
        "textures"
    };
    parse_polyhaven(kind, &get_json(&format!("{}/assets?t={t}", ph_api()))?)
}

pub fn parse_polyhaven(kind: Kind, v: &Value) -> Result<Vec<Item>, String> {
    let obj = v.as_object().ok_or("unexpected Poly Haven response")?;
    let mut items: Vec<Item> = obj
        .iter()
        .map(|(id, a)| Item {
            source: Source::PolyHaven,
            kind,
            id: id.clone(),
            name: a["name"].as_str().unwrap_or(id).to_string(),
            thumb: a["thumbnail_url"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| {
                    format!(
                        "https://cdn.polyhaven.com/asset_img/thumbs/{id}.png?width=256&height=256"
                    )
                }),
            tags: a["tags"]
                .as_array()
                .map(|t| {
                    t.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            popularity: a["download_count"].as_u64().unwrap_or(0),
        })
        .collect();
    items.sort_by(|a, b| b.popularity.cmp(&a.popularity));
    Ok(items)
}

pub fn ambientcg_search(query: &str, offset: usize) -> Result<Vec<Item>, String> {
    let q = crate::ai::web::encode(query);
    let v = get_json(&format!(
        "https://ambientcg.com/api/v2/full_json?type=Material&include=previewData,tagData&sort=Popular&limit=60&offset={offset}&q={q}"
    ))?;
    Ok(parse_ambientcg(&v))
}

pub fn parse_ambientcg(v: &Value) -> Vec<Item> {
    let found = v["foundAssets"].as_array().cloned().unwrap_or_default();
    found
        .iter()
        .filter_map(|a| {
            let id = a["assetId"].as_str()?.to_string();
            let thumb = a["previewImage"]
                .as_object()
                .and_then(|m| m.iter().find(|(k, _)| k.contains("256") && k.to_uppercase().contains("PNG")).or_else(|| m.iter().next()))
                .and_then(|(_, u)| u.as_str().map(str::to_string))
                .unwrap_or_else(|| format!("https://acg-media.struffelproductions.com/file/ambientCG-Web/media/thumbnail/256-PNG/{id}.png"));
            Some(Item {
                source: Source::AmbientCg,
                kind: Kind::Textures,
                name: a["displayName"].as_str().unwrap_or(&id).to_string(),
                tags: a["tags"].as_array().map(|t| t.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default(),
                popularity: a["downloadCount"].as_u64().unwrap_or(0),
                id,
                thumb,
            })
        })
        .collect()
}

/// Combined search used by the agent tool.
pub fn search(source: Source, kind: Kind, query: &str, max: usize) -> Result<Vec<Item>, String> {
    let q = query.to_lowercase();
    let mut items = match source {
        Source::PolyHaven => polyhaven_list(kind)?,
        Source::AmbientCg => ambientcg_search(query, 0)?,
    };
    if source == Source::PolyHaven && !q.is_empty() {
        items.retain(|i| matches(i, &q));
    }
    items.truncate(max);
    Ok(items)
}

pub fn matches(i: &Item, q: &str) -> bool {
    q.split_whitespace().all(|w| {
        i.name.to_lowercase().contains(w)
            || i.id.to_lowercase().contains(w)
            || i.tags.iter().any(|t| t.to_lowercase().contains(w))
    })
}

// ------------------------------------------------------------ import

/// Find the diffuse/albedo map in a Poly Haven `files` response.
pub fn ph_diffuse_url(files: &Value) -> Option<String> {
    let obj = files.as_object()?;
    let key = obj.keys().find(|k| {
        let l = k.to_lowercase();
        l == "diffuse" || l == "diff" || l.contains("albedo") || l == "color"
    })?;
    let res = &obj[key];
    for size in ["1k", "2k", "4k"] {
        for fmt in ["jpg", "png"] {
            if let Some(u) = res[size][fmt]["url"].as_str() {
                return Some(u.to_string());
            }
        }
    }
    None
}

/// Download an asset into the project. Returns what was imported.
pub fn import(item_source: Source, kind: Kind, id: &str, root: &Path) -> Result<Imported, String> {
    let safe: String = id
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    match (item_source, kind) {
        (Source::PolyHaven, Kind::Textures) => {
            let files = get_json(&format!("{}/files/{safe}", ph_api()))?;
            let url = ph_diffuse_url(&files).ok_or("no diffuse map found for this texture")?;
            let ext = if url.ends_with(".png") { "png" } else { "jpg" };
            let rel = format!("Textures/{safe}.{ext}");
            write(root, &rel, &get_bytes(&url, 256 << 20)?)?;
            Ok(Imported {
                path: rel,
                kind,
                extents: None,
                color: None,
            })
        }
        (Source::PolyHaven, Kind::Models) => {
            let files = get_json(&format!("{}/files/{safe}", ph_api()))?;
            let g = ["1k", "2k", "4k"]
                .iter()
                .map(|s| &files["gltf"][*s]["gltf"])
                .find(|g| g["url"].is_string())
                .ok_or("no glTF download for this model")?;
            let dir = format!("Models/{safe}");
            let main_url = g["url"].as_str().unwrap();
            let main_name = main_url
                .rsplit('/')
                .next()
                .unwrap_or("model.gltf")
                .split('?')
                .next()
                .unwrap_or("model.gltf");
            let rel = format!("{dir}/{main_name}");
            write(root, &rel, &get_bytes(main_url, 256 << 20)?)?;
            if let Some(inc) = g["include"].as_object() {
                for (sub, f) in inc {
                    if let Some(u) = f["url"].as_str() {
                        if sub.contains("..") {
                            continue;
                        }
                        write(root, &format!("{dir}/{sub}"), &get_bytes(u, 256 << 20)?)?;
                    }
                }
            }
            let data = crate::model::load(&root.join(&rel))?;
            Ok(Imported {
                path: rel,
                kind,
                extents: Some(data.extents.to_array()),
                color: Some(data.base_color),
            })
        }
        (Source::AmbientCg, _) => {
            let zip = get_bytes(
                &format!("https://ambientcg.com/get?file={safe}_1K-JPG.zip"),
                256 << 20,
            )?;
            let mut z = zip::ZipArchive::new(std::io::Cursor::new(zip))
                .map_err(|e| format!("ambientCG zip: {e}"))?;
            let name = (0..z.len())
                .filter_map(|i| z.by_index(i).ok().map(|f| f.name().to_string()))
                .find(|n| {
                    n.contains("_Color.") || n.contains("_Albedo.") || n.contains("_Diffuse.")
                })
                .ok_or("no color map in the ambientCG download")?;
            let mut bytes = vec![];
            z.by_name(&name)
                .map_err(|e| e.to_string())?
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            let rel = format!("Textures/{safe}.jpg");
            write(root, &rel, &bytes)?;
            Ok(Imported {
                path: rel,
                kind: Kind::Textures,
                extents: None,
                color: None,
            })
        }
    }
}

fn write(root: &Path, rel: &str, bytes: &[u8]) -> Result<(), String> {
    let p = crate::ai::tools::project_path(root, rel)?;
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(&p, bytes).map_err(|e| e.to_string())
}

// ------------------------------------------------------------ UI

pub enum StoreAction {
    ApplyTexture(String),
    AddModel(Imported, String),
    Log(String),
}

type Slot<T> = Arc<Mutex<Option<Result<T, String>>>>;

pub struct Store {
    source: Source,
    kind: Kind,
    query: String,
    results: Slot<Vec<Item>>,
    loaded_for: Option<(Source, Kind, String)>,
    ph_cache: HashMap<Kind, Vec<Item>>,
    page: usize,
    thumbs: HashMap<String, egui::TextureHandle>,
    thumb_pending: HashSet<String>,
    thumb_tx: Sender<(String, Option<egui::ColorImage>)>,
    thumb_rx: Receiver<(String, Option<egui::ColorImage>)>,
    jobs: Arc<Mutex<HashMap<String, Result<Option<Imported>, String>>>>,
    show_links: bool,
}

impl Default for Store {
    fn default() -> Self {
        let (thumb_tx, thumb_rx) = channel();
        Self {
            source: Source::PolyHaven,
            kind: Kind::Textures,
            query: String::new(),
            results: Arc::new(Mutex::new(None)),
            loaded_for: None,
            ph_cache: HashMap::new(),
            page: 0,
            thumbs: HashMap::new(),
            thumb_pending: HashSet::new(),
            thumb_tx,
            thumb_rx,
            jobs: Arc::new(Mutex::new(HashMap::new())),
            show_links: false,
        }
    }
}

const PAGE: usize = 24;

pub const OTHER_LIBRARIES: &[(&str, &str, &str, &str)] = &[
    (
        "Kenney",
        "https://kenney.nl/assets",
        "40,000+ CC0 game assets: 3D kits, 2D sprites, UI, audio",
        "40.000+ CC0 oyun varlığı: 3D kitler, 2D sprite, UI, ses",
    ),
    (
        "Quaternius",
        "https://quaternius.com",
        "CC0 low-poly 3D models & animated characters (glTF)",
        "CC0 low-poly 3D modeller ve animasyonlu karakterler (glTF)",
    ),
    (
        "Poly Pizza",
        "https://poly.pizza",
        "Thousands of free low-poly models (glTF)",
        "Binlerce ücretsiz low-poly model (glTF)",
    ),
    (
        "OpenGameArt",
        "https://opengameart.org",
        "Community 2D/3D art, music and sound effects",
        "Topluluk 2D/3D çizim, müzik ve ses efektleri",
    ),
    (
        "itch.io free assets",
        "https://itch.io/game-assets/free",
        "Free asset packs from indie creators",
        "Bağımsız yapımcılardan ücretsiz paketler",
    ),
    (
        "Freesound",
        "https://freesound.org",
        "Creative-Commons sound effects",
        "Creative Commons ses efektleri",
    ),
];

impl Store {
    fn refresh(&mut self, ctx: &egui::Context) {
        let key = (self.source, self.kind, self.query.clone());
        if self.loaded_for.as_ref() == Some(&key) {
            return;
        }
        self.page = 0;
        // Poly Haven: catalog fetched once per kind, then filtered locally.
        if self.source == Source::PolyHaven {
            if let Some(all) = self.ph_cache.get(&self.kind) {
                let q = self.query.to_lowercase();
                let list: Vec<Item> = all
                    .iter()
                    .filter(|i| q.is_empty() || matches(i, &q))
                    .cloned()
                    .collect();
                *self.results.lock().unwrap() = Some(Ok(list));
                self.loaded_for = Some(key);
                return;
            }
        }
        self.loaded_for = Some(key);
        *self.results.lock().unwrap() = None;
        let slot = self.results.clone();
        let (source, kind, query) = (self.source, self.kind, self.query.clone());
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let q = query.to_lowercase();
            let r = match source {
                Source::PolyHaven => polyhaven_list(kind).map(|mut v| {
                    if !q.is_empty() {
                        v.retain(|i| matches(i, &q));
                    }
                    v
                }),
                Source::AmbientCg => ambientcg_search(&query, 0),
            };
            *slot.lock().unwrap() = Some(r);
            ctx.request_repaint();
        });
    }

    fn request_thumb(&mut self, ctx: &egui::Context, url: &str) {
        if self.thumbs.contains_key(url)
            || self.thumb_pending.contains(url)
            || self.thumb_pending.len() > 8
        {
            return;
        }
        self.thumb_pending.insert(url.to_string());
        let tx = self.thumb_tx.clone();
        let url = url.to_string();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let img = get_bytes(&url, 4 << 20)
                .ok()
                .and_then(|b| image::load_from_memory(&b).ok())
                .map(|i| {
                    let i = i.thumbnail(160, 160).to_rgba8();
                    egui::ColorImage::from_rgba_unmultiplied(
                        [i.width() as usize, i.height() as usize],
                        i.as_raw(),
                    )
                });
            let _ = tx.send((url, img));
            ctx.request_repaint();
        });
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, root: &Path, has_selection: bool) -> Vec<StoreAction> {
        let mut actions = vec![];
        let ctx = ui.ctx().clone();
        while let Ok((url, img)) = self.thumb_rx.try_recv() {
            self.thumb_pending.remove(&url);
            if let Some(img) = img {
                self.thumbs.insert(
                    url.clone(),
                    ctx.load_texture(url, img, egui::TextureOptions::LINEAR),
                );
            }
        }
        // Header
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.source, Source::PolyHaven, "Poly Haven");
            ui.selectable_value(&mut self.source, Source::AmbientCg, "ambientCG");
            ui.toggle_value(
                &mut self.show_links,
                t("More free libraries", "Diğer ücretsiz kütüphaneler"),
            );
            ui.separator();
            if self.source == Source::PolyHaven {
                ui.selectable_value(&mut self.kind, Kind::Textures, t("Textures", "Dokular"));
                ui.selectable_value(&mut self.kind, Kind::Models, t("3D Models", "3D Modeller"));
            } else {
                self.kind = Kind::Textures;
                ui.label(RichText::new(t("PBR materials", "PBR malzemeler")).weak());
            }
            ui.separator();
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text(t(
                        "🔍 Search (e.g. wood, rock, chair)",
                        "🔍 Ara (örn. wood, rock, chair)",
                    ))
                    .desired_width(220.0),
            );
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.loaded_for = None;
            }
            ui.label(
                RichText::new("CC0")
                    .small()
                    .color(Color32::from_rgb(120, 210, 140)),
            )
            .on_hover_text(t(
                "Public domain — free for commercial use, no attribution required.",
                "Kamu malı — ticari kullanım serbest, atıf gerekmez.",
            ));
        });
        if self.show_links {
            egui::Frame::new().fill(theme::BG2).corner_radius(6).inner_margin(8).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(t("Download a pack, then use Content Browser › Import (glTF/GLB models, PNG/JPG textures).", "Bir paket indir, sonra İçerik Tarayıcı › İçe aktar (glTF/GLB model, PNG/JPG doku).")).small());
                for (name, url, en, tr) in OTHER_LIBRARIES {
                    ui.horizontal(|ui| {
                        ui.hyperlink_to(RichText::new(*name).strong(), *url);
                        ui.label(RichText::new(t(en, tr)).small().weak());
                    });
                }
            });
        }
        // Poly Haven results arrive as a full catalog: cache it for local filtering.
        let res = self.results.lock().unwrap().clone();
        // Cache under the kind the results were actually loaded for.
        if let (Some(Ok(list)), Some((Source::PolyHaven, kind, q))) =
            (&res, self.loaded_for.clone())
        {
            if q.is_empty() {
                self.ph_cache.entry(kind).or_insert_with(|| list.clone());
            }
        }
        let key_changed = self.loaded_for.as_ref().map_or(true, |k| {
            k.0 != self.source
                || k.1 != self.kind
                || (k.0 == Source::PolyHaven && k.2 != self.query)
        });
        if key_changed {
            self.refresh(&ctx);
        }
        ui.separator();
        let items = match self.results.lock().unwrap().clone() {
            None => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(t("Loading catalog…", "Katalog yükleniyor…"));
                });
                return actions;
            }
            Some(Err(e)) => {
                ui.colored_label(
                    Color32::LIGHT_RED,
                    format!(
                        "{} {e}",
                        t(
                            "Could not reach the asset service:",
                            "Varlık servisine ulaşılamadı:"
                        )
                    ),
                );
                if ui.button(t("Retry", "Tekrar dene")).clicked() {
                    self.loaded_for = None;
                }
                return actions;
            }
            Some(Ok(items)) => items,
        };
        let pages = items.len().div_ceil(PAGE).max(1);
        self.page = self.page.min(pages - 1);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{} {}", items.len(), t("assets", "varlık")))
                    .small()
                    .weak(),
            );
            if ui
                .add_enabled(self.page > 0, egui::Button::new("‹"))
                .clicked()
            {
                self.page -= 1;
            }
            ui.label(RichText::new(format!("{}/{}", self.page + 1, pages)).small());
            if ui
                .add_enabled(self.page + 1 < pages, egui::Button::new("›"))
                .clicked()
            {
                self.page += 1;
            }
        });
        let page: Vec<Item> = items
            .iter()
            .skip(self.page * PAGE)
            .take(PAGE)
            .cloned()
            .collect();
        let jobs = self.jobs.lock().unwrap().clone();
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                let cols = ((ui.available_width() + 10.0) / 162.0).floor().max(1.0) as usize;
                for row in page.chunks(cols) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
                        for it in row.iter().cloned() {
                            self.request_thumb(&ctx, &it.thumb);
                            let key = format!("{:?}/{}", it.source, it.id);
                            egui::Frame::new()
                                .fill(theme::BG2)
                                .corner_radius(8)
                                .inner_margin(6)
                                .show(ui, |ui| {
                                    ui.set_width(140.0);
                                    ui.vertical(|ui| {
                                        let (rect, _) = ui.allocate_exact_size(
                                            egui::vec2(140.0, 110.0),
                                            egui::Sense::hover(),
                                        );
                                        match self.thumbs.get(&it.thumb) {
                                            Some(tex) => {
                                                ui.painter().image(
                                                    tex.id(),
                                                    rect,
                                                    egui::Rect::from_min_max(
                                                        egui::pos2(0.0, 0.0),
                                                        egui::pos2(1.0, 1.0),
                                                    ),
                                                    Color32::WHITE,
                                                );
                                            }
                                            None => {
                                                ui.painter().rect_filled(rect, 4.0, theme::BG0);
                                                ui.painter().text(
                                                    rect.center(),
                                                    egui::Align2::CENTER_CENTER,
                                                    if it.kind == Kind::Models {
                                                        "📦"
                                                    } else {
                                                        "🖼"
                                                    },
                                                    egui::FontId::proportional(28.0),
                                                    Color32::from_gray(90),
                                                );
                                            }
                                        }
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(&it.name).strong().size(12.0),
                                            )
                                            .truncate(),
                                        );
                                        ui.label(
                                            RichText::new(it.source.label()).size(10.5).weak(),
                                        );
                                        match jobs.get(&key) {
                                            None => {
                                                let label = if it.kind == Kind::Models {
                                                    t("⬇ Add to project", "⬇ Projeye ekle")
                                                } else {
                                                    t("⬇ Download", "⬇ İndir")
                                                };
                                                if ui
                                                    .add(
                                                        egui::Button::new(
                                                            RichText::new(label)
                                                                .size(12.0)
                                                                .color(Color32::WHITE),
                                                        )
                                                        .fill(ACCENT),
                                                    )
                                                    .clicked()
                                                {
                                                    self.start_import(&it, root, &ctx);
                                                }
                                            }
                                            Some(Ok(None)) => {
                                                ui.horizontal(|ui| {
                                                    ui.spinner();
                                                    ui.label(
                                                        RichText::new(t(
                                                            "downloading…",
                                                            "indiriliyor…",
                                                        ))
                                                        .size(11.0),
                                                    );
                                                });
                                            }
                                            Some(Ok(Some(imp))) => {
                                                ui.label(
                                                    RichText::new(format!("✔ {}", imp.path))
                                                        .size(10.5)
                                                        .color(Color32::from_rgb(120, 210, 140)),
                                                );
                                                if imp.kind == Kind::Models {
                                                    if ui
                                                        .button(t(
                                                            "+ Place in scene",
                                                            "+ Sahneye koy",
                                                        ))
                                                        .clicked()
                                                    {
                                                        actions.push(StoreAction::AddModel(
                                                            imp.clone(),
                                                            it.name.clone(),
                                                        ));
                                                    }
                                                } else if ui
                                                    .add_enabled(
                                                        has_selection,
                                                        egui::Button::new(t(
                                                            "Apply to selected",
                                                            "Seçiliye uygula",
                                                        )),
                                                    )
                                                    .clicked()
                                                {
                                                    actions.push(StoreAction::ApplyTexture(
                                                        imp.path.clone(),
                                                    ));
                                                }
                                            }
                                            Some(Err(e)) => {
                                                ui.label(
                                                    RichText::new(
                                                        e.chars().take(80).collect::<String>(),
                                                    )
                                                    .size(10.5)
                                                    .color(Color32::LIGHT_RED),
                                                );
                                                if ui.small_button(t("Retry", "Tekrar")).clicked() {
                                                    self.jobs.lock().unwrap().remove(&key);
                                                }
                                            }
                                        }
                                    });
                                });
                        }
                    });
                    ui.add_space(10.0);
                }
            });
        // Report finished imports once.
        for (k, v) in jobs {
            if let Ok(Some(imp)) = v {
                if self.thumb_pending.insert(format!("logged:{k}")) {
                    actions.push(StoreAction::Log(format!(
                        "{} {}",
                        t("Imported", "İçe aktarıldı:"),
                        imp.path
                    )));
                }
            }
        }
        actions
    }

    fn start_import(&mut self, it: &Item, root: &Path, ctx: &egui::Context) {
        let key = format!("{:?}/{}", it.source, it.id);
        self.jobs.lock().unwrap().insert(key.clone(), Ok(None));
        let jobs = self.jobs.clone();
        let (source, kind, id, root, ctx) = (
            it.source,
            it.kind,
            it.id.clone(),
            root.to_path_buf(),
            ctx.clone(),
        );
        std::thread::spawn(move || {
            let r = import(source, kind, &id, &root).map(Some);
            jobs.lock().unwrap().insert(key, r);
            ctx.request_repaint();
        });
    }
}
