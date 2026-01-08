use anyhow::{Result, anyhow};
use reqwest::{Client, header};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::time::{SystemTime, UNIX_EPOCH};
use url::form_urlencoded;

const MIXIN_KEY_ENC_TAB: [usize; 64] = [
    46, 47, 18, 2, 53, 8, 23, 32, 15, 50, 10, 31, 58, 3, 45, 35, 27, 43, 5, 49, 33, 9, 42, 19, 29,
    28, 14, 39, 12, 38, 41, 13, 37, 48, 7, 16, 24, 55, 40, 61, 26, 17, 0, 1, 60, 51, 30, 4, 22, 25,
    54, 21, 56, 59, 6, 63, 57, 62, 11, 36, 20, 34, 44, 52,
];

const APP_KEY: &str = "aae92bc66f3edfab";
const APP_SEC: &str = "af125a0d5279fd576c1b4418a3e8276d";

#[derive(Clone)]
pub struct BiliApi {
    client: Client,
}

// Response Structures
#[derive(Deserialize, Debug)]
struct WbiImg {
    img_url: String,
    sub_url: String,
}

#[derive(Deserialize, Debug)]
struct NavData {
    wbi_img: WbiImg,
    // is_login: bool, // unused currently
}

#[derive(Deserialize, Debug)]
struct NavResponse {
    data: Option<NavData>,
}

#[derive(Deserialize, Debug)]
struct CommonResponse {
    code: i32,
    message: Option<String>,
    msg: Option<String>,
}

impl CommonResponse {
    fn is_success(&self) -> bool {
        self.code == 0
    }
    fn error_msg(&self) -> String {
        self.message
            .clone()
            .or(self.msg.clone())
            .unwrap_or_else(|| format!("Unknown error code: {}", self.code))
    }
}

#[derive(Deserialize)]
struct RtmpInfo {
    addr: String,
    code: String,
}

#[derive(Deserialize)]
struct StartLiveData {
    rtmp: RtmpInfo,
}

#[derive(Deserialize)]
struct StartLiveResponse {
    #[serde(rename = "code")]
    _code: i32,
    #[serde(rename = "msg")]
    _msg: Option<String>,
    #[serde(rename = "message")]
    _message: Option<String>,
    data: Option<StartLiveData>,
}

impl BiliApi {
    pub fn new() -> Self {
        Self::new_with_cookies("")
    }

    pub fn new_with_cookies(cookie_str: &str) -> Self {
        let mut headers = header::HeaderMap::new();
        if let Ok(val) = header::HeaderValue::from_str(cookie_str) {
            headers.insert(header::COOKIE, val);
        }

        let client = Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/58.0.3029.110 Safari/537.3")
            .default_headers(headers)
            .build()
            .unwrap();

        Self { client }
    }

    fn get_mixin_key(orig: &str) -> String {
        MIXIN_KEY_ENC_TAB
            .iter()
            .filter_map(|&i| orig.chars().nth(i))
            .take(32)
            .collect()
    }

    fn app_sign(params: &mut HashMap<String, String>) {
        params.insert("appkey".to_string(), APP_KEY.to_string());

        // Sort params using BTreeMap
        let sorted_params: BTreeMap<&String, &String> = params.iter().collect();

        let encoded_query = form_urlencoded::Serializer::new(String::new())
            .extend_pairs(sorted_params)
            .finish();

        let sign_str = format!("{}{}", encoded_query, APP_SEC);
        let sign = format!("{:x}", md5::compute(sign_str.as_bytes()));
        params.insert("sign".to_string(), sign);
    }

    pub async fn get_server_time(&self) -> Result<i64> {
        #[derive(Deserialize)]
        struct TimeData {
            now: i64,
        }
        #[derive(Deserialize)]
        struct TimeResp {
            data: TimeData,
        }

        let resp = self
            .client
            .get("https://api.bilibili.com/x/report/click/now")
            .send()
            .await?
            .json::<TimeResp>()
            .await?;
        Ok(resp.data.now)
    }

