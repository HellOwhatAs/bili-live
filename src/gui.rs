use crate::app_core::{BiliLiveContext, RepaintSignal, UiEffect};
use eframe::egui;
use qrcode::QrCode;
use std::sync::Arc;

#[derive(PartialEq)]
enum Tab {
    Setup,
    Live,
    Result,
}

struct EguiRepaintSignal(egui::Context);

impl RepaintSignal for EguiRepaintSignal {
    fn request_repaint(&self) {
        self.0.request_repaint();
    }
}

pub struct BiliLiveApp {
    core: BiliLiveContext,

    // UI Only State
    selected_tab: Tab,
    qr_image: Option<egui::TextureHandle>,
    show_qr_window: bool,
    qr_window_title: String,
    input_bullet: String,
    selected_parent_idx: Option<usize>,
}

impl BiliLiveApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let signal = Arc::new(EguiRepaintSignal(cc.egui_ctx.clone()));
        let core = BiliLiveContext::new(Some(signal));

        let app = Self {
            core,
            selected_tab: Tab::Setup,
            qr_image: None,
            show_qr_window: false,
            qr_window_title: "请扫码登录".to_string(),
            input_bullet: String::new(),
            selected_parent_idx: None,
        };

        Self::configure_fonts(&cc.egui_ctx);
        Self::configure_styles(&cc.egui_ctx);

        app
    }

    fn create_qr_image(content: &str) -> egui::ColorImage {
        let code = QrCode::new(content.as_bytes()).unwrap();
        let image = code.render::<image::Rgba<u8>>().build();
        let size = [image.width() as _, image.height() as _];
        egui::ColorImage::from_rgba_unmultiplied(size, image.into_flat_samples().as_slice())
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
        // Process UI effects from Core
        let effects = self.core.process_messages();
        for effect in effects {
            match effect {
                UiEffect::ShowQrReceived(url, _key) => {
                    let image = Self::create_qr_image(&url);
                    self.qr_image = Some(ctx.load_texture("qr_code", image, Default::default()));
                    self.show_qr_window = true;
                    self.qr_window_title = "请扫码登录".to_string();
                }
                UiEffect::FaceAuthQrReceived(content) => {
                    let image = Self::create_qr_image(&content);
                    self.qr_image =
                        Some(ctx.load_texture("face_auth_qr", image, Default::default()));
                    self.show_qr_window = true;
                    self.qr_window_title = "需要人脸认证".to_string();
                }
                UiEffect::HideQrWindow => {
                    self.show_qr_window = false;
                }
                UiEffect::SwitchToResultTab => {
                    self.selected_tab = Tab::Result;
                }
            }
        }

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

impl BiliLiveApp {
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
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.label(&self.core.log_messages);
                    });
            });
    }

    fn draw_setup_tab(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        ui.group(|ui| {
            ui.label("账号管理");
            if ui.button("刷新本地Cookies").clicked() {
                self.core.refresh_cookies();
            }
            if ui.button("扫码登录").clicked() {
                self.core.fetch_qrcode();
            }
        });

        if let Some(c) = &self.core.cookies {
            ui.label(format!("当前RoomID: {}", c.room_id));
        }
    }

    fn draw_live_tab(&mut self, ui: &mut egui::Ui) {
        ui.label("直播标题:");
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.core.live_settings.title);
            if ui.button("更新标题").clicked() {
                self.core.update_title();
            }
        });

        ui.label("分区选择:");
        // Sync parent selection from ID if needed
        if self.selected_parent_idx.is_none() && !self.core.live_settings.area_id.is_empty() {
            for (p_idx, p) in self.core.partitions.iter().enumerate() {
                if p.children
                    .iter()
                    .any(|c| c.id == self.core.live_settings.area_id)
                {
                    self.selected_parent_idx = Some(p_idx);
                    break;
                }
            }
        }

        let mut selected_parent_name = "选择父分区".to_string();
        if let Some(idx) = self.selected_parent_idx {
            if let Some(p) = self.core.partitions.get(idx) {
                selected_parent_name = p.name.clone();
            }
        }

        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("parent_combo")
                .selected_text(selected_parent_name)
                .show_ui(ui, |ui| {
                    for (i, p) in self.core.partitions.iter().enumerate() {
                        ui.selectable_value(&mut self.selected_parent_idx, Some(i), &p.name);
                    }
                });

            if let Some(idx) = self.selected_parent_idx {
                if let Some(parent) = self.core.partitions.get(idx) {
                    let mut selected_sub_name = "选择子分区".to_string();
                    if let Some(sub) = parent
                        .children
                        .iter()
                        .find(|s| s.id == self.core.live_settings.area_id)
                    {
                        selected_sub_name = sub.name.clone();
                    }

                    egui::ComboBox::from_id_salt("sub_combo")
                        .selected_text(selected_sub_name)
                        .show_ui(ui, |ui| {
                            for sub in &parent.children {
                                ui.selectable_value(
                                    &mut self.core.live_settings.area_id,
                                    sub.id.clone(),
                                    &sub.name,
                                );
                            }
                        });
                }
            }
        });

        ui.label(format!("当前分区ID: {}", self.core.live_settings.area_id));
        if self.core.live_settings.area_id.is_empty() {
            self.core.live_settings.area_id = "374".to_string();
        }

        ui.add_space(10.0);

        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.core.live_state.is_live, egui::Button::new("开始直播"))
                .clicked()
            {
                self.core.start_live();
            }

            if ui
                .add_enabled(self.core.live_state.is_live, egui::Button::new("停止直播"))
                .clicked()
            {
                self.core.stop_live();
            }
        });

        ui.separator();
        ui.label("发送弹幕:");
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.input_bullet);
            if ui.button("发送").clicked() {
                self.core.send_bullet(self.input_bullet.clone());
                self.input_bullet.clear();
            }
        });
    }

    fn draw_result_tab(&mut self, ui: &mut egui::Ui) {
        ui.label("推流地址:");
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.core.live_state.live_url);
            if ui.button("复制").clicked() {
                ui.output_mut(|o| o.copied_text = self.core.live_state.live_url.clone());
            }
        });

        ui.label("推流码:");
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.core.live_state.live_code);
            if ui.button("复制").clicked() {
                ui.output_mut(|o| o.copied_text = self.core.live_state.live_code.clone());
            }
        });

        if !self.core.live_state.live_url.is_empty() {
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
}
