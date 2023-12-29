use anyhow::{Context, Result};
use chrono::NaiveDateTime;
use sqlx::SqlitePool;

use crate::models::HeaterState;

/// Create a connection pool from the given connection string to a Sqlite database.
pub async fn create_db_pool(connection_string: &str) -> Result<SqlitePool> {
    SqlitePool::connect(connection_string)
        .await
        .context("Failed to connect to database")
}

/// Represents the layer to the database.
#[derive(Debug)]
pub struct Database {
    db_pool: SqlitePool,
}

impl Database {
    /// Create a new database instances.
    pub async fn new(db_pool: SqlitePool) -> Result<Self> {
        tracing::trace!("Applying migrations");
        sqlx::migrate!().run(&db_pool).await?;
        tracing::trace!("Migrations completed");

        Ok(Self { db_pool })
    }

    /// Get the history of temperatures for the last 24 hours.
    #[tracing::instrument(skip(self))]
    pub async fn get_history_from_last_24_hours(
        &self,
    ) -> Result<Vec<TemperatureMeasurementRecord>> {
        sqlx::query_as!(
            TemperatureMeasurementRecord,
            "SELECT * FROM history WHERE timestamp > date('now', '-1 day')"
        )
        .fetch_all(&self.db_pool)
        .await
        .context("Failed to fetch history of time measurements")
    }

    /// Insert a temperature measurement into the database.
    #[tracing::instrument(skip(self))]
    pub async fn insert_reading(
        &self,
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
        .execute(&self.db_pool)
        .await
        .context("Failed to insert measurement")?;

        Ok(())
    }

    /// Record a state for a given heater.
    #[tracing::instrument(skip(self))]
    pub async fn insert_heater_state(&self, heater_id: &str, state: HeaterState) -> Result<()> {
        sqlx::query!("INSERT INTO heater_history (timestamp, shelly_id, is_active) VALUES (current_timestamp, ?, ?)", heater_id, state)
            .execute(&self.db_pool).await.context("Failed to insert heater history")?;

        Ok(())
    }
}

// Allowing unused code for now, as we want to have a struct representing the database records.
#[allow(unused)]
#[derive(Debug)]
#[cfg_attr(test, derive(sqlx::FromRow))]
pub struct TemperatureMeasurementRecord {
    timestamp: NaiveDateTime,
    location: String,
    temperature: i64,
    humidity: i64,
}

#[allow(unused)]
#[derive(Debug)]
pub struct HeaterHistoryRecord {
    timestamp: NaiveDateTime,
    shelly_id: String,
    is_active: bool,
}

#[cfg(test)]
mod test {
    use fake::{Fake, Faker};

    use super::*;

    #[sqlx::test]
    fn insert_reading(pool: SqlitePool) {
        // Arrange
        let subject = Database::new(pool.clone()).await.unwrap();
        let location: String = Faker.fake();
        let temperature: i64 = Faker.fake();
        let humidity: i64 = Faker.fake();

        // Act
        subject
            .insert_reading(&location, temperature, humidity)
            .await
            .expect("inserting reading not to fail");

        // Assert
        let results = sqlx::query_as::<_, TemperatureMeasurementRecord>("select * from history")
            .fetch_all(&pool)
            .await
            .expect("query failed");
        assert_eq!(results.len(), 1);
        let row = results.get(0).unwrap();
        assert_eq!(row.location, location);
        assert_eq!(row.temperature, temperature);
        assert_eq!(row.humidity, humidity);
    }
}
