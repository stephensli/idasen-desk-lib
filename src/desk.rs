use std::collections::{BTreeSet, HashMap};

use btleplug::api::{Characteristic, Peripheral as _, PeripheralProperties, WriteType};
use btleplug::platform::Peripheral;

use super::error::DeskError;

static UUID_HEIGHT: &str = "99fa0021-338a-1024-8a49-009c0215f78a";
static UUID_COMMAND: &str = "99fa0002-338a-1024-8a49-009c0215f78a";
static UUID_REFERENCE_INPUT: &str = "99fa0031-338a-1024-8a49-009c0215f78a";

// Not currently used but can be used to determine if the given device is a desk or not. If it is
// a desk then the services (services_uuid) list will contain this uuid.
static UUID_ADV_SVC: &str = "99fa0001-338a-1024-8a49-009c0215f78a";

static MAX_HEIGHT: f32 = 1.27;
static MIN_HEIGHT: f32 = 0.62;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
}

pub(crate) struct Desk {
    name: String,
    peripheral: Peripheral,
    desk_properties: PeripheralProperties,
    desk_characteristics: BTreeSet<Characteristic>,
    characteristics_map: HashMap<String, Characteristic>,
}

impl Desk {
    /// Create a new instance of the desk.
    ///
    /// # Arguments
    ///
    /// * `peripheral`: The desk Peripheral for communicating over bluetooth.
    ///
    /// returns: Desk
    ///
    pub async fn new(peripheral: Peripheral) -> Desk {
        let desk_properties = peripheral.properties().await.unwrap().unwrap();
        let desk_characteristics = peripheral.characteristics();

        let name = desk_properties.local_name.as_ref().unwrap();
        let desk_characteristics_map = get_character_map(&desk_characteristics);

        log::debug!("created new instance of device {:?}", name);

        Desk {
            name: name.to_string(),
            desk_properties,
            peripheral,
            desk_characteristics,
            characteristics_map: desk_characteristics_map,
        }
    }

    /// Get the current height of the desk by communicating over bluetooth
    ///
    /// returns: <Result<f32, btleplug::Error>
    pub async fn get_height(&self) -> Result<f32, btleplug::Error> {
        let characteristic = self.characteristics_map.get(UUID_HEIGHT).unwrap();
        let height_value = self.peripheral.read(characteristic).await?;

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

        // we call into both kinds since command char and ref char, linux
        // plays up if and when we use the normal method of calling.
        let (_, _) = tokio::join!(
            self.peripheral
                .write(command_char, &command_stop, WriteType::WithoutResponse),
            self.peripheral
                .write(ref_char, &command_ref_input, WriteType::WithoutResponse)
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
    pub async fn move_to_target(&self, target: f32) -> Result<(), DeskError> {
        if target > MAX_HEIGHT {
            return Err(DeskError::TargetHeightTooHigh(target));
        }

        if target < MIN_HEIGHT {
            return Err(DeskError::TargetHeightTooLow(target));
        }

        let mut previous_height = self.get_height().await?;
        let will_move_up = target > previous_height;

        loop {
            let height = self.get_height().await?;
            let difference = target - height;

            log::debug!(
                "target={:?}, height={:?}, difference={:?}",
                target,
                height,
                difference
            );

            // the device has a moving action to protect the user if it applies pressure  to
            // something when moving. This will result in the desk moving in the opposite direction
            // when the device detects something. Moving out th way. If we detect this, stop.
            if (height < previous_height && will_move_up)
                || height > previous_height && !will_move_up
            {
                log::warn!("stopped moving because desk safety feature kicked in.");
                return Err(DeskError::DeskMoveSafetyKickedIn);
            }

            // if we are within our tolerance for moving the desk then we can go and stop the moving.
            // testing. Additionally ensure to stop first to keep in line with our tolerance.
            // Otherwise a shift in the difference could occur when pulling the final destination.
            if difference.abs() < 0.005 {
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

            previous_height = height;
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
    pub async fn move_direction(&self, direction: Direction) -> Result<(), DeskError> {
        let command_characteristic = self.characteristics_map.get(UUID_COMMAND).unwrap();

        let command = if direction == Direction::Up {
            vec![0x47, 0x00]
        } else {
            vec![0x46, 0x00]
        };

        self.peripheral
            .write(command_characteristic, &command, WriteType::WithoutResponse)
            .await?;

        Ok(())
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
