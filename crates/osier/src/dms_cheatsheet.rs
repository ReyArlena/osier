//! Osier's shortcuts as a Dux Shell (DankMaterialShell) cheatsheet, so `dms keybinds show
//! osier` and the DMS keybinds overlay list them.
//!
//! Output only: `keymap.toml` is the source. Written to `<config>/dms/cheatsheets/osier.json`
//! when `<config>/dms` exists (`<config>` = `$XDG_CONFIG_HOME`, else `~/.config`), on startup
//! and whenever the bindings change. The file is rewritten only when its content changes.
//! Format: https://duxlabs.dev/docs/dms/cli-keybinds-cheatsheets

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::keymap::{Keymap, Scope, actions, pretty};

/// `<config>/dms`, whether or not it exists.
pub(crate) fn dms_dir() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(config.join("dms"))
}

/// The cheatsheet document: categories in registry order, one entry per key.
pub(crate) fn document(keymap: &Keymap) -> String {
    let mut binds = Map::new();
    for spec in actions() {
        let keys = keymap.keys_for(spec.id);
        if keys.is_empty() {
            continue;
        }
        let entries = binds
            .entry(spec.category.to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        let Value::Array(list) = entries else {
            continue;
        };
        let mut shown: Vec<String> = Vec::new();
        for k in keys {
            let key = pretty(k);
            if shown.contains(&key) {
                continue;
            }
            shown.push(key.clone());
            let mut entry = json!({ "key": key, "desc": spec.name });
            let subcat = match spec.scope {
                Scope::App => None,
                Scope::List => Some("In lists"),
                Scope::Settings => Some("In settings"),
            };
            if let (Some(sub), Value::Object(o)) = (subcat, &mut entry) {
                o.insert("subcat".into(), Value::String(sub.into()));
            }
            list.push(entry);
        }
    }
    let doc = json!({ "title": "Osier", "provider": "osier", "binds": Value::Object(binds) });
    serde_json::to_string_pretty(&doc).unwrap_or_default() + "\n"
}

/// Write the cheatsheet under `dms` if that directory exists. `Ok(true)` when written,
/// `Ok(false)` when DMS is absent or the file already had this content.
pub(crate) fn write(dms: &Path, keymap: &Keymap) -> std::io::Result<bool> {
    if !dms.is_dir() {
        return Ok(false);
    }
    let dir = dms.join("cheatsheets");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("osier.json");
    let text = document(keymap);
    if std::fs::read_to_string(&path).ok().as_deref() == Some(text.as_str()) {
        return Ok(false);
    }
    let tmp = dir.join(".osier.json.tmp");
    std::fs::write(&tmp, &text)?;
    std::fs::rename(&tmp, &path)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::{document, write};
    use crate::keymap::Keymap;

    fn sample() -> Keymap {
        let mut k = Keymap::default();
        k.keys.insert("quit", vec!["ctrl-q".into()]);
        k.keys.insert("refresh", vec!["ctrl-r".into(), "f5".into()]);
        k.keys
            .insert("select_next", vec!["down".into(), "j".into()]);
        k.keys.insert("focus_settings_search", vec!["/".into()]);
        k.keys.insert("tab_log", vec![]);
        k
    }

    #[test]
    fn document_matches_the_dms_format() {
        let doc: serde_json::Value = serde_json::from_str(&document(&sample())).unwrap();
        assert_eq!(doc["title"], "Osier");
        assert_eq!(doc["provider"], "osier");
        let general = doc["binds"]["General"].as_array().unwrap();
        assert_eq!(
            general[0],
            serde_json::json!({ "key": "Ctrl+Q", "desc": "Quit" })
        );
        assert_eq!(general[2]["key"], "F5");
        assert_eq!(doc["binds"]["Lists"][1]["key"], "J");
        assert_eq!(doc["binds"]["Lists"][1]["subcat"], "In lists");
        assert_eq!(doc["binds"]["Search"][0]["subcat"], "In settings");
        assert!(
            doc["binds"].get("Tabs").is_none(),
            "unbound actions are left out"
        );
        // Category order follows the registry.
        let cats: Vec<&String> = doc["binds"].as_object().unwrap().keys().collect();
        assert_eq!(cats, ["General", "Search", "Lists"]);
    }

    #[test]
    fn writes_only_when_dms_exists_and_content_changed() {
        let dir = tempfile::tempdir().unwrap();
        let dms = dir.path().join("dms");
        assert!(!write(&dms, &sample()).unwrap(), "no DMS, no file");
        assert!(!dms.exists());
        std::fs::create_dir(&dms).unwrap();
        assert!(write(&dms, &sample()).unwrap(), "creates cheatsheets/");
        let file = dms.join("cheatsheets/osier.json");
        assert!(file.is_file());
        assert!(
            !write(&dms, &sample()).unwrap(),
            "same content is not rewritten"
        );
        let mut changed = sample();
        changed.keys.insert("quit", vec!["ctrl-shift-q".into()]);
        assert!(write(&dms, &changed).unwrap());
        assert!(
            std::fs::read_to_string(&file)
                .unwrap()
                .contains("Ctrl+Shift+Q")
        );
    }
}
