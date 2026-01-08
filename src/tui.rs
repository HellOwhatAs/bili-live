use crate::app_core::{BiliLiveContext, RepaintSignal, UiEffect};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{prelude::*, widgets::*};
use std::{
    error::Error,
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

struct TuiRepaintSignal(Arc<AtomicBool>);

impl RepaintSignal for TuiRepaintSignal {
    fn request_repaint(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

pub fn run_tui() -> Result<(), Box<dyn Error>> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create App
    let repaint_flag = Arc::new(AtomicBool::new(true));
    let signal = Arc::new(TuiRepaintSignal(repaint_flag.clone()));
    let core = BiliLiveContext::new(Some(signal));
    let mut app = BiliLiveTui::new(core, repaint_flag);

    let res = run_app(&mut terminal, &mut app);

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{:?}", err);
    }

    Ok(())
}

fn run_app<B: Backend<Error = io::Error>>(
    terminal: &mut Terminal<B>,
    app: &mut BiliLiveTui,
) -> io::Result<()> {
    loop {
        if app.repaint_flag.load(Ordering::Relaxed) {
            terminal.draw(|f| app.ui(f))?;
            app.repaint_flag.store(false, Ordering::Relaxed);
        }

        // Poll with timeout for updates
        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind == KeyEventKind::Press {
                        app.handle_input(key.code);
                    }
                }
                Event::Resize(_, _) => {
                    app.request_repaint();
                }
                _ => {}
            }
        }

        app.update();

        if app.should_quit {
            return Ok(());
        }
    }
}

enum Tab {
    Setup,
    Live,
    Result,
}

enum Focus {
    None,
    TitleInput,
    BulletInput,
    ParentPartition,
    SubPartition,
}

struct BiliLiveTui {
    core: BiliLiveContext,
    repaint_flag: Arc<AtomicBool>,
    should_quit: bool,

    selected_tab: Tab,

    // Popups
    show_qr_popup: bool,
    qr_content: Option<String>,
    qr_title: String,

    // Inputs
    focus: Focus,
    input_buffer: String, // Shared buffer for currently focused input

    // List States for scrolling
    parent_list_state: ListState,
    sub_list_state: ListState,

    // Selection state
    selected_parent_idx: usize,
    selected_sub_idx: usize,

    // Scroll state for logs
    log_scroll: u16,

    // State sync
    last_partition_count: usize,
}

impl BiliLiveTui {
    fn new(core: BiliLiveContext, repaint_flag: Arc<AtomicBool>) -> Self {
        let mut parent_list_state = ListState::default();
        parent_list_state.select(Some(0));
        let mut sub_list_state = ListState::default();
        sub_list_state.select(Some(0));

        Self {
            core,
            repaint_flag,
            should_quit: false,
            selected_tab: Tab::Setup,
            show_qr_popup: false,
            qr_content: None,
            qr_title: String::new(),
            focus: Focus::None,
            input_buffer: String::new(),
            selected_parent_idx: 0,
            selected_sub_idx: 0,
            parent_list_state,
            sub_list_state,
            log_scroll: 0,
            last_partition_count: 0,
        }
    }

    fn update(&mut self) {
        let effects = self.core.process_messages();
        for effect in effects {
            match effect {
                UiEffect::ShowQrReceived(url, _key) => {
                    self.show_qr_popup = true;
                    self.qr_title = "请扫码登录 (Esc关闭)".to_string();
                    self.qr_content = Some(url);
                    self.request_repaint();
                }
                UiEffect::FaceAuthQrReceived(content) => {
                    self.show_qr_popup = true;
                    self.qr_title = "需要人脸认证 (Esc关闭)".to_string();
                    self.qr_content = Some(content);
                    self.request_repaint();
                }
                UiEffect::HideQrWindow => {
                    self.show_qr_popup = false;
                    self.request_repaint();
                }
                UiEffect::SwitchToResultTab => {
                    self.selected_tab = Tab::Result;
                    self.request_repaint();
                }
            }
        }

        // Sync partition selection if data loaded
        if self.core.partitions.len() != self.last_partition_count {
            self.last_partition_count = self.core.partitions.len();
            if self.last_partition_count > 0 {
                let target_id = &self.core.live_settings.area_id;
                let mut found = false;

                for (p_idx, parent) in self.core.partitions.iter().enumerate() {
                    for (s_idx, sub) in parent.children.iter().enumerate() {
                        if &sub.id == target_id {
                            self.selected_parent_idx = p_idx;
                            self.selected_sub_idx = s_idx;

                            self.parent_list_state.select(Some(p_idx));
                            self.sub_list_state.select(Some(s_idx));

                            found = true;
                            break;
                        }
                    }
                    if found {
                        break;
                    }
                }
                self.request_repaint();
            }
        }

        // Auto-scroll logs
        // Simple logic: user can scroll up, but if at bottom, stick to bottom?
        // For now just keep it simple.
    }

