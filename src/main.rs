use std::error::Error;

use btleplug::platform::Manager;
use env_logger::Target;

use crate::desk::Desk;

mod bluetooth;
mod desk;
mod error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    env_logger::builder().target(Target::Stdout).init();

    let manager = Manager::new().await.unwrap();
    let desk_peripheral = bluetooth::find_desk_adapter(&manager, true).await?;

    let desk = Desk::new(desk_peripheral).await;

    println!("{}", desk.get_height().await?);

    if desk.get_height().await? > 1.0 {
        desk.move_to_target(0.74).await?;
    } else {
        desk.move_to_target(1.12).await?;
    }

    Ok(())
}