    pub async fn start_live(
        &self,
        room_id: &str,
        csrf: &str,
        area_id: &str,
        build: &str,
        version: &str,
        ts: i64,
    ) -> Result<(String, String)> {
        // 3. Start Live
        let mut start_data = HashMap::from([
            ("room_id".to_string(), room_id.to_string()),
            ("platform".to_string(), "pc_link".to_string()),
            ("area_v2".to_string(), area_id.to_string()),
            ("backup_stream".to_string(), "0".to_string()),
            ("csrf".to_string(), csrf.to_string()),
            ("csrf_token".to_string(), csrf.to_string()),
            ("build".to_string(), build.to_string()),
            ("version".to_string(), version.to_string()),
            ("ts".to_string(), ts.to_string()),
        ]);

        Self::app_sign(&mut start_data);

        let resp_json = self
            .client
            .post("https://api.live.bilibili.com/room/v1/Room/startLive")
            .form(&start_data)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        // Manual check for code to handle special error cases like face auth
        if let Some(code) = resp_json["code"].as_i64() {
            if code != 0 {
                // Check for face auth requirement
                let msg = resp_json["msg"]
                    .as_str()
                    .or(resp_json["message"].as_str())
                    .unwrap_or("Unknown error");
                if code == 60024 || msg.contains("人脸") {
                    // 60024 is typical for face auth needed
                    let qr = resp_json["data"]["qr"].as_str().unwrap_or("");
                    return Err(anyhow!("需要人脸认证: {}", qr));
                }
                return Err(anyhow!("开始直播失败 ({}): {}", code, msg));
            }
        }

        // Use typed deserialization for the happy path data
        let resp: StartLiveResponse = serde_json::from_value(resp_json)?;

        if let Some(data) = resp.data {
            Ok((data.rtmp.addr, data.rtmp.code))
        } else {
            Err(anyhow!("Missing data in response"))
        }
    }

    pub async fn get_live_version(&self) -> Result<(String, String)> {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut v_data = HashMap::from([("ts".to_string(), ts.to_string())]);
        Self::app_sign(&mut v_data);

        let query = form_urlencoded::Serializer::new(String::new())
            .extend_pairs(&v_data)
            .finish();
        let v_url = format!(
            "https://api.live.bilibili.com/xlive/app-blink/v1/liveVersionInfo/getHomePageLiveVersion?{}",
            query
        );

        let v_resp = self
            .client
            .get(&v_url)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        // Helper to safely get string from number or string field
        let safe_str = |v: &serde_json::Value| -> String {
            if v.is_number() {
                v.to_string()
            } else {
                v.as_str().unwrap_or("0").to_string()
            }
        };

        if let Some(data) = v_resp.get("data") {
            Ok((safe_str(&data["build"]), safe_str(&data["curr_version"])))
        } else {
            Ok(("0".to_string(), "0".to_string()))
        }
    }

    pub async fn update_room_title(&self, room_id: &str, csrf: &str, title: &str) -> Result<()> {
        let title_data = HashMap::from([
            ("room_id", room_id),
            ("csrf", csrf),
            ("csrf_token", csrf),
            ("title", title),
        ]);

        self.client
            .post("https://api.live.bilibili.com/room/v1/Room/update")
            .form(&title_data)
            .send()
            .await?;
        Ok(())
    }

    pub async fn stop_live(&self, room_id: &str, csrf: &str) -> Result<()> {
        let data = HashMap::from([
            ("room_id", room_id),
            ("csrf", csrf),
            ("csrf_token", csrf),
            ("platform", "pc_link"),
        ]);

        let resp = self
            .client
            .post("https://api.live.bilibili.com/room/v1/Room/stopLive")
            .form(&data)
            .send()
            .await?
            .json::<CommonResponse>()
            .await?;

        if !resp.is_success() {
            return Err(anyhow!("停止直播失败: {}", resp.error_msg()));
        }
        Ok(())
    }

    async fn get_wbi_keys(&self) -> Result<(String, String)> {
        let resp = self
            .client
            .get("https://api.bilibili.com/x/web-interface/nav")
            .send()
            .await?
            .json::<NavResponse>()
            .await?;

        let data = resp.data.ok_or_else(|| anyhow!("Failed to get nav data"))?;
        let img_url = data.wbi_img.img_url;
        let sub_url = data.wbi_img.sub_url;

        let img_key = img_url
            .rsplit('/')
            .next()
            .unwrap_or("")
            .split('.')
            .next()
            .unwrap_or("")
            .to_string();
        let sub_key = sub_url
            .rsplit('/')
            .next()
            .unwrap_or("")
            .split('.')
            .next()
            .unwrap_or("")
            .to_string();

        Ok((img_key, sub_key))
    }

