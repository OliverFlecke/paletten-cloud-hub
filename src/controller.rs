use std::time::Duration;

use anyhow::{Context, Result};
use lazy_static::lazy_static;
use rumqttc::v5::{
    mqttbytes::{
        v5::{Packet, Publish},
        QoS::{self, ExactlyOnce},
    },
    AsyncClient,
    Event::{Incoming, Outgoing},
    EventLoop, MqttOptions,
};

use crate::models::{Heater, HeaterState};

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

#[derive(Debug, Default)]
struct State {
    enabled: bool,
    desired_temperature: Option<f64>,
    current_temperature: Option<f64>,
}

pub struct Controller {
    client: AsyncClient,
    eventloop: EventLoop,
    state: State,
}

impl Controller {
    pub async fn new() -> Self {
        let mut mqtt_options = MqttOptions::new(MQTT_ID, MQTT_HOST, MQTT_PORT);
        mqtt_options.set_keep_alive(Duration::from_secs(5));

        let (client, eventloop) = AsyncClient::new(mqtt_options, 10);

        Self {
            client,
            eventloop,
            state: State::default(),
        }
    }

    pub async fn run_until_completion(mut self) -> Result<()> {
        self.client
            .subscribe("temperature/+", ExactlyOnce)
            .await
            .context("Failed to subscribe to temperature topics")?;

        loop {
            match self.eventloop.poll().await {
                Ok(notification) => {
                    match notification {
                        // TODO: Should this actually return an error or should we just handle it here?
                        Incoming(incoming) => self.handle_incoming_message(incoming).await?,

                        // Do nothing for outgoing requests
                        Outgoing(_) => {}
                    };
                }
                Err(e) => tracing::error!(error = %e),
            }
        }
    }

    async fn handle_incoming_message(&mut self, message: Packet) -> Result<()> {
        if let Packet::Publish(Publish { topic, payload, .. }) = message {
            match topic.as_ref() {
                b"temperature/set" => {
                    let desired_temperature = payload
                        .escape_ascii()
                        .to_string()
                        .parse::<f64>()
                        .context("Failed to parse temperature to float")?;
                    self.set_desired_temperature(desired_temperature).await?;
                }
                b"temperature/inside" => {
                    let temperature = payload
                        .escape_ascii()
                        .to_string()
                        .parse::<f64>()
                        .context("Failed to parse temperature to float")?;
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

        let Some(desired_temperature) = self.state.desired_temperature else {
            tracing::warn!(state = ?self.state, "Missing desired temperature");
            return Ok(());
        };
        let Some(current_temperature) = self.state.current_temperature else {
            tracing::warn!(state = ?self.state, "Missing current temperature");
            return Ok(());
        };

        self.set_heaters_state(if desired_temperature > current_temperature {
            HeaterState::On
        } else {
            HeaterState::Off
        })
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
}