    fn request_repaint(&self) {
        self.repaint_flag.store(true, Ordering::Relaxed);
    }

    fn handle_input(&mut self, key: KeyCode) {
        if self.show_qr_popup {
            match key {
                KeyCode::Esc => {
                    self.show_qr_popup = false;
                    self.request_repaint();
                }
                _ => {}
            }
            return;
        }

        match self.focus {
            Focus::TitleInput => {
                match key {
                    KeyCode::Enter => {
                        self.core.live_settings.title = self.input_buffer.clone();
                        self.core.update_title();
                        self.focus = Focus::None;
                        self.input_buffer.clear();
                    }
                    KeyCode::Esc => {
                        self.focus = Focus::None; // Cancel
                        self.input_buffer.clear();
                    }
                    KeyCode::Char(c) => self.input_buffer.push(c),
                    KeyCode::Backspace => {
                        self.input_buffer.pop();
                    }
                    _ => {}
                }
            }
            Focus::BulletInput => match key {
                KeyCode::Enter => {
                    if !self.input_buffer.is_empty() {
                        self.core.send_bullet(self.input_buffer.clone());
                        self.input_buffer.clear();
                    }
                }
                KeyCode::Esc => {
                    self.focus = Focus::None;
                    self.input_buffer.clear();
                }
                KeyCode::Char(c) => self.input_buffer.push(c),
                KeyCode::Backspace => {
                    self.input_buffer.pop();
                }
                _ => {}
            },
            Focus::None | Focus::ParentPartition | Focus::SubPartition => {
                self.handle_navigation_input(key);
            }
        }
        self.request_repaint();
    }

    fn handle_navigation_input(&mut self, key: KeyCode) {
        match key {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('1') => self.selected_tab = Tab::Setup,
            KeyCode::Char('2') => self.selected_tab = Tab::Live,
            KeyCode::Char('3') => self.selected_tab = Tab::Result,
            KeyCode::Tab => {
                self.selected_tab = match self.selected_tab {
                    Tab::Setup => Tab::Live,
                    Tab::Live => Tab::Result,
                    Tab::Result => Tab::Setup,
                };
            }
            _ => match self.selected_tab {
                Tab::Setup => self.handle_setup_input(key),
                Tab::Live => self.handle_live_input(key),
                Tab::Result => self.handle_result_input(key),
            },
        }
    }

    fn handle_setup_input(&mut self, key: KeyCode) {
        match key {
            KeyCode::Char('r') => self.core.refresh_cookies(),
            KeyCode::Char('l') => self.core.fetch_qrcode(),
            _ => {}
        }
    }

