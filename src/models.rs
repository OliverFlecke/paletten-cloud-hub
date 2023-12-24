use derive_getters::Getters;

/// Describes the states a heater can be on.
#[derive(Debug, strum::AsRefStr, strum::EnumString, strum::Display, sqlx::Type)]
#[repr(u8)]
pub enum HeaterState {
    #[strum(serialize = "off")]
    Off = 0,
    #[strum(serialize = "on")]
    On = 1,
}

#[derive(Debug, Getters)]
pub struct Heater {
    #[allow(unused)]
    name: String,
    id: String,
}

impl Heater {
    pub fn new(id: String, name: String) -> Self {
        Self { id, name }
    }
}

#[derive(Debug, serde::Deserialize, Getters)]
pub struct Measurement {
    temperature: f64,
    humidity: f64,
}
