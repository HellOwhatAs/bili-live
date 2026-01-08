#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use crate::api::BiliApi;
use crate::config::{LiveSettings, UserCookies};
use eframe::egui;
use qrcode::QrCode;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

mod api;
mod config;

#[derive(PartialEq)]
enum Tab {
    Setup,
    Live,
    Result,
}

enum AppMessage {
    Log(String),
    QrCodeGenerated(String, String, egui::ColorImage),
    QrCodePollResult(bool, String, Option<UserCookies>),
    PartitionLoaded(serde_json::Value),
    StreamInfo(String, String),
    BulletSent(bool, String),
    RoomTitleUpdated(bool, String),
    RoomTitleFetched(String),
    LiveVersionFetched(String, String),
    ServerTimeSynced(i64),
    StopLiveResult(bool, String),
    FaceAuthQrGenerated(egui::ColorImage),
}

use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
struct SubPartition {
    id: String,
    name: String,
    #[serde(rename = "parent_name")]
    _parent_name: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ParentPartition {
    #[serde(rename = "id")]
    _id: i64,
    name: String,
    #[serde(rename = "list")]
    children: Vec<SubPartition>,
}

struct BiliLiveApp {
    rt: Runtime,
    tx: mpsc::Sender<AppMessage>,
    rx: mpsc::Receiver<AppMessage>,

    // UI State
    selected_tab: Tab,
    log_messages: String,
    status_msg: String,

    // Data State
    api: BiliApi,
    cookies: Option<UserCookies>,
    live_settings: LiveSettings,
    partitions: Vec<ParentPartition>,
    selected_parent_idx: Option<usize>,

    // QR Code State
    qr_image: Option<egui::TextureHandle>,
    show_qr_window: bool,
    qr_window_title: String,

    // Live State
    live_url: String,
    live_code: String,
    is_live: bool,
    live_build: String,
    live_version: String,
    time_offset: i64,

    // Inputs
    input_bullet: String,
}

impl BiliLiveApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (tx, rx) = mpsc::channel(100);
        let rt = Runtime::new().expect("Failed to create Tokio runtime");

        // Initialize State
        let mut app = Self {
            rt,
            tx: tx.clone(),
            rx,
            selected_tab: Tab::Setup,
            log_messages: String::new(),
            status_msg: "就绪".to_string(),
            api: BiliApi::new(),
            cookies: None,
            live_settings: config::load_last_settings(),
            partitions: Vec::new(),
            selected_parent_idx: None,
            qr_image: None,
            show_qr_window: false,
            qr_window_title: "请扫码登录".to_string(),
            live_url: String::new(),
            live_code: String::new(),
            is_live: false,
            live_build: "0".to_string(),
            live_version: "0".to_string(),
            time_offset: 0,
            input_bullet: String::new(),
        };

        // Initialize UI Logic
        Self::configure_fonts(&cc.egui_ctx);
        Self::configure_styles(&cc.egui_ctx);

        // Load initial data
        app.fetch_live_version();
        app.fetch_server_time();
        if let Some(cookies) = config::load_cookies() {
            app.set_cookies(cookies);
            app.fetch_partition();
        }

