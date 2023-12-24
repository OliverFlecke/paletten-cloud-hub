use std::{sync::Arc, time::Duration};

use anyhow::{anyhow, Context, Result};
use bytes::Bytes;
use lazy_static::lazy_static;
use rumqttc::v5::{
    mqttbytes::{
        v5::{Filter, Packet, Publish},
        QoS::{self, ExactlyOnce},
    },
    AsyncClient,
    Event::{Incoming, Outgoing},
    EventLoop, MqttOptions,
};
use tokio::sync::Mutex;

use crate::{
    db::TemperatureDatabase,
    models::{Heater, HeaterState, Measurement},
};

const MQTT_ID: &str = "paletten-cloud-hub";
const MQTT_HOST: &str = "mqtt.oliverflecke.me";
const MQTT_PORT: u16 = 1883;

lazy_static! {
    static ref HEATERS: Vec<Heater> = vec![
        Heater::new("C4402D".to_string(), "Spisebord".to_string()),
        Heater::new("C431FB".to_string(), "Sofa".to_string()),
        Heater::new("10DB9C".to_string(), "Soveværelse".to_string()),
    ];
}

type MqttHandler = (AsyncClient, EventLoop);

/// Create a mqtt handler with the default broker and configuration.
pub fn create_mqtt_handler() -> MqttHandler {
    let mut mqtt_options = MqttOptions::new(MQTT_ID, MQTT_HOST, MQTT_PORT);
    mqtt_options.set_keep_alive(Duration::from_secs(5));

    AsyncClient::new(mqtt_options, 10)
}

/// Struct to listen and adjust heater state based on a desired state.
pub struct Controller {
    client: AsyncClient,
    eventloop: EventLoop,
    state: State,
    db: Arc<Mutex<dyn TemperatureDatabase>>,
}

impl Controller {
    pub fn new((client, eventloop): MqttHandler, db: Arc<Mutex<dyn TemperatureDatabase>>) -> Self {
        Self {
            client,
            eventloop,
            state: State::default(),
            db,
        }
    }

    /// Execute the controllers loop until completion. Run as a `Future` that
    /// must be polled. Best used with `tokio::spawn`.
    pub async fn run_until_completion(mut self) -> Result<()> {
        self.client
            .subscribe_many([
                Filter::new("temperature/+", ExactlyOnce),
                Filter::new("measurement/+", ExactlyOnce),
                Filter::new("shellies/+/relay/0", ExactlyOnce),
            ])
            .await
            .context("Failed to subscribe to topics")?;

        loop {
            match self.eventloop.poll().await {
                Ok(notification) => {
                    match notification {
                        Incoming(incoming) => {
                            if let Err(e) = self.handle_incoming_message(incoming).await {
                                tracing::error!(error = %e, "Error when handling incomming message");
                            }
                        }

                        // Do nothing for outgoing requests
                        Outgoing(_) => {}
                    };
                }
                Err(e) => tracing::error!(error = %e),
            }
        }
    }

    /// Handle incoming message.
    async fn handle_incoming_message(&mut self, message: Packet) -> Result<()> {
        if let Packet::Publish(Publish { topic, payload, .. }) = message {
            match topic.as_ref() {
                b"temperature/set" => {
                    let desired_temperature = parse_float_payload(&payload)?;
                    self.set_desired_temperature(desired_temperature).await?;
                }
                b"temperature/inside" => {
                    let temperature = parse_float_payload(&payload)?;
                    self.set_inside_temperature(temperature).await?;
                }
                b"temperature/auto" if payload.as_ref() == b"true" => {
                    tracing::info!(state = ?self.state, "Temperature control enabled");
                    self.state.enabled = true;
                }
                b"temperature/auto" if payload.as_ref() == b"false" => {
                    tracing::info!(state = ?self.state, "Temperature control disabled");
                    self.state.enabled = false;
                }
                _ if topic.as_ref().starts_with(b"measurement/") => {
                    self.handle_measurement(topic.as_ref(), payload.as_ref())
                        .await?;
                }
                _ if topic.as_ref().starts_with(b"shellies/") => {
                    tracing::debug!("From `{:?}` received command: {:?}", topic, payload);
                }

                _ => {}
            }
        } else {
            tracing::trace!(incoming = ?message, "Unhandled incoming message");
        }

        Ok(())
    }

