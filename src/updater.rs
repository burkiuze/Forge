//! Self-update from GitHub Releases.
//!
//! CI publishes every push to `main` as a release tagged `build-<run_number>` with fixed asset
//! names. On start-up Forge compares that number with its own `FORGE_BUILD` and, if newer, asks
//! the user whether to update. Installed Windows copies run the new silent installer; portable,
//! Linux and macOS copies replace their own executable.

use serde_json::Value;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub const REPO: &str = "burkiuze/Forge";
pub const BUILD: u32 = parse_build(env!("FORGE_BUILD"));
pub const COMMIT: &str = env!("FORGE_COMMIT");

const fn parse_build(s: &str) -> u32 {
    let b = s.as_bytes();
    let mut i = 0;
    let mut n = 0u32;
    while i < b.len() {
        if b[i] >= b'0' && b[i] <= b'9' {
            n = n * 10 + (b[i] - b'0') as u32;
        }
        i += 1;
    }
    n
}

#[derive(Clone, Debug)]
pub struct Release {
    pub build: u32,
    pub title: String,
    pub notes: String,
    pub assets: Vec<(String, String, u64)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    Available,
    Downloading(f32),
    ReadyToRestart,
    Failed(String),
}

pub struct Updater {
    pub state: Arc<Mutex<UpdateState>>,
    pub release: Arc<Mutex<Option<Release>>>,
    /// The user chose "later" for this build.
    pub dismissed: bool,
}

impl Default for Updater {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(UpdateState::Idle)),
            release: Arc::new(Mutex::new(None)),
            dismissed: false,
        }
    }
}