    fn handle_live_input(&mut self, key: KeyCode) {
        match key {
            KeyCode::Char('t') => {
                self.focus = Focus::TitleInput;
                self.input_buffer = self.core.live_settings.title.clone();
            }
            KeyCode::Char('b') => {
                self.focus = Focus::BulletInput;
                self.input_buffer.clear();
            }
            KeyCode::Char('s') => {
                if !self.core.live_state.is_live {
                    self.core.start_live();
                }
            }
            KeyCode::Char('x') => {
                if self.core.live_state.is_live {
                    self.core.stop_live();
                }
            }
            KeyCode::Char('p') => {
                // Focus Partition Selection
                if matches!(self.focus, Focus::None) {
                    self.focus = Focus::ParentPartition;
                } else if matches!(self.focus, Focus::ParentPartition) {
                    self.focus = Focus::SubPartition;
                } else {
                    self.focus = Focus::None;
                }
            }
            KeyCode::Up | KeyCode::Down => {
                if matches!(self.focus, Focus::ParentPartition) {
                    if !self.core.partitions.is_empty() {
                        if key == KeyCode::Up {
                            if self.selected_parent_idx > 0 {
                                self.selected_parent_idx -= 1;
                            }
                        } else {
                            if self.selected_parent_idx < self.core.partitions.len() - 1 {
                                self.selected_parent_idx += 1;
                            }
                        }
                        self.parent_list_state
                            .select(Some(self.selected_parent_idx));

                        self.selected_sub_idx = 0; // Reset sub
                        self.sub_list_state.select(Some(0));
                    }
                } else if matches!(self.focus, Focus::SubPartition) {
                    if let Some(parent) = self.core.partitions.get(self.selected_parent_idx) {
                        if !parent.children.is_empty() {
                            if key == KeyCode::Up {
                                if self.selected_sub_idx > 0 {
                                    self.selected_sub_idx -= 1;
                                }
                            } else {
                                if self.selected_sub_idx < parent.children.len() - 1 {
                                    self.selected_sub_idx += 1;
                                }
                            }
                            self.sub_list_state.select(Some(self.selected_sub_idx));

                            // Update actual settings
                            self.core.live_settings.area_id =
                                parent.children[self.selected_sub_idx].id.clone();
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_result_input(&mut self, _key: KeyCode) {
        // Nothing to do
    }

    fn ui(&mut self, f: &mut Frame) {
        let size = f.area();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Tabs
                Constraint::Min(0),    // Content
                Constraint::Length(8), // Logs
            ])
            .split(size);

        // Draw Tabs
        let titles = vec!["Setup (1)", "Live (2)", "Result (3)"];
        let tabs = Tabs::new(titles)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Bili-Live TUI"),
            )
            .select(match self.selected_tab {
                Tab::Setup => 0,
                Tab::Live => 1,
                Tab::Result => 2,
            })
            .style(Style::default().fg(Color::Cyan))
            .highlight_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            );
        f.render_widget(tabs, chunks[0]);

        // Draw Content
        // We need interior mutability for list states if we can't change signature to &mut self
        // But render_stateful_widget requires &mut State.
        // ui(&mut self) is already taking &mut self.
        // But help functions draw_xxx take &self. We should change them to &mut self.
        match self.selected_tab {
            Tab::Setup => self.draw_setup_tab(f, chunks[1]),
            Tab::Live => self.draw_live_tab(f, chunks[1]),
            Tab::Result => self.draw_result_tab(f, chunks[1]),
        }

        // Draw Logs
        let log_text = self.core.log_messages.clone();
        let logs: Vec<Line> = log_text.lines().map(Line::from).collect();
        let log_lines = logs.len() as u16;
        let log_area_height = chunks[2].height.saturating_sub(2);
        self.log_scroll = if log_lines > log_area_height {
            log_lines - log_area_height
        } else {
            0
        };

        let log_widget = Paragraph::new(logs)
            .block(Block::default().borders(Borders::ALL).title("Logs"))
            .scroll((self.log_scroll, 0))
            .wrap(Wrap { trim: true });
        f.render_widget(log_widget, chunks[2]);

        // Draw Popups
        if self.show_qr_popup {
            self.draw_qr_popup(f);
        }
    }

    fn draw_setup_tab(&mut self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(0),
            ])
            .split(area);

        f.render_widget(
            Paragraph::new("按 'r' 刷新Cookies").block(Block::default().borders(Borders::ALL)),
            chunks[0],
        );
        f.render_widget(
            Paragraph::new("按 'l' 扫码登录").block(Block::default().borders(Borders::ALL)),
            chunks[1],
        );

        if let Some(c) = &self.core.cookies {
            f.render_widget(
                Paragraph::new(format!("已登录 RoomID: {}", c.room_id)),
                chunks[2],
            );
        } else {
            f.render_widget(Paragraph::new("未登录"), chunks[2]);
        }
    }

