use std::collections::{BTreeSet, HashMap};
use std::env::var;
use std::sync::Arc;

use btleplug::api::{BDAddr, Characteristic, Peripheral as _, PeripheralProperties, WriteType};
use btleplug::platform::{Manager, Peripheral};
use futures::StreamExt;
use tokio::sync::RwLock;
use tokio::time::Instant;

use crate::{bluetooth, DeskError};
use crate::DeskError::CannotSubscribePosition;

static UUID_HEIGHT: &str = "99fa0021-338a-1024-8a49-009c0215f78a";
static UUID_COMMAND: &str = "99fa0002-338a-1024-8a49-009c0215f78a";
static UUID_REFERENCE_INPUT: &str = "99fa0031-338a-1024-8a49-009c0215f78a";

// Not currently used but can be used to determine if the given device is a desk or not. If it is
// a desk then the services (services_uuid) list will contain this uuid.
#[allow(dead_code)]
static UUID_ADV_SVC: &str = "99fa0001-338a-1024-8a49-009c0215f78a";

static MAX_HEIGHT: f32 = 1.27;
static MIN_HEIGHT: f32 = 0.62;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
}

pub struct Desk {
    pub name: String,
    peripheral: Arc<RwLock<Peripheral>>,
    desk_properties: PeripheralProperties,
    desk_characteristics: BTreeSet<Characteristic>,
    characteristics_map: HashMap<String, Characteristic>,
}

impl Desk {
    pub async fn new(mac_address: &str) -> Result<Desk, DeskError> {
        let manager = Manager::new().await.unwrap();

        let address = mac_address.parse::<BDAddr>().unwrap();
        let desk_peripheral = bluetooth::find_desk_adapter(address, &manager, true).await?;

        Ok(Desk::from_peripheral(desk_peripheral).await?)
    }

    /// Create a new instance of the desk from a bluetooth peripheral.
    ///
    /// # Arguments
    ///
    /// * `peripheral`: The desk Peripheral for communicating over bluetooth.
    ///
    /// returns: Desk
    ///
    pub async fn from_peripheral(peripheral: Peripheral) -> Result<Desk, DeskError> {
        let desk_properties = peripheral.properties().await.unwrap().unwrap();
        let desk_characteristics = peripheral.characteristics();

        let name = desk_properties.local_name.as_ref().unwrap();
        let desk_characteristics_map = get_character_map(&desk_characteristics);

        if peripheral.subscribe(desk_characteristics_map.get(UUID_HEIGHT).unwrap()).await.is_err() {
            return Err(DeskError::CannotSubscribePosition);
        }

        log::debug!("created new instance of device {:?}", name);

        let desk = Desk {
            name: name.to_string(),
            desk_properties,
            peripheral: Arc::new(RwLock::new(peripheral)),
            desk_characteristics,
            characteristics_map: desk_characteristics_map,
        };

        // desk.read_height_notifications().await;


        Ok(desk)
    }

    /// Get the current height of the desk by communicating over bluetooth
    ///
    /// returns: <Result<f32, btleplug::Error>
    pub async fn get_height(&self) -> Result<f32, btleplug::Error> {
        let characteristic = self.characteristics_map.get(UUID_HEIGHT).unwrap();
        let height_value = self.peripheral.read().await.read(characteristic).await?;

        Ok(bytes_to_meters(height_value))
    }

    /// Tell the desk to stop moving.
    ///
    /// The desk does not stop automatically unless the safety kicks in, otherwise move action
    /// move the desk in steps of 1 second.
    ///
    /// returns: Result<(), Error>
    ///
    pub async fn stop(&self) -> Result<(), btleplug::Error> {
        let command_char = self.characteristics_map.get(UUID_COMMAND).unwrap();
        let ref_char = self.characteristics_map.get(UUID_REFERENCE_INPUT).unwrap();

        let command_stop = vec![0xFF, 0x00];
        let command_ref_input = vec![0x01, 0x80];

        let per = self.peripheral.read().await;

        // we call into both kinds since command char and ref char, linux
        // plays up if and when we use the normal method of calling.
        let (_, _) = tokio::join!(
            per.write(command_char, &command_stop, WriteType::WithoutResponse),
            per.write(ref_char, &command_ref_input, WriteType::WithoutResponse)
        );

        Ok(())
    }

