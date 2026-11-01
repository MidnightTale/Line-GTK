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
        let file = match code {
            "vi" | "vie" | "vietnamese" => "vie.json",
            "th" | "thai" => "thai.json",
            "ja" | "ja-JP" | "ja_JP" | "japanese" => "ja.json",
            _ => "eng.json",
        };
        // 翻訳がない項目は英語で表示し、内部キーの露出を防ぐ。
        let mut map = read_map(&lang_path(repo_root, "eng.json")).unwrap_or_default();
        if file != "eng.json"
            && let Some(translated) = read_map(&lang_path(repo_root, file))
        {
            map.extend(translated);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn placeholders(text: &str) -> Vec<&str> {
        let mut values: Vec<_> = text
            .split('{')
            .skip(1)
            .filter_map(|part| part.split_once('}').map(|(name, _)| name))
            .collect();
        values.sort_unstable();
        values
    }

    #[test]
    fn japanese_catalog_is_complete() {
        let en: HashMap<String, String> =
            serde_json::from_str(include_str!("../assets/lang/eng.json")).unwrap();
        let ja: HashMap<String, String> =
            serde_json::from_str(include_str!("../assets/lang/ja.json")).unwrap();
        assert_eq!(en.len(), ja.len());
        for (key, english) in &en {
            let japanese = ja.get(key).expect("日本語訳がありません");
            assert!(!japanese.trim().is_empty(), "空の日本語訳：{key}");
            assert_eq!(placeholders(english), placeholders(japanese), "{key}");
        }
        let language = I18n::load(Path::new(env!("CARGO_MANIFEST_DIR")), "ja");
        assert_eq!(language.t("send"), "送信");
        assert_eq!(
            language.tf("call_calling", &[("name", "しらふか")]),
            "しらふかに発信中…"
        );
    }

    #[test]
    fn vietnamese_catalog_is_complete_and_default_language_is_preserved() {
        let en: HashMap<String, String> =
            serde_json::from_str(include_str!("../assets/lang/eng.json")).unwrap();
        let vi: HashMap<String, String> =
            serde_json::from_str(include_str!("../assets/lang/vie.json")).unwrap();
        assert_eq!(en.len(), vi.len());
        for (key, english) in &en {
            let translated = vi.get(key).expect("Missing Vietnamese translation");
            assert!(
                !translated.trim().is_empty(),
                "Empty Vietnamese translation: {key}"
            );
            assert_eq!(placeholders(english), placeholders(translated), "{key}");
        }
        let config = crate::config::AppConfig::default();
        assert_eq!(config.language, "vi");
        let language = I18n::load(Path::new(env!("CARGO_MANIFEST_DIR")), &config.language);
        assert_eq!(language.t("send"), "Gửi");
        assert_eq!(language.tf("reply_to", &[("name", "Lan")]), "Trả lời Lan");
    }

    #[test]
    fn missing_translations_fall_back_to_english() {
        let root = std::env::temp_dir().join(format!("line-gtk-i18n-test-{}", std::process::id()));
        let directory = root.join("assets/lang");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("eng.json"),
            r#"{"send":"Send","future":"Future"}"#,
        )
        .unwrap();
        fs::write(directory.join("ja.json"), r#"{"send":"送信"}"#).unwrap();
        let language = I18n::load(&root, "ja");
        assert_eq!(language.t("send"), "送信");
        assert_eq!(language.t("future"), "Future");
        fs::remove_file(directory.join("ja.json")).unwrap();
        assert_eq!(I18n::load(&root, "ja").t("send"), "Send");
        fs::remove_dir_all(root).unwrap();
    }
}
