use color_eyre::config::HookBuilder;
use ratatui::{
    backend::{Backend, CrosstermBackend},
    crossterm::{
        ExecutableCommand,
        event::{self, Event, KeyCode, KeyEventKind},
        terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    },
    layout::{Constraint, Direction, Layout},
    terminal::Terminal,
    widgets::{Block, List, ListState},
};
use std::io::stdout;

fn init_error_hooks() -> color_eyre::Result<()> {
    let (panic, error) = HookBuilder::default().into_hooks();
    let panic = panic.into_panic_hook();
    let error = error.into_eyre_hook();
    color_eyre::eyre::set_hook(Box::new(move |e| {
        let _ = restore_terminal();
        error(e)
    }))?;
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        panic(info);
    }));
    Ok(())
}

fn init_terminal() -> color_eyre::Result<Terminal<impl Backend>> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout());
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

fn restore_terminal() -> color_eyre::Result<()> {
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

fn list_vertical(keycode: KeyCode, list_state: &mut ListState, max_len: usize) {
    match keycode {
        KeyCode::Down => {
            let tmp = list_state.selected_mut().as_mut().unwrap();
            if *tmp < max_len - 1 {
                *tmp += 1;
            } else {
                *tmp = 0;
            }
        }
        KeyCode::Up => {
            let tmp = list_state.selected_mut().as_mut().unwrap();
            if *tmp > 0 {
                *tmp -= 1;
            } else {
                *tmp = max_len - 1;
            }
        }
        _ => unreachable!(),
    }
}

pub fn ask_area(
    area_list: &Vec<(String, Vec<(String, String)>)>,
) -> Result<&str, Box<dyn std::error::Error>> {
    init_error_hooks()?;
    let mut terminal = init_terminal()?;
    let title = "Live Area";
    let length = area_list.len();
    let lengths = area_list.iter().map(|(_, li)| li.len()).collect::<Vec<_>>();
    let area_items = area_list
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    let left_length = area_items
        .iter()
        .map(String::len)
        .max()
        .unwrap()
        .max(title.len());
    println!("{}", left_length);
    let list = List::new(area_items)
        .block(Block::bordered().title(title))
        .highlight_style(
            ratatui::style::Style::new().add_modifier(ratatui::style::Modifier::REVERSED),
        )
        .highlight_symbol("> ");
    let lists = area_list
        .iter()
        .map(|(title, li)| {
            List::new(li.iter().map(|(name, _)| name.clone()).collect::<Vec<_>>())
                .block(Block::bordered().title(title.as_str()))
                .highlight_style(
                    ratatui::style::Style::new().add_modifier(ratatui::style::Modifier::REVERSED),
                )
                .highlight_symbol(">> ")
        })
        .collect::<Vec<_>>();

    let mut list_state = ListState::default();
    list_state.select(Some(0));
    let mut list_states = vec![ListState::default(); length];
    let mut is_left = true;

    let area = loop {
        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                let cur_idx = list_state.selected().unwrap();
                match key.code {
                    KeyCode::Left => {
                        if !is_left {
                            is_left = !is_left;
                            list_states[cur_idx].select(None);
                        }
                    }
                    KeyCode::Right => {
                        if is_left {
                            is_left = !is_left;
                            list_states[cur_idx].select(Some(0));
                        }
                    }
                    KeyCode::Up | KeyCode::Down => {
                        if is_left {
                            list_vertical(key.code, &mut list_state, length);
                        } else {
                            list_vertical(key.code, &mut list_states[cur_idx], lengths[cur_idx]);
                        }
                    }
                    KeyCode::Enter => {
                        if !is_left {
                            let cur_idx_r = list_states[cur_idx].selected().unwrap();
                            break &area_list[cur_idx].1[cur_idx_r].1;
                        }
                    }
                    _ => {}
                }
            }
        }

        terminal.draw(|frame| {
            let [left_area, right_area] = Layout::new(
                Direction::Horizontal,
                [
                    Constraint::Length((left_length + 4) as u16),
                    Constraint::Fill(1),
                ],
            )
            .areas(frame.size());
            frame.render_stateful_widget(&list, left_area, &mut list_state);
            let cur_idx = list_state.selected().unwrap();
            frame.render_stateful_widget(&lists[cur_idx], right_area, &mut list_states[cur_idx]);
        })?;
    };
    restore_terminal()?;
    Ok(area)
}
