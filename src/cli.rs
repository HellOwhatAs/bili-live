use std::fmt::Debug;
use clap::{arg, command, value_parser, ArgAction, Command};
use viuer::{Config, print};

pub fn build_commands() -> Command {
    command!() // requires `cargo` feature
        .subcommand(
            Command::new("status")
                .about("check live room status")
        )
        .subcommand(
            Command::new("start")
                .about("start live")
                .arg(
                    arg!(-a --area <AREA> "the live area")
                    .required(false)
                    .value_parser(value_parser!(String))
                )
        )
        .subcommand(
            Command::new("stop")
                .about("stop live")
        )
        .subcommand(
            Command::new("clean")
                .about("clean login data")
                .arg(
                    arg!(--area "just clean the live area data")
                    .action(ArgAction::SetTrue)
                    .required(false)
                )
        )
}

pub fn print_pairs(head: &dyn Debug, pairs: &[(String, String)]) {
    println!("{:?}:", head);
    for (k, v) in pairs {
        println!("- {}: {}", k, v);
    }
}

pub async fn print_image(img_url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let binding = reqwest::get(img_url).await?.bytes().await?;
    let img = image::load_from_memory(&binding)?;
    print(&img, &Config { truecolor: true, ..Config::default() })?;
    Ok(())
}