    /// Track the latest temperature received.
    #[tracing::instrument(skip(self), fields(state = ?self.state))]
    async fn set_inside_temperature(&mut self, temperature: f64) -> Result<()> {
        self.state.current_temperature = Some(temperature);
        self.check_temperature().await?;
        Ok(())
    }

    /// Set the desired temperature for the controller.
    #[tracing::instrument(skip(self), fields(state = ?self.state))]
    async fn set_desired_temperature(&mut self, temperature: f64) -> Result<()> {
        self.state.desired_temperature = Some(temperature);
        self.check_temperature().await?;
        Ok(())
    }

    /// Check the current temperature against the desired temperature and
    /// update the heaters as needed.
    #[tracing::instrument(skip(self), fields(state = ?self.state))]
    async fn check_temperature(&mut self) -> Result<()> {
        if !self.state.enabled {
            tracing::info!(state = ?self.state, "Controller is disabled");
            return Ok(());
        }

        let Some(heater_state) = self.state.get_heater_state() else {
            tracing::warn!(state = ?self.state, "Missing desired or current temperature");
            return Ok(());
        };

        self.set_heaters_state(heater_state)
            .await
            .context("Failed to set heater state")
    }

    /// Set the heaters to either on or off.
    #[tracing::instrument(skip(self))]
    async fn set_heaters_state(&mut self, state: HeaterState) -> Result<()> {
        for heater in HEATERS.iter() {
            self.client
                .publish(
                    format!("shellies/shelly1-{}/relay/0/command", heater.id()),
                    QoS::AtLeastOnce,
                    true,
                    state.to_string(),
                )
                .await
                .context("Failed to publish to MQTT")?;
        }

        Ok(())
    }

    /// Handle receiving a measurement reading.
    #[tracing::instrument(skip(self, topic, payload))]
    async fn handle_measurement(&mut self, topic: &[u8], payload: &[u8]) -> Result<()> {
        tracing::trace!("Receved {:?} => {:?}", topic, payload);
        let re = regex::bytes::Regex::new(r#"measurement/(?<place>inside|outside)"#)
            .expect("invalid regex");
        let Some(place) = re
            .captures(topic.as_ref())
            .and_then(|m| m.name("place"))
            .and_then(|x| std::str::from_utf8(x.as_bytes()).ok())
        else {
            return Err(anyhow!(
                "Received measurement from unknown place: '{:?}'",
                topic
            ));
        };

        let Ok(measurement) = serde_json::from_slice::<Measurement>(payload) else {
            return Err(anyhow!("Failed to deserialize payload: {:?}", payload));
        };

        tracing::info!("{} => {:?}", place, measurement);

        self.db
            .lock()
            .await
            .insert_reading(
                place,
                *measurement.temperature() as i64,
                *measurement.humidity() as i64,
            )
            .await?;

        Ok(())
    }
}

/// Represents the state of the heating system, including whether the automated
/// temperature control is enabled or not.
#[derive(Debug, Default)]
struct State {
    enabled: bool,
    desired_temperature: Option<f64>,
    current_temperature: Option<f64>,
}

impl State {
    pub fn get_heater_state(&self) -> Option<HeaterState> {
        self.desired_temperature
            .zip(self.current_temperature)
            .map(|(desired, current)| {
                if desired > current {
                    HeaterState::On
                } else {
                    HeaterState::Off
                }
            })
    }
}

/// Parse `Bytes` which represents the string representation of a float.
fn parse_float_payload(payload: &Bytes) -> Result<f64> {
    payload
        .escape_ascii()
        .to_string()
        .parse::<f64>()
        .context("Failed to parse temperature to float")
}
