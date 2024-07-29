mod login;
mod live;
mod cli;
mod tui;

use std::path::Path;
use chrono::{Datelike, Utc, DateTime};
use login::LoginData;

async fn login<P: AsRef<Path>>(data_path: P) -> Result<(LoginData, DateTime<Utc>), Box<dyn std::error::Error>> {
    let now = Utc::now();
    let today = (now.year(), now.month(), now.day());

    let mut login_data = match login::LoginData::load(&data_path) {
        Ok(login_data) => login_data,
        _ => {
            let (refresh_token, url) = loop {
                match login::login().await {
                    Ok(result) => break result,
                    Err(e) => println!("{:?}", e)
                }
            };
            let binding = reqwest::Url::parse(&url).unwrap();
            let login_data = login::LoginData {
                cookies: binding.query_pairs().into_owned().collect(),
                refresh_token,
                last_run: today,
                area: None
            };
            login_data.dump(&data_path)?;
            login_data
        }
    };

    if login_data.last_run != today {
        login_data.last_run = today;
        login_data.refresh_cookie().await?;
        login_data.dump(&data_path)?;
    }

    Ok((login_data, now))
}

async fn valid_area(area: &str) -> Result<bool, Box<dyn std::error::Error>> {
    Ok(live::live_area_list().await?.into_iter()
        .any(|(_, li)| 
            li.into_iter().any(|(_, id)| &id == area)))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data_path = {
        let mut data_path = dirs::home_dir().unwrap();
        data_path.push("bili-live-cookies.json");
        data_path
    };
    let cmds = cli::build_commands();
    match cmds.get_matches().subcommand() {
        Some(("status", _)) => {
            let (login_data, now) = login(&data_path).await?;
            let uid = login_data.cookies["DedeUserID"].as_str();
            let ((living, start_time), (area_id, area_name, cover_url)) = live::get_live_status(&uid).await?;
            let start_time = DateTime::from_timestamp(start_time as i64, 0).ok_or("live start time out of range")?;
            let mut pairs = vec![("is living".to_string(), living.to_string())];
            if living {
                pairs.push(("start time".to_string(), start_time.to_string()));
                pairs.push((
                    "live duration".to_string(),
                    {
                        let mut delta = (
                            now - start_time
                        ).num_seconds();
                        let sec = delta % 60;
                        delta /= 60;
                        let min = delta % 60;
                        delta /= 60;
                        let hour = delta;
                        format!("{}:{}:{}", hour, min, sec)
                    }
                ));
                pairs.push((
                    "area".to_string(),
                    format!("{}[{}]", area_name, area_id)
                ))
            }
            cli::print_image(&cover_url).await?;
            cli::print_pairs(&"status", &pairs);
        },
        Some(("start", arg_match)) => {
            let (mut login_data, _) = login(&data_path).await?;
            let area = arg_match.get_one::<String>("area");
            let area = match (area, login_data.area) {
                (Some(area), _) if valid_area(area).await? => {
                    login_data.area = Some(area.clone());
                    login_data.dump(&data_path)?;
                    area.clone()
                },
                (None, Some(area)) => area,
                _ => {
                    let area_list = live::live_area_list().await?;
                    let area = tui::ask_area(&area_list)?.to_string();
                    login_data.area = Some(area.clone());
                    login_data.dump(&data_path)?;
                    area
                }
            };
            let ((addr, code), message) = live::start_live(&login_data.cookies, &area).await?;
            let mut pairs = vec![
                ("addr".to_string(), addr),
                ("code".to_string(), code),
            ];
            if !message.is_empty() {
                pairs.push(("message".to_string(), message));
            }
            cli::print_pairs(&"start", &pairs);
        },
        Some(("stop", _)) => {
            let (login_data, _) = login(&data_path).await?;
            let message = live::stop_live(&login_data.cookies).await?;
            let mut pairs = vec![];
            if !message.is_empty() {
                pairs.push(("message".to_string(), message));
            }
            cli::print_pairs(&"stop", &pairs);
        },
        Some(("clean", arg_match)) => {
            let area = *arg_match.get_one::<bool>("area").unwrap();
            if area {
                let (mut login_data, _) = login(&data_path).await?;
                login_data.area = None;
                login_data.dump(&data_path)?;
            }
            else {
                std::fs::remove_file(&data_path)?;
            }
        },
        Some((cmd, _)) => panic!("{}", cmd),
        None => cli::build_commands().print_help()?,
    }
    Ok(())
}