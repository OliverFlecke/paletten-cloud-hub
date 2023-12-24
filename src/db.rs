use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::NaiveDateTime;
use sqlx::{Connection, SqliteConnection};

use crate::models::HeaterState;

#[async_trait]
#[cfg_attr(test, mockall::automock)]
pub trait TemperatureDatabase: Send + std::fmt::Debug {
    /// Get the history of temperatures for the last 24 hours.
    async fn get_history_from_last_24_hours(&mut self)
        -> Result<Vec<TemperatureMeasurementRecord>>;

    /// Insert a temperature measurement into the database.
    async fn insert_reading(
        &mut self,
        location: &str,
        temperature: i64,
        humidity: i64,
    ) -> Result<()>;

    /// Record a state for a given heater.
    async fn insert_heater_state(&mut self, heater_id: &str, state: HeaterState) -> Result<()>;
}

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
}

#[async_trait]
impl TemperatureDatabase for Database {
    #[tracing::instrument(skip(self))]
    async fn get_history_from_last_24_hours(
        &mut self,
    ) -> Result<Vec<TemperatureMeasurementRecord>> {
        sqlx::query_as!(
            TemperatureMeasurementRecord,
            "SELECT * FROM history WHERE timestamp > date('now', '-1 day')"
        )
        .fetch_all(&mut self.connection)
        .await
        .context("Failed to fetch history of time measurements")
    }

    #[tracing::instrument(skip(self))]
    async fn insert_reading(
        &mut self,
        location: &str,
        temperature: i64,
        humidity: i64,
    ) -> Result<()> {
        sqlx::query!(
            "INSERT INTO history(timestamp, location, temperature, humidity) VALUES (current_timestamp, ?, ?, ?)",
            location,
            temperature,
            humidity
        )
        .execute(&mut self.connection)
        .await
        .context("Failed to insert measurement")?;

        Ok(())
    }

    #[tracing::instrument(skip(self))]
    async fn insert_heater_state(&mut self, heater_id: &str, state: HeaterState) -> Result<()> {
        sqlx::query!("INSERT INTO heater_history (timestamp, shelly_id, is_active) VALUES (current_timestamp, ?, ?)", heater_id, state)
            .execute(&mut self.connection).await.context("Failed to insert heater history")?;

        Ok(())
    }
}

#[derive(Debug)]
pub struct TemperatureMeasurementRecord {
    timestamp: NaiveDateTime,
    location: String,
    temperature: i64,
    humidity: i64,
}

#[derive(Debug)]
pub struct HeaterHistoryRecord {
    timestamp: NaiveDateTime,
    shelly_id: String,
    is_active: bool,
}
