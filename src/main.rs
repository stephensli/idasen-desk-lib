use std::error::Error;

use btleplug::platform::{Manager, Peripheral};
use clap::Parser;
use env_logger::Target;
use log::{log_enabled, Level};

use crate::desk::Desk;

mod bluetooth;
mod desk;
mod error;

#[derive(Parser, Debug)]
#[clap(about, version, author)]
struct Args {
    #[clap(long)]
    sit: bool,

    #[clap(long)]
    stand: bool,

    #[clap(long = "move", short = 'm')]
    move_to: Option<u8>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    env_logger::builder().target(Target::Stdout).init();

    let cli_arguments: Args = Args::parse();

    log::debug!("input arguments {:?}", cli_arguments);

    let manager = Manager::new().await.unwrap();
    let desk_peripheral = bluetooth::find_desk_adapter(&manager, true).await?;

    let desk = Desk::new(desk_peripheral).await;

    // handle the case in which the device target amount was specified. // we allow this being a
    // whole number, e.g 74, which will be later converted into a float value.
    if let Some(target_value) = cli_arguments.move_to {
        desk.move_to_target((target_value as f32) / 100.0).await?;
        return Ok(());
    }

    if log_enabled!(Level::Debug) {
        log::debug!(desk.to_string())
    }

    let current_desk_height = desk.get_height().await?;
    log::debug!("starting desk position {:?}", current_desk_height);

    // if the user has specified sit or stand.
    if cli_arguments.stand {
        desk.move_to_target(1.12).await?;
        return Ok(());
    } else if cli_arguments.sit {
        desk.move_to_target(0.74).await?;
        return Ok(());
    }

    // otherwise lets go and determine it and do it ourself.
    if current_desk_height > 1.0 {
        desk.move_to_target(0.74).await?;
    } else {
        desk.move_to_target(1.12).await?;
    }

    Ok(())
}
