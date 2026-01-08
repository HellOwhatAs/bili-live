use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;

#[derive(Default, Debug, Clone)]
pub struct UserCookies {
    pub room_id: String,
    pub cookie_str: String,
    pub csrf: String,
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
    let map = load_ini_map("cookies.txt")?;

    // Support both "cookie" and "cookie_str" keys if format varies
    let cookie = map.get("cookie").or(map.get("cookie_str"))?;
    let room_id = map.get("room_id")?;
    let csrf = map.get("csrf")?;

    Some(UserCookies {
        room_id: room_id.clone(),
        cookie_str: cookie.clone(),
        csrf: csrf.clone(),
    })
}

pub fn save_cookies(cookies: &UserCookies) {
    let content = format!(
        "room_id: {}\ncookie: {}\ncsrf: {}\n",
        cookies.room_id, cookies.cookie_str, cookies.csrf
    );
    let _ = fs::write("cookies.txt", content);
}

pub fn load_last_settings() -> LiveSettings {
    if let Ok(content) = fs::read_to_string("last_settings.json") {
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        LiveSettings::default()
    }
}

pub fn save_last_settings(settings: &LiveSettings) {
    if let Ok(content) = serde_json::to_string(settings) {
        let _ = fs::write("last_settings.json", content);
    }
}

// Helpers

fn load_ini_map(path: &str) -> Option<HashMap<String, String>> {
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
