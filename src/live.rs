use serde_json::Value;
use std::collections::HashMap;

pub async fn get_room_id(uid: &str) -> Result<String, Box<dyn std::error::Error>> {
    let mut res: Value = serde_json::from_slice(
        reqwest::get(format!("https://api.live.bilibili.com/live_user/v1/Master/info?uid={uid}"))
        .await?.bytes().await?.as_ref()
    )?;
    match res["data"]["room_id"].take() {
        Value::Number(room_id) => Ok(room_id.to_string()),
        _ => panic!("{:?}", res)
    }
}

pub async fn get_live_status(uid: &str) -> Result<((bool, u64), (i64, String, String)), Box<dyn std::error::Error>> {
    let mut res: Value = serde_json::from_slice(
        reqwest::get(format!("https://api.live.bilibili.com/room/v1/Room/get_status_info_by_uids?uids[]={uid}"))
        .await?.bytes().await?.as_ref()
    )?;
    let live_status = match res["data"][uid]["live_status"].take() {
        Value::Number(live_status) => live_status.as_u64().unwrap() != 0,
        _ => panic!("{:?}", res)
    };
    let live_time = match res["data"][uid]["live_time"].take() {
        Value::Number(live_time) => live_time.as_u64().unwrap(),
        _ => panic!("{:?}", res)
    };
    let area_id = match res["data"][uid]["area_v2_id"].take() {
        Value::Number(area_id) => area_id.as_i64().unwrap(),
        _ => panic!("{:?}", res)
    };
    let area_name = match res["data"][uid]["area_v2_name"].take() {
        Value::String(area_name) => area_name,
        _ => panic!("{:?}", res)
    };
    let cover_url = match res["data"][uid]["cover_from_user"].take() {
        Value::String(cover_url) => cover_url,
        _ => panic!("{:?}", res)
    };
    Ok(((live_status, live_time), (area_id, area_name, cover_url)))
}

pub async fn live_area_list() -> Result<Vec<(String, Vec<(String, String)>)>, Box<dyn std::error::Error>> {
    let mut res: Value = serde_json::from_slice(
        reqwest::get("https://api.live.bilibili.com/room/v1/Area/getList")
        .await?.bytes().await?.as_ref()
    )?;
    let data = match res["data"].take() {
        Value::Array(data) => data,
        _ => panic!("{:?}", res)
    };
    Ok(data.into_iter().map(|mut e| {
        let name = match e["name"].take() {
            Value::String(name) => name,
            _ => panic!("{:?}", e)
        };
        let list = match e["list"].take() {
            Value::Array(item) => {
                item.into_iter().map(|mut v| (
                    match v["name"].take() {
                        Value::String(name) => name,
                        _ => panic!("{:?}", v)
                    },
                    match v["id"].take() {
                        Value::String(id) => id,
                        _ => panic!("{:?}", v)
                    }
                )).collect()
            },
            _ => panic!("{:?}", e)
        };
        (name, list)
    }).collect())
}

async fn post_live(cookies: &HashMap<String, String>, url: &'static str, area: Option<&str>) -> Result<bytes::Bytes, Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let csrf = cookies["bili_jct"].clone();
    let uid = cookies["DedeUserID"].as_str();
    let room_id = get_room_id(uid).await?;

    let mut data = HashMap::new();
    data.insert("room_id", room_id.as_str());
    data.insert("platform", "web_link");
    data.insert("csrf_token", &csrf);
    data.insert("csrf", &csrf);
    data.insert("visit_id", "");
    if let Some(area) = area {
        data.insert("area_v2", area);
    }

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::COOKIE,
        (cookies.into_iter().map(|(k, v)| format!("{k}={v};")).collect::<Vec<_>>().join(" ")).parse().unwrap(),
    );

    let resp = client.post(url)
        .headers(headers)
        .form(&data)
        .send().await?.bytes().await?;
    Ok(resp)
}

pub async fn start_live(cookies: &HashMap<String, String>, area: &str) -> Result<((String, String), String), Box<dyn std::error::Error>> {
    let resp = post_live(cookies, "https://api.live.bilibili.com/room/v1/Room/startLive", Some(area)).await?;
    let mut val: Value = serde_json::from_slice(resp.as_ref())?;
    let addr = match val["data"]["rtmp"]["addr"].take() {
        Value::String(addr) => addr,
        _ => panic!("{:?}", val)
    };
    let code = match val["data"]["rtmp"]["code"].take() {
        Value::String(addr) => addr,
        _ => panic!("{:?}", val)
    };
    let message = match val["message"].take() {
        Value::String(message) => message,
        _ => panic!("{:?}", val)
    };
    Ok(((addr, code), message))
}

pub async fn stop_live(cookies: &HashMap<String, String>) -> Result<String, Box<dyn std::error::Error>> {
    let resp = post_live(cookies, "https://api.live.bilibili.com/room/v1/Room/stopLive", None).await?;
    let mut val: Value = serde_json::from_slice(resp.as_ref())?;
    match val["message"].take() {
        Value::String(message) => Ok(message),
        _ => panic!("{:?}", val)
    }
}