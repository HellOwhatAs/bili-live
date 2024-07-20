use qrcode::QrCode;
use qrcode::render::unicode;
use tokio::time::{sleep, Duration};
use serde_json::Value;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::Path};

pub async fn generate_qr() -> Result<(String, String), Box<dyn std::error::Error>> {
    let mut res: Value = serde_json::from_slice(
        reqwest::get("https://passport.bilibili.com/x/passport-login/web/qrcode/generate")
        .await?.bytes().await?.as_ref()
    )?;
    let url = match res["data"]["url"].take() {
        Value::String(url) => url,
        _ => panic!("{:?}", res)
    };
    let token = match res["data"]["qrcode_key"].take() {
        Value::String(url) => url,
        _ => panic!("{:?}", res)
    };
    let qr_svg = QrCode::new(url)?.render::<unicode::Dense1x2>().build();
    Ok((qr_svg, token))
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
        reqwest::get(format!("https://passport.bilibili.com/x/passport-login/web/qrcode/poll?qrcode_key={}", token))
        .await?.bytes().await?.as_ref()
    )?;
    let status: u64 = match res["data"]["code"].take() {
        Value::Number(num) => num.as_u64().unwrap(),
        _ => panic!("{:?}", res)
    };

    match status {
        86101 => Ok(LoginStatus::NotScanned),
        86090 => Ok(LoginStatus::Scanned),
        0 => {
            return Ok(LoginStatus::Success((
                match res["data"]["refresh_token"].take() {
                    Value::String(refresh_token) => refresh_token,
                    _ => panic!("{:?}", res)
                },
                match res["data"]["url"].take() {
                    Value::String(url) => url,
                    _ => panic!("{:?}", res)
                }
            )));
        }
        _ => Ok(LoginStatus::OutofDate)
    }
}

pub async fn login() -> Result<(String, String), Box<dyn std::error::Error>> {
    let (qr_svg, token) = generate_qr().await?;
    println!("{}", qr_svg);

    let mut sleep_sec = 500;
    loop {
        match check_login_status(&token).await? {
            LoginStatus::NotScanned => {},
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
    pub area: Option<String>
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
}
