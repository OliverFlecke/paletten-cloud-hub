use std::time::Duration;

use anyhow::{Context, Result};
use rumqttc::v5::{
    mqttbytes::{
        v5::{Packet, Publish},
        QoS::ExactlyOnce,
    },
    AsyncClient,
    Event::{Incoming, Outgoing},
    EventLoop, MqttOptions,
};

const MQTT_ID: &str = "paletten-cloud-hub";
const MQTT_HOST: &str = "mqtt.oliverflecke.me";
const MQTT_PORT: u16 = 1883;

#[derive(Debug, Default)]
struct State {
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
                    self.set_desired_temperature(desired_temperature).await;
                }
                _ if topic.as_ref().starts_with(b"temperature/") => {}
                _ => {}
            }
        } else {
            tracing::trace!(incoming = ?message, "Unhandled incoming message");
        }

        Ok(())
    }

    #[tracing::instrument(skip(self))]
    async fn set_desired_temperature(&mut self, temperature: f64) {
        self.state.desired_temperature = Some(temperature);
        self.check_temperature().await;
    }

    /// Check the current temperature against the desired temperature and
    /// update the heaters as needed.
    #[tracing::instrument(skip(self))]
    async fn check_temperature(&mut self) {
        let Some(desired_temperature) = self.state.desired_temperature else {
            tracing::warn!("Missing desired temperature");
            return;
        };
        let Some(current_temperature) = self.state.current_temperature else {
            tracing::warn!("Missing current temperature");
            return;
        };

        if let Err(e) = self
            .set_heaters_state(if desired_temperature < current_temperature {
                HeaterState::On
            } else {
                HeaterState::Off
            })
            .await
        {
            tracing::error!(error = ?e, "Failed to set heater state");
        }
    }

    /// Set the heaters to either on or off.
    #[tracing::instrument(skip(self))]
    async fn set_heaters_state(&mut self, state: HeaterState) -> Result<()> {
        // TODO: Publish message to mqtt to turn on heathers.

        Ok(())
    }
}

/// Describes the states a heater can be on.
#[derive(Debug)]
enum HeaterState {
    Off,
    On,
}