    pub fn enc_wbi(params: &mut HashMap<String, String>, img_key: &str, sub_key: &str) -> String {
        let mixin_key = Self::get_mixin_key(&format!("{}{}", img_key, sub_key));
        let curr_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        params.insert("wts".to_string(), curr_time.to_string());

        let mut sorted_keys: Vec<&String> = params.keys().collect();
        sorted_keys.sort();

        let mut query_str = String::new();
        for key in sorted_keys {
            let val = params.get(key).unwrap();
            // Filter characters "!'()*"
            let filtered_val: String = val.chars().filter(|c| !"!'()*".contains(*c)).collect();

            if !query_str.is_empty() {
                query_str.push('&');
            }
            query_str.push_str(&format!(
                "{}={}",
                key,
                form_urlencoded::byte_serialize(filtered_val.as_bytes()).collect::<String>()
            ));
        }

        let to_hash = format!("{}{}", query_str, mixin_key);
        let w_rid = format!("{:x}", md5::compute(to_hash));

        format!("{}&w_rid={}", query_str, w_rid)
    }

    pub async fn send_bullet(
        &self,
        msg: &str,
        room_id: &str,
        csrf: &str,
    ) -> Result<(bool, String)> {
        let (img_key, sub_key) = self.get_wbi_keys().await?;

        let mut query_params = HashMap::new();
        query_params.insert("web_location".to_string(), "444.8".to_string());
        let query_string = Self::enc_wbi(&mut query_params, &img_key, &sub_key);

        let rnd = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .to_string();
        let form_data = HashMap::from([
            ("bubble", "0"),
            ("msg", msg),
            ("color", "16777215"),
            ("mode", "1"),
            ("fontsize", "25"),
            ("rnd", &rnd),
            ("roomid", room_id),
            ("csrf", csrf),
            ("csrf_token", csrf),
        ]);

        let url = format!("https://api.live.bilibili.com/msg/send?{}", query_string);

        let resp = self.client.post(&url).form(&form_data).send().await?;

        let text = resp.text().await?;
        // Parse simple JSON response
        #[derive(Deserialize)]
        struct BulletResp {
            code: i32,
            message: Option<String>,
            msg: Option<String>,
        }

        if let Ok(json) = serde_json::from_str::<BulletResp>(&text) {
            match json.code {
                0 => Ok((true, "发送成功".to_string())),
                1003212 => Ok((false, "超出限制长度".to_string())),
                -101 => Ok((false, "未登录".to_string())),
                10031 => Ok((false, "发送频率过高".to_string())),
                _ => Ok((
                    false,
                    format!(
                        "错误: {} ({})",
                        json.message.or(json.msg).unwrap_or("?".into()),
                        json.code
                    ),
                )),
            }
        } else {
            Ok((false, format!("Failed to parse: {}", text)))
        }
    }

    pub async fn get_qrcode(&self) -> Result<(String, String)> {
        let resp = self
            .client
            .get("https://passport.bilibili.com/x/passport-login/web/qrcode/generate")
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        let url = resp["data"]["url"].as_str().unwrap_or("").to_string();
        let key = resp["data"]["qrcode_key"]
            .as_str()
            .unwrap_or("")
            .to_string();
        Ok((url, key))
    }

    pub async fn poll_qrcode(&self, key: &str) -> Result<serde_json::Value> {
        let resp = self
            .client
            .get("https://passport.bilibili.com/x/passport-login/web/qrcode/poll")
            .query(&[("qrcode_key", key)])
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;
        Ok(resp)
    }

    pub async fn get_partition(&self) -> Result<serde_json::Value> {
        let resp = self
            .client
            .get("https://api.live.bilibili.com/room/v1/Area/getList?show_pinyin=1")
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;
        Ok(resp)
    }

    pub async fn get_room_id(&self, mid: i64) -> Result<i64> {
        let url = format!(
            "https://api.live.bilibili.com/room/v1/Room/get_status_info_by_uids?uids[]={}",
            mid
        );
        let resp = self
            .client
            .get(&url)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        if let Some(user_data) = resp.get("data").and_then(|d| d.get(mid.to_string())) {
            if let Some(room_id) = user_data.get("room_id") {
                return Ok(room_id.as_i64().unwrap_or(0));
            }
        }

        Err(anyhow!("获取直播间ID失败"))
    }

    pub async fn get_room_title(&self, room_id: &str) -> Result<String> {
        let url = format!(
            "https://api.live.bilibili.com/room/v1/Room/get_info?room_id={}",
            room_id
        );
        let resp = self
            .client
            .get(&url)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        if let Some(title) = resp.get("data").and_then(|d| d.get("title")) {
            return Ok(title.as_str().unwrap_or("").to_string());
        }

        Err(anyhow!("获取房间标题失败"))
    }
}
