use base64::{URL_SAFE_NO_PAD, decode_config};
use qrcode::QrCode;
use qrcode::render::unicode;
use rsa::{PaddingScheme, PublicKey, RsaPublicKey};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Sha256;
use std::{collections::HashMap, io::Read, path::Path};
use tokio::time::{Duration, sleep};

pub async fn generate_qr() -> Result<(String, String), Box<dyn std::error::Error>> {
    let mut res: Value = serde_json::from_slice(
        reqwest::get("https://passport.bilibili.com/x/passport-login/web/qrcode/generate")
            .await?
            .bytes()
            .await?
            .as_ref(),
    )?;
    let url = match res["data"]["url"].take() {
        Value::String(url) => url,
        _ => panic!("{:?}", res),
    };
    let token = match res["data"]["qrcode_key"].take() {
        Value::String(url) => url,
        _ => panic!("{:?}", res),
    };
    let qr = QrCode::new(url)?.render::<unicode::Dense1x2>().build();
    Ok((qr, token))
}

#[derive(Debug)]
pub enum LoginStatus {
    NotScanned,
    Scanned,
    Success((String, String)),
    OutofDate,
}

pub async fn check_login_status(token: &str) -> Result<LoginStatus, Box<dyn std::error::Error>> {
    let mut res: Value = serde_json::from_slice(
        reqwest::get(format!(
            "https://passport.bilibili.com/x/passport-login/web/qrcode/poll?qrcode_key={}",
            token
        ))
        .await?
        .bytes()
        .await?
        .as_ref(),
    )?;
    let status: u64 = match res["data"]["code"].take() {
        Value::Number(num) => num.as_u64().unwrap(),
        _ => panic!("{:?}", res),
    };

    match status {
        86101 => Ok(LoginStatus::NotScanned),
        86090 => Ok(LoginStatus::Scanned),
        0 => {
            return Ok(LoginStatus::Success((
                match res["data"]["refresh_token"].take() {
                    Value::String(refresh_token) => refresh_token,
                    _ => panic!("{:?}", res),
                },
                match res["data"]["url"].take() {
                    Value::String(url) => url,
                    _ => panic!("{:?}", res),
                },
            )));
        }
        _ => Ok(LoginStatus::OutofDate),
    }
}