    fn draw_live_tab(&mut self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Title
                Constraint::Min(5),    // Partition
                Constraint::Length(3), // Controls
                Constraint::Length(3), // Bullet
            ])
            .split(area);

        // Title
        let title_style = if matches!(self.focus, Focus::TitleInput) {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        };
        let display_title = if matches!(self.focus, Focus::TitleInput) {
            self.input_buffer.clone()
        } else {
            self.core.live_settings.title.clone()
        };

        f.render_widget(
            Paragraph::new(format!("直播标题 (按 't' 编辑): {}", display_title))
                .block(Block::default().borders(Borders::ALL))
                .style(title_style),
            chunks[0],
        );

        // Partition
        let partition_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(chunks[1]);

        // Parent List
        let parents: Vec<ListItem> = self
            .core
            .partitions
            .iter()
            .map(|p| ListItem::new(p.name.clone()))
            .collect();
        let parent_state_style = if matches!(self.focus, Focus::ParentPartition) {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        };
        let parent_list = List::new(parents)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("父分区 (按 'p' 切换/上下选择)"),
            )
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol(">> ")
            .style(parent_state_style);

        // Use stateful render
        // We need to cast self as mutable to get mutable reference to state? No, ui takes &mut self
        f.render_stateful_widget(
            parent_list,
            partition_chunks[0],
            &mut self.parent_list_state,
        );

        // Sub List
        let mut sub_items = Vec::new();
        if let Some(parent) = self.core.partitions.get(self.selected_parent_idx) {
            for sub in parent.children.iter() {
                let is_selected_partition = sub.id == self.core.live_settings.area_id;
                let marker = if is_selected_partition {
                    " [当前]"
                } else {
                    ""
                };
                sub_items.push(ListItem::new(format!("{}{}", sub.name, marker)));
            }
        }
        let sub_state_style = if matches!(self.focus, Focus::SubPartition) {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        };
        let sub_list = List::new(sub_items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("子分区 (按 'p' 切换到此)"),
            )
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol(">> ")
            .style(sub_state_style);

        f.render_stateful_widget(sub_list, partition_chunks[1], &mut self.sub_list_state);

        // Controls
        let status = if self.core.live_state.is_live {
            "正在直播"
        } else {
            "未直播"
        };
        let controls = format!("当前状态: {} | 按 's' 开始 | 按 'x' 停止", status);
        f.render_widget(
            Paragraph::new(controls).block(Block::default().borders(Borders::ALL)),
            chunks[2],
        );

        // Bullet
        let bullet_style = if matches!(self.focus, Focus::BulletInput) {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        };
        let bullet_txt = if matches!(self.focus, Focus::BulletInput) {
            self.input_buffer.clone()
        } else {
            "按 'b' 发送弹幕...".to_string()
        };
        f.render_widget(
            Paragraph::new(bullet_txt)
                .block(Block::default().borders(Borders::ALL).title("弹幕"))
                .style(bullet_style),
            chunks[3],
        );
    }

    fn draw_result_tab(&mut self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(0),
            ])
            .split(area);

        f.render_widget(
            Paragraph::new(format!("推流地址: {}", self.core.live_state.live_url))
                .block(Block::default().borders(Borders::ALL)),
            chunks[0],
        );
        f.render_widget(
            Paragraph::new(format!("推流码: {}", self.core.live_state.live_code))
                .block(Block::default().borders(Borders::ALL)),
            chunks[1],
        );
        f.render_widget(
            Paragraph::new("请填入OBS中").block(Block::default().borders(Borders::ALL)),
            chunks[2],
        );
    }

    fn draw_qr_popup(&self, f: &mut Frame) {
        if let Some(content) = &self.qr_content {
            // Generate QR string
            let code = qrcode::QrCode::new(content.as_bytes()).unwrap();
            let string = code.render::<qrcode::render::unicode::Dense1x2>().build();

            // Calculate required size + padding/borders
            let lines: Vec<&str> = string.lines().collect();
            let height = lines.len() as u16 + 2; // +2 for border
            let width = lines.first().map(|l| l.chars().count()).unwrap_or(0) as u16 + 4; // +4 for padding

            let area = centered_rect_fixed(width, height, f.area());

            f.render_widget(Clear, area); // Clear background
            let block = Block::default()
                .title(self.qr_title.as_str())
                .borders(Borders::ALL)
                .style(Style::default().bg(Color::Black).fg(Color::White));
            f.render_widget(
                Paragraph::new(string)
                    .block(block)
                    .alignment(Alignment::Center),
                area,
            );
        } else {
            let area = centered_rect(60, 60, f.area());
            f.render_widget(Clear, area);
            f.render_widget(
                Block::default().title("Loading...").borders(Borders::ALL),
                area,
            );
        }
    }
}

fn centered_rect_fixed(width: u16, height: u16, area: Rect) -> Rect {
    let center_y = (area.height.saturating_sub(height)) / 2;
    let center_x = (area.width.saturating_sub(width)) / 2;

    let new_height = std::cmp::min(height, area.height);
    let new_width = std::cmp::min(width, area.width);

    Rect {
        x: area.x + center_x,
        y: area.y + center_y,
        width: new_width,
        height: new_height,
    }
}

// Helper for centering popup (Percentage based)
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
