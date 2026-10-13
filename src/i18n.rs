//! Tiny two-language UI localisation (English / Türkçe).

use std::sync::atomic::{AtomicBool, Ordering};

static TURKISH: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize, Default)]
pub enum Lang {
    #[default]
    En,
    Tr,
}

impl Lang {
    pub fn label(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Tr => "Türkçe",
        }
    }
}

pub fn set(lang: Lang) {
    TURKISH.store(lang == Lang::Tr, Ordering::Relaxed);
}

/// Pick the string for the active language.
pub fn t(en: &'static str, tr: &'static str) -> &'static str {
    if TURKISH.load(Ordering::Relaxed) {
        tr
    } else {
        en
    }
}

/// Best guess from the OS locale (`LANG`, `LC_ALL`, Windows user locale via env).
pub fn detect() -> Lang {
    for k in ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"] {
        if let Ok(v) = std::env::var(k) {
            if v.to_ascii_lowercase().starts_with("tr") {
                return Lang::Tr;
            }
        }
    }
    Lang::En
}
