use std::time::Duration;

use btleplug::api::{BDAddr, Central, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Adapter, Manager, Peripheral};
use tokio::time;

use crate::error::DeskError;

const RETRY_COUNT: usize = 3;

/// Locate the first adapter on the device. If the device does not support
/// or have access to bluetooth then this will fail.
///
/// # Arguments
///
/// * `manager`: The bluetooth device manager.
///
/// returns: Option<Adapter>
///
async fn find_first_adapter(manager: &Manager) -> Option<Adapter> {
    let central_adapter = manager
        .adapters()
        .await
        .expect("Unable to fetch adapter list.")
        .into_iter()
        .nth(0);

    central_adapter
}

/// Locate the desk by the given desk_address.  
///
/// # Arguments
///
/// * `desk_address`: The target address BDAddr.
/// * `central`: The bluetooth adapter used to locate said desk.
///
/// returns: Result<Option<Peripheral>, DeskError>
///
async fn find_desk(
    desk_address: BDAddr,
    central: &Adapter,
) -> Result<Option<Peripheral>, DeskError> {
    for x in central.peripherals().await? {
        if x.address() == desk_address {
            return Ok(Some(x));
        }
    }

    Ok(None)
}

/// Using the connection manager, locate a adapter from the current_device and use said adapter to
/// connect to the desk
///
/// # Arguments
///
/// * `manager`: The manager used to locate the desk.
/// * `connect`: If we should try to connect or not.
///
/// returns: Result<Peripheral, DeskError>
///
pub(crate) async fn find_desk_adapter(
    address: BDAddr,
    manager: &Manager,
    connect: bool,
) -> Result<Peripheral, DeskError> {
    let adapter = find_first_adapter(&manager).await.unwrap();

    // start scanning for devices, this could probably be something related to polling
    // instead of this method of scanning. Something that could be started earlier.
    // This is a little slow and could be faster.
    adapter.start_scan(ScanFilter::default()).await.unwrap();
    time::sleep(Duration::from_secs(3)).await;

    let desk_peripheral = find_desk(address, &adapter).await?.unwrap();

    if !connect {
        return Ok(desk_peripheral);
    }

    // lets go and try to connect to the device RETRY_COUNT times before giving up. Since sometimes
    // we can just fail for any given reason and reconnecting sometimes helps.
    for i in 0..RETRY_COUNT {
        match desk_peripheral.connect().await {
            Ok(_) => break,
            Err(e) => {
                if i == RETRY_COUNT {
                    return Err(e.into());
                }

                // sleep for a small duration to allow some buffer between the connection times, just
                // so we don't overwhelm the device being connected too.
                time::sleep(Duration::from_millis(50)).await;
            }
        }
    }

    desk_peripheral.discover_services().await?;

    Ok(desk_peripheral)
}