pub fn tag_build(tag: &str) -> u32 {
    tag.rsplit(|c: char| !c.is_ascii_digit())
        .find(|s| !s.is_empty())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn fetch_latest() -> Result<Release, String> {
    // FORGE_UPDATE_API overrides the endpoint (testing / mirrors).
    let url = std::env::var("FORGE_UPDATE_API")
        .unwrap_or_else(|_| format!("https://api.github.com/repos/{REPO}/releases/latest"));
    let r = crate::ai::client::agent()
        .get(&url)
        .set("Accept", "application/vnd.github+json")
        .call();
    let v: Value = match r {
        Ok(r) => r.into_json().map_err(|e| e.to_string())?,
        Err(ureq::Error::Status(404, _)) => return Err("no releases yet".into()),
        Err(e) => return Err(e.to_string()),
    };
    let tag = v["tag_name"].as_str().unwrap_or_default().to_string();
    Ok(Release {
        build: tag_build(&tag),
        title: v["name"].as_str().unwrap_or(&tag).to_string(),
        notes: v["body"].as_str().unwrap_or_default().to_string(),
        assets: v["assets"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|x| {
                        (
                            x["name"].as_str().unwrap_or_default().to_string(),
                            x["browser_download_url"]
                                .as_str()
                                .unwrap_or_default()
                                .to_string(),
                            x["size"].as_u64().unwrap_or(0),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// Is this copy installed by the Windows setup (vs. portable)?
fn installed_copy() -> bool {
    cfg!(windows)
        && std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("Uninstall.exe").is_file()))
            .unwrap_or(false)
}

fn wanted_asset() -> &'static str {
    if cfg!(windows) {
        if installed_copy() {
            "Forge-Setup.exe"
        } else {
            "Forge-portable.exe"
        }
    } else if cfg!(target_os = "macos") {
        "forge-macos.tar.gz"
    } else {
        "forge-linux-x64.tar.gz"
    }
}

impl Updater {
    pub fn check(&self, ctx: &egui::Context) {
        let state = self.state.clone();
        let rel = self.release.clone();
        let ctx = ctx.clone();
        *state.lock().unwrap() = UpdateState::Checking;
        std::thread::spawn(move || {
            let s = match fetch_latest() {
                Ok(r) if r.build > BUILD && r.assets.iter().any(|a| a.0 == wanted_asset()) => {
                    *rel.lock().unwrap() = Some(r);
                    UpdateState::Available
                }
                Ok(r) => {
                    *rel.lock().unwrap() = Some(r);
                    UpdateState::UpToDate
                }
                Err(e) => UpdateState::Failed(e),
            };
            *state.lock().unwrap() = s;
            ctx.request_repaint();
        });
    }

    pub fn state(&self) -> UpdateState {
        self.state.lock().unwrap().clone()
    }

    pub fn available(&self) -> Option<Release> {
        (self.state() == UpdateState::Available)
            .then(|| self.release.lock().unwrap().clone())
            .flatten()
    }

    /// Download the new build and stage it. Afterwards call `restart`.
    pub fn install(&self, ctx: &egui::Context) {
        let Some(rel) = self.release.lock().unwrap().clone() else {
            return;
        };
        let Some((name, url, size)) = rel.assets.iter().find(|a| a.0 == wanted_asset()).cloned()
        else {
            return;
        };
        let state = self.state.clone();
        let ctx = ctx.clone();
        *state.lock().unwrap() = UpdateState::Downloading(0.0);
        std::thread::spawn(move || {
            let r = (|| -> Result<(), String> {
                let dir = crate::config::data_dir().join("update");
                let _ = std::fs::remove_dir_all(&dir);
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let file = dir.join(&name);
                let resp = crate::ai::client::builder()
                    .redirects(8)
                    .build()
                    .get(&url)
                    .set("User-Agent", "Forge")
                    .call()
                    .map_err(|e| e.to_string())?;
                let mut reader = resp.into_reader();
                let mut out = std::fs::File::create(&file).map_err(|e| e.to_string())?;
                let mut buf = vec![0u8; 1 << 20];
                let mut done = 0u64;
                loop {
                    let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
                    if n == 0 {
                        break;
                    }
                    std::io::Write::write_all(&mut out, &buf[..n]).map_err(|e| e.to_string())?;
                    done += n as u64;
                    *state.lock().unwrap() = UpdateState::Downloading(if size > 0 {
                        done as f32 / size as f32
                    } else {
                        0.0
                    });
                    ctx.request_repaint();
                }
                drop(out);
                stage(&file)
            })();
            *state.lock().unwrap() = match r {
                Ok(()) => UpdateState::ReadyToRestart,
                Err(e) => UpdateState::Failed(e),
            };
            ctx.request_repaint();
        });
    }

    /// Launch the new version and quit this one.
    pub fn restart(&self, ctx: &egui::Context) {
        let dir = crate::config::data_dir().join("update");
        let setup = dir.join("Forge-Setup.exe");
        if setup.is_file() {
            // Silent install over the current installation; the installer relaunches Forge.
            let _ = std::process::Command::new(&setup)
                .arg("/S")
                .arg("/RUN")
                .spawn();
        } else if let Some(exe) = launch_path() {
            let _ = std::process::Command::new(exe).spawn();
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

/// The executable to relaunch. After a self-replace the running image was renamed to
/// `*.old`, so `current_exe()` points there; the new build lives at the original name.
fn launch_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let s = exe.to_string_lossy();
    let base = s.strip_suffix(" (deleted)").unwrap_or(&s);
    let base = base.strip_suffix(".old").unwrap_or(base);
    let p = PathBuf::from(base);
    // `forge.old` came from `forge` (no extension) or `Forge.exe` (with_extension("old")).
    if p.is_file() {
        return Some(p);
    }
    let exe_name = p.with_extension(std::env::consts::EXE_EXTENSION);
    exe_name.is_file().then_some(exe_name)
}

/// Put the downloaded build in place (self-replace for portable / Unix copies).
fn stage(file: &std::path::Path) -> Result<(), String> {
    let name = file.file_name().unwrap().to_string_lossy().to_string();
    if name == "Forge-Setup.exe" {
        return Ok(());
    }
    let new_bin: PathBuf = if name.ends_with(".tar.gz") {
        let out = file.with_extension("").with_extension("");
        std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        tar::Archive::new(flate2::read::GzDecoder::new(
            std::fs::File::open(file).map_err(|e| e.to_string())?,
        ))
        .unpack(&out)
        .map_err(|e| e.to_string())?;
        out.join("forge")
    } else {
        file.to_path_buf()
    };
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let old = exe.with_extension("old");
    let _ = std::fs::remove_file(&old);
    // A running executable can be renamed (Windows too), then the new one takes its place.
    std::fs::rename(&exe, &old).map_err(|e| format!("cannot replace executable: {e}"))?;
    if let Err(e) = std::fs::copy(&new_bin, &exe) {
        let _ = std::fs::rename(&old, &exe);
        return Err(e.to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755));
    }
    Ok(())
}

/// Remove the previous executable left behind by a self-update.
pub fn cleanup_old() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("old"));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tags() {
        assert_eq!(super::tag_build("build-123"), 123);
        assert_eq!(super::tag_build("v0.2.0-build.45"), 45);
        assert_eq!(super::tag_build("nightly"), 0);
    }
}
