use crate::api::BiliApi;
use crate::config::{self, LiveSettings, UserCookies};
use crate::types::ParentPartition;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use url::Url;

pub trait RepaintSignal: Send + Sync {
    fn request_repaint(&self);
}

// A no-op implementation for when we don't have a UI yet or for testing
struct NoOpRepaintSignal;
impl RepaintSignal for NoOpRepaintSignal {
    fn request_repaint(&self) {}
}

pub enum AppMessage {
    Log(String),
    QrCodeGenerated(String, String), // url, key
    QrCodePollResult(bool, String, Option<UserCookies>),
    CookieRefreshed(String, String), // new_refresh_token, new_cookie_str
    PartitionLoaded(Vec<ParentPartition>), // Parsed data
    StreamInfo(String, String),
    BulletSent(bool, String),
    RoomTitleUpdated(bool, String),
    RoomTitleFetched(String),
    LiveVersionFetched(String, String),
    ServerTimeSynced(i64),
    StopLiveResult(bool, String),
    FaceAuthQrRequired(String), // content
    ParseLoginUrlResult(bool, String, Option<UserCookies>),
}

pub struct LiveState {
    pub live_url: String,
    pub live_code: String,
    pub is_live: bool,
    pub live_build: String,
    pub live_version: String,
    pub time_offset: i64,
}

impl Default for LiveState {
    fn default() -> Self {
        Self {
            live_url: String::new(),
            live_code: String::new(),
            is_live: false,
            live_build: "0".to_string(),
            live_version: "0".to_string(),
            time_offset: 0,
        }
    }
}

pub enum UiEffect {
    ShowQrReceived(String, String), // url, key
    FaceAuthQrReceived(String),     // content
    HideQrWindow,
    SwitchToResultTab,
}

pub struct BiliLiveContext {
    pub rt: Runtime,
    tx: mpsc::Sender<AppMessage>,
    rx: mpsc::Receiver<AppMessage>,
    repaint_signal: Arc<dyn RepaintSignal>,

    pub api: BiliApi,
    pub cookies: Option<UserCookies>,
    pub live_settings: LiveSettings,
    pub partitions: Vec<ParentPartition>,
    pub live_state: LiveState,

    // UI-agnostic logs
    pub log_messages: String,
}

impl BiliLiveContext {
    pub fn new(repaint_signal: Option<Arc<dyn RepaintSignal>>) -> Self {
        let (tx, rx) = mpsc::channel(100);
        let rt = Runtime::new().expect("Failed to create Tokio runtime");

        let signal = repaint_signal.unwrap_or_else(|| Arc::new(NoOpRepaintSignal));

        let mut ctx = Self {
            rt,
            tx,
            rx,
            repaint_signal: signal,
            api: BiliApi::new(),
            cookies: None,
            live_settings: config::load_last_settings(),
            partitions: Vec::new(),
            live_state: LiveState::default(),
            log_messages: String::new(),
        };

        // Load initial data
        ctx.fetch_live_version();
        ctx.fetch_server_time();
        ctx.load_local_cookies();

        ctx
    }

    pub fn set_cookies(&mut self, cookies: UserCookies) {
        self.api = BiliApi::new_with_cookies(&cookies.cookie_str);
        self.fetch_room_title(cookies.room_id.clone());
        self.cookies = Some(cookies);
        self.log("已加载 Cookies".to_string());
    }

    pub fn log(&mut self, msg: String) {
        self.log_messages.push_str(&format!("{}\n", msg));
    }