pub async fn login() -> Result<(String, String), Box<dyn std::error::Error>> {
    let (qr, token) = generate_qr().await?;
    println!("{}", qr);

    let mut sleep_sec = 500;
    loop {
        match check_login_status(&token).await? {
            LoginStatus::NotScanned => {}
            LoginStatus::Scanned => println!("QR Code Scanned"),
            LoginStatus::Success(res) => return Ok(res),
            LoginStatus::OutofDate => return Err("QR Code Out of Date".into()),
        }
        sleep(Duration::from_millis(sleep_sec)).await;
        sleep_sec = std::cmp::min(2000, sleep_sec * 2);
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct LoginData {
    pub cookies: HashMap<String, String>,
    pub refresh_token: String,
    pub last_run: (i32, u32, u32),
    pub area: Option<String>,
}

impl LoginData {
    pub fn dump<P: AsRef<Path>>(self: &Self, fname: P) -> Result<(), Box<dyn std::error::Error>> {
        let file = std::fs::File::create(fname)?;
        let writer = std::io::BufWriter::new(file);
        serde_json::to_writer(writer, &self)?;
        Ok(())
    }

    pub fn load<P: AsRef<Path>>(fname: P) -> Result<LoginData, Box<dyn std::error::Error>> {
        let file = std::fs::File::open(fname)?;
        let reader = std::io::BufReader::new(file);
        let result: LoginData = serde_json::from_value(serde_json::from_reader(reader)?)?;
        Ok(result)
    }

    pub async fn refresh_cookie(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(timestamp) = self.need_refresh().await? {
            let old_refresh_token = self.post_cookie_refresh(timestamp).await?;
            let res = self.confirm_refresh(old_refresh_token).await?;
            if res["code"].as_i64().ok_or("code is not an integer")? != 0 {
                Err(format!("{:?}", res))?;
            }
        }
        Ok(())
    }

    async fn need_refresh(&self) -> Result<Option<usize>, Box<dyn std::error::Error>> {
        let client = reqwest::Client::new();
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::COOKIE,
            (self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v};"))
                .collect::<Vec<_>>()
                .join(" "))
            .parse()
            .unwrap(),
        );
        let csrf = self.cookies["bili_jct"].as_str();
        let res: Value = serde_json::from_slice(
            client
                .get(format!(
                    "https://passport.bilibili.com/x/passport-login/web/cookie/info?csrf={}",
                    csrf
                ))
                .headers(headers)
                .send()
                .await?
                .bytes()
                .await?
                .as_ref(),
        )?;
        let refresh = res["data"]["refresh"]
            .as_bool()
            .ok_or("refresh is not a Boolean")?;
        if refresh {
            let timestamp = res["data"]["timestamp"]
                .as_u64()
                .ok_or("timestamp is not a Number")? as usize;
            Ok(Some(timestamp))
        } else {
            Ok(None)
        }
    }

    fn get_correspond_path(timestamp: u128) -> Result<String, Box<dyn std::error::Error>> {
        // JWK 公钥的组件
        let n = "y4HdjgJHBlbaBN04VERG4qNBIFHP6a3GozCl75AihQloSWCXC5HDNgyinEnhaQ_4-gaMud_GF50elYXLlCToR9se9Z8z433U3KjM-3Yx7ptKkmQNAMggQwAVKgq3zYAoidNEWuxpkY_mAitTSRLnsJW-NCTa0bqBFF6Wm1MxgfE";
        let e = "AQAB";

        // 解码 base64url 编码的 n 和 e
        let n_bytes = decode_config(n, URL_SAFE_NO_PAD)?;
        let e_bytes = decode_config(e, URL_SAFE_NO_PAD)?;

        // 创建 RSA 公钥
        let public_key = RsaPublicKey::new(
            rsa::BigUint::from_bytes_be(&n_bytes),
            rsa::BigUint::from_bytes_be(&e_bytes),
        )?;

        // 创建要加密的数据
        let data = format!("refresh_{}", timestamp);
        let data_bytes = data.as_bytes();

        // 使用 RSA-OAEP SHA-256 进行加密
        let padding = PaddingScheme::new_oaep::<Sha256>();
        let mut rng = rand::thread_rng();
        let encrypted_data = public_key.encrypt(&mut rng, padding, data_bytes)?;

        // 将加密结果转换为十六进制字符串
        let encrypted_hex = encrypted_data
            .iter()
            .map(|byte| format!("{:02x}", byte))
            .collect::<String>();

        Ok(encrypted_hex)
    }

    async fn get_refresh_csrf(
        &self,
        timestamp: usize,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let correspond_path = Self::get_correspond_path(timestamp as u128)?;

        let client = reqwest::Client::new();
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::COOKIE,
            (self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v};"))
                .collect::<Vec<_>>()
                .join(" "))
            .parse()
            .unwrap(),
        );
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            "charset=GBK;".parse().unwrap(),
        );

        let bytes = client
            .get(format!(
                "https://www.bilibili.com/correspond/1/{}",
                correspond_path
            ))
            .headers(headers)
            .send()
            .await?
            .bytes()
            .await?;

        let mut decoder = flate2::read::GzDecoder::new(std::io::Cursor::new(bytes));
        let mut decompressed_data = Vec::new();
        decoder.read_to_end(&mut decompressed_data)?;
        let res = String::from_utf8(decompressed_data)?;

        let html = Html::parse_document(&res);
        let selector = Selector::parse(r"#\31-name")?; // css escape 1 -> \31
        let refresh_csrf = html
            .select(&selector)
            .next()
            .ok_or("cannot find #1-name")?
            .text()
            .next()
            .ok_or("#1-name does not contain inner text")?;
        Ok(refresh_csrf.to_owned())
    }

    async fn post_cookie_refresh(
        &mut self,
        timestamp: usize,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let client = reqwest::Client::new();

        let mut data = HashMap::new();
        data.insert("csrf", self.cookies["bili_jct"].to_owned());
        data.insert("refresh_csrf", self.get_refresh_csrf(timestamp).await?);
        data.insert("source", "main_web".to_owned());
        data.insert("refresh_token", self.refresh_token.to_owned());

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::COOKIE,
            (self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v};"))
                .collect::<Vec<_>>()
                .join(" "))
            .parse()
            .unwrap(),
        );

        let resp = client
            .post("https://passport.bilibili.com/x/passport-login/web/cookie/refresh")
            .headers(headers)
            .form(&data)
            .send()
            .await?;

        for cookie in resp.headers().get_all(reqwest::header::SET_COOKIE) {
            let parsed_cookie = cookie::Cookie::parse(cookie.to_str()?)?;
            self.cookies.insert(
                parsed_cookie.name().to_owned(),
                parsed_cookie.value().to_owned(),
            );
        }

        let mut res: Value = serde_json::from_slice(resp.bytes().await?.as_ref())?;
        let mut old_refresh_token = match res["data"]["refresh_token"].take() {
            Value::String(s) => s,
            _ => panic!("{:?}", res),
        };
        std::mem::swap(&mut self.refresh_token, &mut old_refresh_token);
        Ok(old_refresh_token)
    }

    async fn confirm_refresh(
        &self,
        refresh_token_old: String,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        let client = reqwest::Client::new();

        let mut data = HashMap::new();
        data.insert("csrf", self.cookies["bili_jct"].to_owned());
        data.insert("refresh_token", refresh_token_old);

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::COOKIE,
            (self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v};"))
                .collect::<Vec<_>>()
                .join(" "))
            .parse()
            .unwrap(),
        );

        let res: Value = serde_json::from_slice(
            client
                .post("https://passport.bilibili.com/x/passport-login/web/confirm/refresh")
                .headers(headers)
                .form(&data)
                .send()
                .await?
                .bytes()
                .await?
                .as_ref(),
        )?;

        Ok(res)
    }
}
