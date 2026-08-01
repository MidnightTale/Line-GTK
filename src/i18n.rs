use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

static LANG: RwLock<Option<I18n>> = RwLock::new(None);

#[derive(Debug, Clone, Default)]
pub struct I18n {
    map: HashMap<String, String>,
}

impl I18n {
    pub fn load(repo_root: &Path, code: &str) -> Self {
        let file = if code == "th" || code == "thai" {
            "thai.json"
        } else {
            "eng.json"
        };
        let path = lang_path(repo_root, file);
        let map = read_map(&path).unwrap_or_else(|| {
            // fallback bundled next to exe / eng
            read_map(&lang_path(repo_root, "eng.json")).unwrap_or_default()
        });
        Self { map }
    }

    pub fn t(&self, key: &str) -> String {
        self.map
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.to_string())
    }

    pub fn tf(&self, key: &str, pairs: &[(&str, &str)]) -> String {
        let mut s = self.t(key);
        for (k, v) in pairs {
            s = s.replace(&format!("{{{k}}}"), v);
        }
        s
    }
}

fn lang_path(repo_root: &Path, file: &str) -> PathBuf {
    repo_root.join("assets/lang").join(file)
}

fn read_map(path: &Path) -> Option<HashMap<String, String>> {
    let raw = fs::read_to_string(path).ok()?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    let obj = v.as_object()?;
    let mut map = HashMap::new();
    for (k, val) in obj {
        if let Some(s) = val.as_str() {
            map.insert(k.clone(), s.to_string());
        }
    }
    Some(map)
}

pub fn set_lang(repo_root: &Path, code: &str) {
    let i18n = I18n::load(repo_root, code);
    if let Ok(mut g) = LANG.write() {
        *g = Some(i18n);
    }
}

pub fn t(key: &str) -> String {
    if let Ok(g) = LANG.read()
        && let Some(i) = g.as_ref()
    {
        return i.t(key);
    }
    key.to_string()
}

pub fn tf(key: &str, pairs: &[(&str, &str)]) -> String {
    if let Ok(g) = LANG.read()
        && let Some(i) = g.as_ref()
    {
        return i.tf(key, pairs);
    }
    key.to_string()
}