        app
    }

    fn set_cookies(&mut self, cookies: UserCookies) {
        self.api = BiliApi::new_with_cookies(&cookies.cookie_str);
        self.fetch_room_title(cookies.room_id.clone());
        self.cookies = Some(cookies);
        self.log("已加载 Cookies".to_string());
    }

    fn log(&mut self, msg: String) {
        self.log_messages.push_str(&format!("{}\n", msg));
        self.status_msg = msg;
    }

    // --- Async Tasks Wrappers ---

    fn fetch_live_version(&self) {
        let tx = self.tx.clone();
        let api = self.api.clone();
        self.rt.spawn(async move {
            if let Ok((b, v)) = api.get_live_version().await {
                let _ = tx.send(AppMessage::LiveVersionFetched(b, v)).await;
            }
        });
    }

    fn fetch_partition(&self) {
        let tx = self.tx.clone();
        let api = self.api.clone();
        self.rt.spawn(async move {
            match api.get_partition().await {
                Ok(data) => {
                    let _ = tx.send(AppMessage::PartitionLoaded(data)).await;
                }
                Err(_) => {
                    let _ = tx.send(AppMessage::Log("获取分区失败".to_string())).await;
                }
            }
        });
    }

    fn generate_qr(&self, ctx: &egui::Context) {
        let tx = self.tx.clone();
        let api = self.api.clone();
        let ctx = ctx.clone();

        self.rt.spawn(async move {
            match api.get_qrcode().await {
                Ok((url, key)) => {
                    let image = Self::create_qr_image(&url);
                    let _ = tx.send(AppMessage::QrCodeGenerated(url, key, image)).await;
                    ctx.request_repaint();
                }
                Err(e) => {
                    let _ = tx
                        .send(AppMessage::Log(format!("获取二维码失败: {}", e)))
                        .await;
                }
            }
        });
    }

    fn poll_qr_login(&self, key: String, ctx: &egui::Context) {
        let tx = self.tx.clone();
        let api = self.api.clone();
        let ctx = ctx.clone();

        self.rt.spawn(async move {
            // Poll for 3 minutes
            for _ in 0..180 {
                if let Ok(resp) = api.poll_qrcode(&key).await {
                    if let Some(code) = resp["data"]["code"].as_i64() {
                        match code {
                            0 => {
                                let url = resp["data"]["url"].as_str().unwrap_or("");
                                let _ = tx.send(Self::parse_login_url(url).await).await;
                                break;
                            }
                            86038 => {
                                // Expired
                                let _ = tx
                                    .send(AppMessage::QrCodePollResult(
                                        false,
                                        "二维码已过期".to_string(),
                                        None,
                                    ))
                                    .await;
                                break;
                            }
                            _ => {} // Pending
                        }
                    }
                }
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            }
            ctx.request_repaint();
        });
    }

    // --- Helpers ---

    fn create_qr_image(content: &str) -> egui::ColorImage {
        let code = QrCode::new(content.as_bytes()).unwrap();
        let image = code.render::<image::Rgba<u8>>().build();
        let size = [image.width() as _, image.height() as _];
        egui::ColorImage::from_rgba_unmultiplied(size, image.into_flat_samples().as_slice())
    }

    // Logic separated from async block for clarity
    async fn parse_login_url(url: &str) -> AppMessage {
        if let Ok(parsed) = url::Url::parse(url) {
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
            };
            return AppMessage::QrCodePollResult(true, "登录成功".to_string(), Some(cookies));
        }
        AppMessage::QrCodePollResult(false, "解析登录URL失败".to_string(), None)
    }

    fn configure_fonts(ctx: &egui::Context) {
        let mut fonts = egui::FontDefinitions::default();
        let font_data = [
            "C:\\Windows\\Fonts\\msyh.ttc",
            "C:\\Windows\\Fonts\\simhei.ttf",
        ]
        .iter()
        .find_map(|p| std::fs::read(p).ok());

        if let Some(data) = font_data {
            fonts
                .font_data
                .insert("system_font".to_owned(), egui::FontData::from_owned(data));
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                family.insert(0, "system_font".to_owned());
            }
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                family.push("system_font".to_owned());
            }
            ctx.set_fonts(fonts);
        }
    }

    fn configure_styles(ctx: &egui::Context) {
        let mut style = (*ctx.style()).clone();
        for (text_style, size) in [
            (egui::TextStyle::Body, 16.0),
            (egui::TextStyle::Button, 16.0),
            (egui::TextStyle::Heading, 20.0),
        ] {
            style.text_styles.insert(
                text_style,
                egui::FontId::new(size, egui::FontFamily::Proportional),
            );
        }
        ctx.set_style(style);
    }
}

impl eframe::App for BiliLiveApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_messages(ctx);

        self.draw_top_panel(ctx);
        self.draw_status_panel(ctx);

        egui::CentralPanel::default().show(ctx, |ui| match self.selected_tab {
            Tab::Setup => self.draw_setup_tab(ui, ctx),
            Tab::Live => self.draw_live_tab(ui),
            Tab::Result => self.draw_result_tab(ui),
        });

        self.draw_qr_window(ctx);
    }
}

