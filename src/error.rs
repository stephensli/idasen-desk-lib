use thiserror::Error;

#[derive(Error, Debug)]
pub enum DeskError {
    #[error("target height {0} too high")]
    TargetHeightTooHigh(f32),

    #[error("target height {0} too low")]
    TargetHeightTooLow(f32),

    #[error("desk move safety kicked in.")]
    DeskMoveSafetyKickedIn,

    #[error("bluetooth error")]
    BluetoothError(#[from] btleplug::Error),
}
