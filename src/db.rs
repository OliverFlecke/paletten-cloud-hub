use anyhow::{Context, Result};
use chrono::NaiveDateTime;
use sqlx::{Connection, SqliteConnection};

use crate::models::HeaterState;

/// Represents the layer to the database.
#[derive(Debug)]
pub struct Database {
    connection: SqliteConnection,
}

impl Database {
    /// Create a new database instances.
    pub async fn new(connection_string: &str) -> Result<Self> {
        let mut connection = SqliteConnection::connect(connection_string)
            .await
            .expect("Failed to connect to database");
        tracing::trace!("Applying migrations");
        sqlx::migrate!().run(&mut connection).await?;
        tracing::trace!("Migrations completed");

        Ok(Self { connection })
    }

    /// Get the history of temperatures for the last 24 hours.
    #[tracing::instrument(skip(self))]
    pub async fn get_history_from_last_24_hours(&mut self) -> Result<Vec<TemperatureMeasurement>> {
        sqlx::query_as!(
            TemperatureMeasurement,
            "SELECT * FROM history WHERE timestamp > date('now', '-1 day')"
        )
        .fetch_all(&mut self.connection)
        .await
        .context("Failed to fetch history of time measurements")
    }

    /// Insert a temperature measurement into the database.
    #[tracing::instrument(skip(self))]
    pub async fn insert_reading(&mut self, measurement: &TemperatureMeasurement) -> Result<()> {
        sqlx::query!(
            "INSERT INTO history(timestamp, location, temperature, humidity) VALUES (?, ?, ?, ?)",
            measurement.timestamp,
            measurement.location,
            measurement.temperature,
            measurement.humidity
        )
        .execute(&mut self.connection)
        .await
        .context("Failed to insert measurement")?;

        Ok(())
    }

    /// Record a state for a given heater.
    #[tracing::instrument(skip(self))]
    pub async fn insert_heater_state(&mut self, heater_id: &str, state: HeaterState) -> Result<()> {
        sqlx::query!("INSERT INTO heater_history (timestamp, shelly_id, is_active) VALUES (current_timestamp, ?, ?)", heater_id, state)
            .execute(&mut self.connection).await.context("Failed to insert heater history")?;

        Ok(())
    }
}

#[derive(Debug)]
pub struct TemperatureMeasurement {
    timestamp: NaiveDateTime,
    location: String,
    temperature: i64,
    humidity: i64,
}

#[derive(Debug)]
pub struct HeaterHistory {
    timestamp: NaiveDateTime,
    shelly_id: String,
    is_active: bool,
}