    /// Move the desk to the specified target float value. Within the constraints of the device
    /// min value and max value.
    ///
    /// # Arguments
    ///
    /// * `target`: The target float value.
    ///
    /// returns: Result<(), DeskError>
    ///
    /// # Examples
    ///
    /// ```
    /// // will error with target height too high
    /// desk.move_to_target(1.28);
    ///
    /// // will error with target height too low
    /// desk.move_to_target(0.61);
    ///
    /// // valid action
    /// desk.move_to_target(0.74);
    /// ```
    pub async fn move_to_target(&self, target: f32) -> Result<(), super::DeskError> {
        if target > MAX_HEIGHT {
            return Err(super::DeskError::TargetHeightTooHigh(target));
        }

        if target < MIN_HEIGHT {
            return Err(super::DeskError::TargetHeightTooLow(target));
        }

        let mut previous_height = self.get_height().await?;
        let mut previous_height_read_at = Instant::now();

        let will_move_up = target > previous_height;

        log::info!("moving desk from {:?} to {:?}", previous_height, target);

        loop {
            let current_height = self.get_height().await?;

            let elapsed_milliseconds = previous_height_read_at.elapsed().as_millis();
            let difference = target - current_height;

            let speed = (difference.abs() as f64 / elapsed_milliseconds as f64) * 100.0;


            log::debug!(
                "target={:?}, current_height={:?} previous_height={:?}, difference={:?}, time_elapsed_milliseconds={:?}, speed={:?}",
                target,
                current_height,
                previous_height,
                difference,
                elapsed_milliseconds,
                speed
            );

            // the device has a moving action to protect the user if it applies pressure  to
            // something when moving. This will result in the desk moving in the opposite direction
            // when the device detects something. Moving out th way. If we detect this, stop.
            //
            // only if our difference is not nothing, meaning we are not doing a minor correction.
            if ((current_height < previous_height && will_move_up)
                || current_height > previous_height && !will_move_up) && difference > 0.010
            {
                log::warn!("stopped moving because desk safety feature kicked in.");
                return Err(super::DeskError::DeskMoveSafetyKickedIn);
            }

            // If we're either:
            // * less than 10 millimetres, or:
            // * less than half a second from target
            // then we need to stop every iteration so that we don't overshoot
            if difference.abs() < (speed / 2.0).max(0.010) as f32 {
                log::info!("hit diff stop");
                self.stop().await?;
            }

            // if we are within our tolerance for moving the desk then we can go and stop the moving.
            // testing. Additionally ensure to stop first to keep in line with our tolerance.
            // Otherwise a shift in the difference could occur when pulling the final destination.
            //
            // within 5mm
            if difference.abs() <= 0.005 {
                self.stop().await?;

                let height = self.get_height().await?;

                log::info!("reached target of {:?}, actual: {:?}", target, height);

                return Ok(());
            }

            if difference > 0.0 {
                self.move_direction(Direction::Up).await?;
            } else if difference < 0.0 {
                self.move_direction(Direction::Down).await?;
            }

            previous_height = self.get_height().await?;
            previous_height_read_at = Instant::now();
        }
    }

    /// Based on the provided direction, the desk will be told to start moving moving up or start
    /// moving down. A move action will only occur for a 1 second interval which is configured
    /// by the desk.
    ///
    /// # Arguments
    ///
    /// * `direction`: The direction in which to move.
    ///
    /// returns: Result<(), DeskError>
    ///
    /// # Examples
    ///
    /// ```
    ///
    /// ```
    pub async fn move_direction(&self, direction: Direction) -> Result<(), super::DeskError> {
        let command_characteristic = self.characteristics_map.get(UUID_COMMAND).unwrap();

        let command = if direction == Direction::Up {
            vec![0x47, 0x00]
        } else {
            vec![0x46, 0x00]
        };

        self.peripheral
            .read().await
            .write(command_characteristic, &command, WriteType::WithoutResponse)
            .await?;

        Ok(())
    }

    async fn read_height_notifications(&self) {
        let mut notifications_stream = self.peripheral.read().await.notifications().await.unwrap().take(10);

        while let Some(notification) = notifications_stream.next().await {
            println!("{:?}", notification);
        }
    }
}

impl ToString for Desk {
    fn to_string(&self) -> String {
        let mut result = format!(
            "id: {:?}\nname: {:?}\n\ncharacteristics:",
            self.desk_properties.address, self.name,
        );

        for x in &self.desk_characteristics {
            result += format!(
                "\nuuid: {:?}\nservice uuid: {:?}\nproperties: {:?}\n",
                x.uuid, x.service_uuid, x.properties
            )
                .as_str()
        }

        result
    }
}

/// Helper to convert the provided BTreeSet to a map to easily access the characteristics based on
/// a given uuid value.
///
/// # Arguments
///
/// * `characters`:
///
/// returns: HashMap<String, Characteristic, RandomState>
///
fn get_character_map(characters: &BTreeSet<Characteristic>) -> HashMap<String, Characteristic> {
    let mut mapping: HashMap<String, Characteristic> = HashMap::new();

    for x in characters {
        mapping.insert(x.uuid.to_string(), x.clone());
    }

    mapping
}

/// Converts the raw height response from the desk into meters.
///
/// # Arguments
///
/// * `raw`: The raw byte response from the desk.
///
/// returns: f32
fn bytes_to_meters(raw: Vec<u8>) -> f32 {
    let high_byte = raw[1] as i32;
    let low_byte = raw[0] as i32;

    let number = (high_byte << 8) + low_byte;
    (number as f32 / 10000.0) + MIN_HEIGHT
}
