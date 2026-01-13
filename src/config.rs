use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Default, Debug, Clone)]
pub struct UserCookies {
    pub room_id: String,
    pub cookie_str: String,
    pub csrf: String,
    pub refresh_token: String,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct LiveSettings {
    pub title: String,
    pub area_id: String,
    pub area_name: String,
    pub sub_area_id: String,
    pub sub_area_name: String,
}

pub fn load_cookies() -> Option<UserCookies> {
    let path = get_config_path("cookies.txt");
    let map = load_ini_map(&path)?;

    // Support both "cookie" and "cookie_str" keys if format varies
    let cookie = map.get("cookie").or(map.get("cookie_str"))?;
    let room_id = map.get("room_id")?;
    let csrf = map.get("csrf")?;
    let refresh_token = map.get("refresh_token").cloned().unwrap_or_default();

    Some(UserCookies {
        room_id: room_id.clone(),
        cookie_str: cookie.clone(),
        csrf: csrf.clone(),
        refresh_token,
    })
}

pub fn save_cookies(cookies: &UserCookies) {
    let content = format!(
        "room_id: {}\ncookie: {}\ncsrf: {}\nrefresh_token: {}\n",
        cookies.room_id, cookies.cookie_str, cookies.csrf, cookies.refresh_token
    );
    let path = get_config_path("cookies.txt");
    let _ = fs::write(path, content);
}

pub fn load_last_settings() -> LiveSettings {
    let path = get_config_path("last_settings.json");
    if let Ok(content) = fs::read_to_string(path) {
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        LiveSettings::default()
    }
}

pub fn save_last_settings(settings: &LiveSettings) {
    let path = get_config_path("last_settings.json");
    if let Ok(content) = serde_json::to_string(settings) {
        let _ = fs::write(path, content);
    }
}

// Helpers

fn get_config_path(filename: &str) -> PathBuf {
    let home = dirs::home_dir().expect("Failed to get home directory");
    let dir = home.join(".bili-live");

    if !dir.exists() {
        let _ = fs::create_dir_all(&dir);
    }
    dir.join(filename)
}

fn load_ini_map<P: AsRef<Path>>(path: P) -> Option<HashMap<String, String>> {
    fs::read_to_string(path).ok().map(|content| {
        content
            .lines()
            .filter_map(|line| {
                let (k, v) = line.split_once(':')?;
                Some((k.trim().to_string(), v.trim().to_string()))
            })
            .collect()
    })
}