    pub fn process_messages(&mut self) -> Vec<UiEffect> {
        let mut effects = Vec::new();
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                AppMessage::Log(s) => self.log(s),
                AppMessage::QrCodeGenerated(url, key) => {
                    effects.push(UiEffect::ShowQrReceived(url.clone(), key.clone()));
                    self.poll_qr_login(key); // Start polling automatically
                }
                AppMessage::QrCodePollResult(success, msg, cookies) => {
                    self.log(msg);
                    if success {
                        effects.push(UiEffect::HideQrWindow);
                        if let Some(c) = cookies {
                            config::save_cookies(&c);
                            self.set_cookies(c);
                            self.fetch_partition();
                        }
                    }
                }
                AppMessage::PartitionLoaded(data) => {
                    self.partitions = data;
                    self.log("分区数据已更新".to_string());
                }
                AppMessage::StreamInfo(url, code) => {
                    self.live_state.live_url = url;
                    self.live_state.live_code = code;
                    self.live_state.is_live = true;
                    effects.push(UiEffect::SwitchToResultTab);
                    self.log("直播开启成功".to_string());
                }
                AppMessage::StopLiveResult(success, msg) => {
                    self.log(msg);
                    if success {
                        self.live_state.is_live = false;
                    }
                }
                AppMessage::RoomTitleUpdated(_success, msg) => {
                    self.log(msg);
                }
                AppMessage::RoomTitleFetched(title) => {
                    self.live_settings.title = title;
                    self.log("房间标题已同步".to_string());
                }
                AppMessage::LiveVersionFetched(b, v) => {
                    self.live_state.live_build = b;
                    self.live_state.live_version = v;
                }
                AppMessage::ServerTimeSynced(offset) => {
                    self.live_state.time_offset = offset;
                    self.log(format!("服务器时间已同步 (Offset: {}s)", offset));
                }
                AppMessage::FaceAuthQrRequired(content) => {
                    effects.push(UiEffect::FaceAuthQrReceived(content));
                    self.log("检测到需要人脸认证".to_string());
                }
                AppMessage::BulletSent(success, msg) => {
                    if success {
                        self.log(format!("弹幕发送成功: {}", msg));
                    } else {
                        self.log(format!("弹幕发送失败: {}", msg));
                    }
                }
                AppMessage::ParseLoginUrlResult(success, msg, cookies) => {
                    // Reuse qrcode poll result logic
                    self.tx
                        .try_send(AppMessage::QrCodePollResult(success, msg, cookies))
                        .ok();
                }
                AppMessage::CookieRefreshed(new_refresh_token, new_cookie_str) => {
                    if let Some(c) = &mut self.cookies {
                        c.cookie_str = new_cookie_str.clone();
                        c.refresh_token = new_refresh_token.clone();
                        config::save_cookies(c);
                        self.log("Cookie刷新成功".to_string());
                    }
                }
            }
        }
        effects
    }

    // --- Actions ---

    pub fn load_local_cookies(&mut self) {
        if let Some(c) = config::load_cookies() {
            self.set_cookies(c);
            self.refresh_cookies();
        } else {
            self.log("未找到cookies.txt".to_string());
        }
        self.fetch_partition();
    }

    pub fn fetch_live_version(&self) {
        let tx = self.tx.clone();
        let api = self.api.clone();
        let signal = self.repaint_signal.clone();
        self.rt.spawn(async move {
            if let Ok((b, v)) = api.get_live_version().await {
                let _ = tx.send(AppMessage::LiveVersionFetched(b, v)).await;
                signal.request_repaint();
            }
        });
    }

    pub fn fetch_partition(&self) {
        let tx = self.tx.clone();
        let api = self.api.clone();
        let signal = self.repaint_signal.clone();
        self.rt.spawn(async move {
            match api.get_partition().await {
                Ok(data) => {
                    // Logic moved from main.rs receive handler to here (or somewhere).
                    // Wait, api returns serde_json::Value. Parsing should happen here on background thread?
                    // Better to parse here to offload UI thread.
                    if let Some(d) = data.get("data") {
                        if let Ok(parsed) =
                            serde_json::from_value::<Vec<ParentPartition>>(d.clone())
                        {
                            let _ = tx.send(AppMessage::PartitionLoaded(parsed)).await;
                        } else {
                            let _ = tx
                                .send(AppMessage::Log("分区数据解析失败".to_string()))
                                .await;
                        }
                    } else {
                        let _ = tx
                            .send(AppMessage::Log("分区响应格式错误".to_string()))
                            .await;
                    }
                }
                Err(_) => {
                    let _ = tx.send(AppMessage::Log("获取分区失败".to_string())).await;
                }
            }
            signal.request_repaint();
        });
    }

    pub fn fetch_qrcode(&self) {
        let tx = self.tx.clone();
        let api = self.api.clone();
        let signal = self.repaint_signal.clone();

        self.rt.spawn(async move {
            match api.get_qrcode().await {
                Ok((url, key)) => {
                    let _ = tx.send(AppMessage::QrCodeGenerated(url, key)).await;
                }
                Err(e) => {
                    let _ = tx
                        .send(AppMessage::Log(format!("获取二维码失败: {}", e)))
                        .await;
                }
            }
            signal.request_repaint();
        });
    }

    fn poll_qr_login(&self, key: String) {
        let tx = self.tx.clone();
        let api = self.api.clone();
        let signal = self.repaint_signal.clone();

        self.rt.spawn(async move {
            for _ in 0..180 {
                if let Ok(resp) = api.poll_qrcode(&key).await {
                    if let Some(code) = resp["data"]["code"].as_i64() {
                        match code {
                            0 => {
                                let url = resp["data"]["url"].as_str().unwrap_or("");
                                let refresh_token =
                                    resp["data"]["refresh_token"].as_str().unwrap_or("");
                                // Parse URL Logic
                                let parsed_msg = Self::parse_login_url(url, refresh_token).await;
                                let _ = tx.send(parsed_msg).await;
                                break;
                            }
                            86038 => {
                                let _ = tx
                                    .send(AppMessage::QrCodePollResult(
                                        false,
                                        "二维码已过期".to_string(),
                                        None,
                                    ))
                                    .await;
                                break;
                            }
                            _ => {}
                        }
                    }
                }
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                // Don't need to repaint every second if nothing changed?
                // But user might want to see "polling..." status?
                // Currently status doesn't change during polling.
            }
            signal.request_repaint();
        });
    }

    async fn parse_login_url(url: &str, refresh_token: &str) -> AppMessage {
        if let Ok(parsed) = Url::parse(url) {
            let params: std::collections::HashMap<_, _> =
                parsed.query_pairs().into_owned().collect();
            let sessdata = params.get("SESSDATA").cloned().unwrap_or_default();
            let bili_jct = params.get("bili_jct").cloned().unwrap_or_default();
            let dede_userid = params.get("DedeUserID").cloned().unwrap_or_default();

            let cookie_str = format!(
                "SESSDATA={}; bili_jct={}; DedeUserID={}",
                sessdata, bili_jct, dede_userid
            );

            let mid = dede_userid.parse::<i64>().unwrap_or(0);
            let temp_api = BiliApi::new_with_cookies(&cookie_str);
            let room_id = temp_api.get_room_id(mid).await.unwrap_or(0).to_string();

            let cookies = UserCookies {
                room_id,
                cookie_str,
                csrf: bili_jct,
                refresh_token: refresh_token.to_string(),
            };
            return AppMessage::ParseLoginUrlResult(true, "登录成功".to_string(), Some(cookies));
        }
        AppMessage::ParseLoginUrlResult(false, "解析登录URL失败".to_string(), None)
    }

    pub fn start_live(&mut self) {
        if self.cookies.is_none() {
            self.log("请先登录".to_string());
            return;
        }
        if let Some(c) = &self.cookies {
            let api = self.api.clone();
            let tx = self.tx.clone();
            let signal = self.repaint_signal.clone();

            let room_id = c.room_id.clone();
            let csrf = c.csrf.clone();
            let area_id = self.live_settings.area_id.clone();
            let build = self.live_state.live_build.clone();
            let version = self.live_state.live_version.clone();
            let offset = self.live_state.time_offset;

            // Save settings
            config::save_last_settings(&self.live_settings);

            self.rt.spawn(async move {
                let _ = tx
                    .send(AppMessage::Log("正在启动直播...".to_string()))
                    .await;
                signal.request_repaint();

                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64;
                let ts = now + offset;

                match api
                    .start_live(&room_id, &csrf, &area_id, &build, &version, ts)
                    .await
                {
                    Ok((url, code)) => {
                        let _ = tx.send(AppMessage::StreamInfo(url, code)).await;
                    }
                    Err(e) => {
                        let err_str = e.to_string();
                        if err_str.contains("人脸") {
                            let qr_content = err_str
                                .split("需要人脸认证: ")
                                .nth(1)
                                .unwrap_or("")
                                .to_string();
                            let msg = if !qr_content.is_empty() {
                                AppMessage::FaceAuthQrRequired(qr_content)
                            } else {
                                AppMessage::Log("需要人脸认证但无法获取二维码".to_string())
                            };
                            let _ = tx.send(msg).await;
                        } else {
                            let _ = tx.send(AppMessage::Log(format!("错误: {}", e))).await;
                        }
                    }
                }
                signal.request_repaint();
            });
        }
    }

    pub fn stop_live(&self) {
        if let Some(c) = &self.cookies {
            let api = self.api.clone();
            let tx = self.tx.clone();
            let signal = self.repaint_signal.clone();
            let room_id = c.room_id.clone();
            let csrf = c.csrf.clone();

            self.rt.spawn(async move {
                let _ = tx
                    .send(AppMessage::Log("正在停止直播...".to_string()))
                    .await;
                signal.request_repaint();

                if let Err(e) = api.stop_live(&room_id, &csrf).await {
                    let _ = tx
                        .send(AppMessage::StopLiveResult(false, e.to_string()))
                        .await;
                } else {
                    let _ = tx
                        .send(AppMessage::StopLiveResult(true, "直播已停止".to_string()))
                        .await;
                }
                signal.request_repaint();
            });
        }
    }

    pub fn update_title(&self) {
        if let Some(c) = &self.cookies {
            let api = self.api.clone();
            let tx = self.tx.clone();
            let signal = self.repaint_signal.clone();
            let room_id = c.room_id.clone();
            let csrf = c.csrf.clone();
            let title = self.live_settings.title.clone();

            self.rt.spawn(async move {
                let _ = tx
                    .send(AppMessage::Log("正在更新房间标题...".to_string()))
                    .await;
                signal.request_repaint();

                if let Err(e) = api.update_room_title(&room_id, &csrf, &title).await {
                    let _ = tx
                        .send(AppMessage::RoomTitleUpdated(
                            false,
                            format!("更新标题失败: {}", e),
                        ))
                        .await;
                } else {
                    let _ = tx
                        .send(AppMessage::RoomTitleUpdated(
                            true,
                            "标题更新成功".to_string(),
                        ))
                        .await;
                }
                signal.request_repaint();
            });
        }
    }

    pub fn send_bullet(&self, msg: String) {
        if let Some(c) = &self.cookies {
            let api = self.api.clone();
            let tx = self.tx.clone();
            let signal = self.repaint_signal.clone();
            let room_id = c.room_id.clone();
            let csrf = c.csrf.clone();

            self.rt.spawn(async move {
                match api.send_bullet(&msg, &room_id, &csrf).await {
                    Ok((s, m)) => {
                        let _ = tx.send(AppMessage::BulletSent(s, m)).await;
                    }
                    Err(e) => {
                        let _ = tx.send(AppMessage::Log(format!("错误: {}", e))).await;
                    }
                }
                signal.request_repaint();
            });
        }
    }

    pub fn fetch_server_time(&self) {
        let tx = self.tx.clone();
        let api = self.api.clone();
        let signal = self.repaint_signal.clone();
        self.rt.spawn(async move {
            if let Ok(server_ts) = api.get_server_time().await {
                let local_ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64;
                let offset = server_ts - local_ts;
                let _ = tx.send(AppMessage::ServerTimeSynced(offset)).await;
                signal.request_repaint();
            }
        });
    }

    pub fn fetch_room_title(&self, room_id: String) {
        let tx = self.tx.clone();
        let api = self.api.clone();
        let signal = self.repaint_signal.clone();
        self.rt.spawn(async move {
            if let Ok(title) = api.get_room_title(&room_id).await {
                let _ = tx.send(AppMessage::RoomTitleFetched(title)).await;
                signal.request_repaint();
            }
        });
    }

    pub fn refresh_cookies(&mut self) {
        let (csrf, refresh_token, cookie_str) = if let Some(c) = &self.cookies {
            (
                c.csrf.clone(),
                c.refresh_token.clone(),
                c.cookie_str.clone(),
            )
        } else {
            self.log("未登录".to_string());
            return;
        };

        if refresh_token.is_empty() {
            self.log("缺少Refresh Token".to_string());
            return;
        }

        let api = self.api.clone();
        let tx = self.tx.clone();
        let signal = self.repaint_signal.clone();

        self.rt.spawn(async move {
            match api.refresh_cookie(&csrf, &refresh_token, &cookie_str).await {
                Ok((new_ref, new_cookie)) => {
                    let _ = tx
                        .send(AppMessage::CookieRefreshed(new_ref, new_cookie))
                        .await;
                }
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("does not need refresh") {
                        let _ = tx.send(AppMessage::Log("Cookie无需刷新".to_string())).await;
                    } else {
                        let _ = tx.send(AppMessage::Log(format!("刷新失败: {}", e))).await;
                    }
                }
            }
            signal.request_repaint();
        });
    }

    pub fn logout(&mut self) {
        config::remove_files();
        self.cookies = None;
        self.live_settings = config::LiveSettings::default();
        self.log("已退出登录".to_string());
    }
}
