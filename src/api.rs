use anyhow::{Result, anyhow};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::{Client, header};
use rsa::{Oaep, RsaPublicKey};
use serde::Deserialize;
use sha2::Sha256;
use std::collections::{BTreeMap, HashMap};
use std::io::Read;
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

    // --- Cookie Refresh Logic (Adapted from login.rs) ---

    // JWK Public Key components
    const REFRESH_KEY_N: &'static str = "y4HdjgJHBlbaBN04VERG4qNBIFHP6a3GozCl75AihQloSWCXC5HDNgyinEnhaQ_4-gaMud_GF50elYXLlCToR9se9Z8z433U3KjM-3Yx7ptKkmQNAMggQwAVKgq3zYAoidNEWuxpkY_mAitTSRLnsJW-NCTa0bqBFF6Wm1MxgfE";
    const REFRESH_KEY_E: &'static str = "AQAB";

    fn get_correspond_path(timestamp: u64) -> Result<String> {
        // Decode base64url encoded n and e
        let n_bytes = URL_SAFE_NO_PAD.decode(Self::REFRESH_KEY_N)?;
        let e_bytes = URL_SAFE_NO_PAD.decode(Self::REFRESH_KEY_E)?;

        // Create RSA Public Key
        let public_key = RsaPublicKey::new(
            rsa::BigUint::from_bytes_be(&n_bytes),
            rsa::BigUint::from_bytes_be(&e_bytes),
        )?;

        // Data to encrypt
        let data = format!("refresh_{}", timestamp);
        let data_bytes = data.as_bytes();

        // Encrypt using RSA-OAEP SHA-256
        let padding = Oaep::new::<Sha256>();
        let mut rng = rand::thread_rng();
        let encrypted_data = public_key.encrypt(&mut rng, padding, data_bytes)?;

        // Convert to hex string
        let encrypted_hex = encrypted_data
            .iter()
            .map(|byte| format!("{:02x}", byte))
            .collect::<String>();

        Ok(encrypted_hex)
    }

    pub async fn need_refresh(&self, csrf: &str) -> Result<Option<u64>> {
        let resp = self
            .client
            .get(format!(
                "https://passport.bilibili.com/x/passport-login/web/cookie/info?csrf={}",
                csrf
            ))
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        if let Some(true) = resp["data"]["refresh"].as_bool() {
            let timestamp = resp["data"]["timestamp"]
                .as_u64()
                .ok_or_else(|| anyhow!("timestamp missing or not u64"))?;
            Ok(Some(timestamp))
        } else {
            Ok(None)
        }
    }

    pub async fn get_refresh_csrf(&self, timestamp: u64) -> Result<String> {
        let correspond_path = Self::get_correspond_path(timestamp)?;

        let bytes = self
            .client
            .get(format!(
                "https://www.bilibili.com/correspond/1/{}",
                correspond_path
            ))
            .header(header::CONTENT_TYPE, "charset=GBK;")
            .send()
            .await?
            .bytes()
            .await?;

        let mut decoder = flate2::read::GzDecoder::new(std::io::Cursor::new(bytes));
        let mut decompressed_data = Vec::new();
        decoder.read_to_end(&mut decompressed_data)?;
        let res = String::from_utf8(decompressed_data)?;

        // Use scraper to extract key
        let html = scraper::Html::parse_document(&res);
        // The ID is "1-name", which needs escaping in CSS selector
        let selector = scraper::Selector::parse(r"#\31-name")
            .map_err(|e| anyhow!("Selector parse error: {:?}", e))?;

        let refresh_csrf = html
            .select(&selector)
            .next()
            .ok_or_else(|| anyhow!("cannot find #1-name"))?
            .text()
            .next()
            .ok_or_else(|| anyhow!("#1-name does not contain inner text"))?;

        Ok(refresh_csrf.to_owned())
    }

    // Returns the new refresh token and the new full cookie string
    pub async fn refresh_cookie(
        &self,
        csrf: &str,
        refresh_token: &str,
        old_cookie_str: &str,
    ) -> Result<(String, String)> {
        // 1. Check if need refresh
        let timestamp = match self.need_refresh(csrf).await? {
            Some(ts) => ts,
            None => return Err(anyhow!("Cookie does not need refresh")),
        };

        // 2. Get Refresh CSRF
        let refresh_csrf = self.get_refresh_csrf(timestamp).await?;

        // 3. Post Refresh
        let mut params = HashMap::new();
        params.insert("csrf", csrf.to_string());
        params.insert("refresh_csrf", refresh_csrf);
        params.insert("source", "main_web".to_string());
        params.insert("refresh_token", refresh_token.to_string());

        let resp = self
            .client
            .post("https://passport.bilibili.com/x/passport-login/web/cookie/refresh")
            .form(&params)
            .send()
            .await?;

        // Parse old cookies
        let mut cookies_map = HashMap::new();
        for pair in old_cookie_str.split(';') {
            let pair = pair.trim();
            if let Some((k, v)) = pair.split_once('=') {
                cookies_map.insert(k.to_string(), v.to_string());
            }
        }

        // Apply new cookies
        for cookie in resp.headers().get_all(header::SET_COOKIE) {
            if let Ok(c_str) = cookie.to_str() {
                if let Ok(parsed_cookie) = cookie::Cookie::parse(c_str) {
                    cookies_map.insert(
                        parsed_cookie.name().to_string(),
                        parsed_cookie.value().to_string(),
                    );
                }
            }
        }

        let body_bytes = resp.bytes().await?;
        let res_json: serde_json::Value = serde_json::from_slice(&body_bytes)?;

        if res_json["code"].as_i64().unwrap_or(-1) != 0 {
            return Err(anyhow!("Refresh failed: {:?}", res_json));
        }

        let new_refresh_token = res_json["data"]["refresh_token"]
            .as_str()
            .ok_or_else(|| anyhow!("No refresh_token in response"))?
            .to_string();

        let new_cookie_str = cookies_map
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("; ");

        // 4. Confirm Refresh
        self.confirm_refresh(csrf, refresh_token, &new_cookie_str)
            .await?;

        Ok((new_refresh_token, new_cookie_str))
    }

    pub async fn confirm_refresh(
        &self,
        csrf: &str,
        old_refresh_token: &str,
        cookie_str: &str,
    ) -> Result<()> {
        // Need a client with the NEW cookies
        let temp_client = Self::new_with_cookies(cookie_str);

        let mut params = HashMap::new();
        params.insert("csrf", csrf.to_string());
        params.insert("refresh_token", old_refresh_token.to_string());

        let resp = temp_client
            .client
            .post("https://passport.bilibili.com/x/passport-login/web/confirm/refresh")
            .form(&params)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        if resp["code"].as_i64().unwrap_or(-1) != 0 {
            return Err(anyhow!("Confirm refresh failed: {:?}", resp));
        }
        Ok(())
    }
}