// UI Drawing Methods
impl BiliLiveApp {
    fn handle_messages(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                AppMessage::Log(s) => self.log(s),
                AppMessage::QrCodeGenerated(_url, key, image) => {
                    self.qr_image = Some(ctx.load_texture("qr_code", image, Default::default()));
                    self.show_qr_window = true;
                    self.qr_window_title = "请扫码登录".to_string();
                    self.poll_qr_login(key, ctx);
                }
                AppMessage::QrCodePollResult(success, msg, cookies) => {
                    self.log(msg);
                    if success {
                        self.show_qr_window = false;
                        if let Some(c) = cookies {
                            config::save_cookies(&c);
                            self.set_cookies(c);
                            self.fetch_partition();
                        }
                    }
                }
                AppMessage::PartitionLoaded(data) => {
                    // Parse and update partitions
                    if let Some(d) = data.get("data") {
                        if let Ok(parsed) =
                            serde_json::from_value::<Vec<ParentPartition>>(d.clone())
                        {
                            self.partitions = parsed;
                            self.log("分区数据已更新".to_string());
                        } else {
                            self.log("分区数据解析失败".to_string());
                        }
                    } else {
                        self.log("分区响应格式错误".to_string());
                    }
                }
                AppMessage::StreamInfo(url, code) => {
                    self.live_url = url;
                    self.live_code = code;
                    self.is_live = true;
                    self.log("直播开启成功".to_string());
                    self.selected_tab = Tab::Result;
                }
                AppMessage::StopLiveResult(success, msg) => {
                    self.log(msg);
                    if success {
                        self.is_live = false;
                    }
                }
                AppMessage::RoomTitleUpdated(_success, msg) => {
                    self.log(msg);
                    if _success {
                        // Optionally refresh title if update succeeded, but msg might be enough
                    }
                }
                AppMessage::RoomTitleFetched(title) => {
                    self.live_settings.title = title;
                    self.log("房间标题已同步".to_string());
                }
                AppMessage::LiveVersionFetched(b, v) => {
                    self.live_build = b;
                    self.live_version = v;
                }
                AppMessage::ServerTimeSynced(offset) => {
                    self.time_offset = offset;
                    self.log(format!("服务器时间已同步 (Offset: {}s)", offset));
                }
                AppMessage::FaceAuthQrGenerated(image) => {
                    self.qr_image =
                        Some(ctx.load_texture("face_auth_qr", image, Default::default()));
                    self.show_qr_window = true;
                    self.qr_window_title = "需要人脸认证".to_string();
                    self.log("检测到需要人脸认证".to_string());
                }
                AppMessage::BulletSent(success, msg) => {
                    if success {
                        self.log(format!("弹幕发送成功: {}", msg));
                    } else {
                        self.log(format!("弹幕发送失败: {}", msg));
                    }
                }
            }
        }
    }

    fn draw_top_panel(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("B站推流码获取工具");
                ui.separator();
                if ui
                    .selectable_label(self.selected_tab == Tab::Setup, "账号设置")
                    .clicked()
                {
                    self.selected_tab = Tab::Setup;
                }
                if ui
                    .selectable_label(self.selected_tab == Tab::Live, "直播设置")
                    .clicked()
                {
                    self.selected_tab = Tab::Live;
                }
                if ui
                    .selectable_label(self.selected_tab == Tab::Result, "推流信息")
                    .clicked()
                {
                    self.selected_tab = Tab::Result;
                }
            });
        });
    }

    fn draw_status_panel(&self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status_panel")
            .resizable(true)
            .default_height(100.0)
            .min_height(30.0)
            .show(ctx, |ui| {
                // ui.label("系统日志:");
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.label(&self.log_messages);
                    });
            });
    }

    fn draw_setup_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.group(|ui| {
            ui.label("账号管理");
            if ui.button("刷新本地Cookies").clicked() {
                if let Some(c) = config::load_cookies() {
                    self.set_cookies(c);
                } else {
                    self.log("未找到cookies.txt".to_string());
                }
            }
            if ui.button("扫码登录").clicked() {
                self.generate_qr(ctx);
            }
        });

        if let Some(c) = &self.cookies {
            ui.label(format!("当前RoomID: {}", c.room_id));
        }
    }

    fn draw_live_tab(&mut self, ui: &mut egui::Ui) {
        ui.label("直播标题:");
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.live_settings.title);
            if ui.button("更新标题").clicked() {
                self.action_update_title();
            }
        });

        ui.label("分区选择:");
        // Sync parent selection from ID if needed
        if self.selected_parent_idx.is_none() && !self.live_settings.area_id.is_empty() {
            for (p_idx, p) in self.partitions.iter().enumerate() {
                if p.children
                    .iter()
                    .any(|c| c.id == self.live_settings.area_id)
                {
                    self.selected_parent_idx = Some(p_idx);
                    break;
                }
            }
        }

        let mut selected_parent_name = "选择父分区".to_string();
        if let Some(idx) = self.selected_parent_idx {
            if let Some(p) = self.partitions.get(idx) {
                selected_parent_name = p.name.clone();
            }
        }

        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("parent_combo")
                .selected_text(selected_parent_name)
                .show_ui(ui, |ui| {
                    for (i, p) in self.partitions.iter().enumerate() {
                        ui.selectable_value(&mut self.selected_parent_idx, Some(i), &p.name);
                    }
                });

            if let Some(idx) = self.selected_parent_idx {
                if let Some(parent) = self.partitions.get(idx) {
                    let mut selected_sub_name = "选择子分区".to_string();
                    if let Some(sub) = parent
                        .children
                        .iter()
                        .find(|s| s.id == self.live_settings.area_id)
                    {
                        selected_sub_name = sub.name.clone();
                    }

                    egui::ComboBox::from_id_salt("sub_combo")
                        .selected_text(selected_sub_name)
                        .show_ui(ui, |ui| {
                            for sub in &parent.children {
                                ui.selectable_value(
                                    &mut self.live_settings.area_id,
                                    sub.id.clone(),
                                    &sub.name,
                                );
                            }
                        });
                }
            }
        });

        ui.label(format!("当前分区ID: {}", self.live_settings.area_id));
        if self.live_settings.area_id.is_empty() {
            self.live_settings.area_id = "374".to_string(); // Default to Chat if empty? Or just warn.
        }

        ui.add_space(10.0);

        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.is_live, egui::Button::new("开始直播"))
                .clicked()
            {
                self.action_start_live();
            }

            if ui
                .add_enabled(self.is_live, egui::Button::new("停止直播"))
                .clicked()
            {
                self.action_stop_live();
            }
        });

        ui.separator();
        ui.label("发送弹幕:");
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.input_bullet);
            if ui.button("发送").clicked() {
                self.action_send_bullet();
            }
        });
    }

    fn draw_result_tab(&mut self, ui: &mut egui::Ui) {
        ui.label("推流地址:");
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.live_url);
            if ui.button("复制").clicked() {
                ui.output_mut(|o| o.copied_text = self.live_url.clone());
            }
        });

        ui.label("推流码:");
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.live_code);
            if ui.button("复制").clicked() {
                ui.output_mut(|o| o.copied_text = self.live_code.clone());
            }
        });

        if !self.live_url.is_empty() {
            ui.label("请将以上信息填入OBS等推流软件中。");
        }
    }

    fn draw_qr_window(&mut self, ctx: &egui::Context) {
        if !self.show_qr_window {
            return;
        }

        let mut show = true;
        egui::Window::new(&self.qr_window_title)
            .open(&mut show)
            .show(ctx, |ui| {
                if let Some(texture) = &self.qr_image {
                    ui.image(texture);
                } else {
                    ui.spinner();
                }

                if self.qr_window_title.contains("人脸") {
                    ui.label("请使用B站APP进行人脸认证。");
                    if ui.button("已完成/关闭").clicked() {
                        self.show_qr_window = false;
                    }
                }
            });
        if !show {
            self.show_qr_window = false;
        }
    }

    // Actions
    fn action_start_live(&self) {
        if let Some(c) = &self.cookies {
            let api = self.api.clone();
            let tx = self.tx.clone();
            let room_id = c.room_id.clone();
            let csrf = c.csrf.clone();
            let area_id = self.live_settings.area_id.clone();
            let build = self.live_build.clone();
            let version = self.live_version.clone();

            // Calculate synchronized timestamp
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;
            let ts = now + self.time_offset;

            // Note: Title is no longer updated here
            config::save_last_settings(&self.live_settings);

            self.rt.spawn(async move {
                let _ = tx
                    .send(AppMessage::Log("正在启动直播...".to_string()))
                    .await;
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
                                let image = Self::create_qr_image(&qr_content);
                                AppMessage::FaceAuthQrGenerated(image)
                            } else {
                                AppMessage::Log("需要人脸认证但无法获取二维码".to_string())
                            };
                            let _ = tx.send(msg).await;
                        } else {
                            let _ = tx.send(AppMessage::Log(format!("错误: {}", e))).await;
                        }
                    }
                }
            });
        }
    }

    fn action_update_title(&self) {
        if let Some(c) = &self.cookies {
            let api = self.api.clone();
            let tx = self.tx.clone();
            let room_id = c.room_id.clone();
            let csrf = c.csrf.clone();
            let title = self.live_settings.title.clone();

            self.rt.spawn(async move {
                let _ = tx
                    .send(AppMessage::Log("正在更新房间标题...".to_string()))
                    .await;
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
            });
        }
    }

    fn action_stop_live(&self) {
        if let Some(c) = &self.cookies {
            let api = self.api.clone();
            let tx = self.tx.clone();
            let room_id = c.room_id.clone();
            let csrf = c.csrf.clone();

            self.rt.spawn(async move {
                let _ = tx
                    .send(AppMessage::Log("正在停止直播...".to_string()))
                    .await;
                if let Err(e) = api.stop_live(&room_id, &csrf).await {
                    let _ = tx
                        .send(AppMessage::StopLiveResult(false, e.to_string()))
                        .await;
                } else {
                    let _ = tx
                        .send(AppMessage::StopLiveResult(true, "直播已停止".to_string()))
                        .await;
                }
            });
        }
    }

    fn action_send_bullet(&mut self) {
        if let Some(c) = &self.cookies {
            let api = self.api.clone();
            let tx = self.tx.clone();
            let msg = self.input_bullet.clone();
            let room_id = c.room_id.clone();
            let csrf = c.csrf.clone();
            self.input_bullet.clear();

            self.rt.spawn(async move {
                match api.send_bullet(&msg, &room_id, &csrf).await {
                    Ok((s, m)) => {
                        let _ = tx.send(AppMessage::BulletSent(s, m)).await;
                    }
                    Err(e) => {
                        let _ = tx.send(AppMessage::Log(format!("错误: {}", e))).await;
                    }
                }
            });
        }
    }

    fn fetch_server_time(&self) {
        let tx = self.tx.clone();
        let api = self.api.clone();
        self.rt.spawn(async move {
            if let Ok(server_ts) = api.get_server_time().await {
                let local_ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64;
                let offset = server_ts - local_ts;
                let _ = tx.send(AppMessage::ServerTimeSynced(offset)).await;
            }
        });
    }

    fn fetch_room_title(&self, room_id: String) {
        let tx = self.tx.clone();
        let api = self.api.clone();
        self.rt.spawn(async move {
            if let Ok(title) = api.get_room_title(&room_id).await {
                let _ = tx.send(AppMessage::RoomTitleFetched(title)).await;
            }
        });
    }
}

fn main() -> eframe::Result<()> {
    env_logger::init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([500.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native(
        "bili-live",
        options,
        Box::new(|cc| Ok(Box::new(BiliLiveApp::new(cc)))),
    )
}
