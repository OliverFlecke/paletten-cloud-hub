use std::{str::FromStr, sync::Arc, time::Duration};

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
use tokio::sync::{
    mpsc::{channel, Receiver, Sender},
    Mutex,
};

use crate::{
    db::Database,
    models::{Heater, HeaterState, Measurement},
};

#[cfg(debug_assertions)]
const MQTT_ID: &str = "paletten-cloud-hub-dev";
#[cfg(not(debug_assertions))]
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
type AsyncDatabase = Arc<Mutex<Database>>;

/// Create a mqtt handler with the default broker and configuration.
pub fn create_mqtt_handler() -> MqttHandler {
    let mut mqtt_options = MqttOptions::new(MQTT_ID, MQTT_HOST, MQTT_PORT);
    mqtt_options.set_keep_alive(Duration::from_secs(5));

    AsyncClient::new(mqtt_options, 10)
}

pub async fn create(
    mqtt_client: AsyncClient,
    mqtt_eventloop: EventLoop,
    db: AsyncDatabase,
) -> Result<(Controller, Executor)> {
    let (tx, rx) = channel::<Action>(10);
    mqtt_client
        .subscribe_many([
            Filter::new("temperature/+", ExactlyOnce),
            Filter::new("measurement/+", ExactlyOnce),
            Filter::new("shellies/+/relay/0", ExactlyOnce),
        ])
        .await
        .context("Failed to subscribe to topics")?;

    let controller = Controller::new(mqtt_eventloop, tx);
    let executor = Executor::new(mqtt_client, db, rx);

    Ok((controller, executor))
}

/// An action recevied from the controller.
#[derive(Debug, Clone)]
pub enum Action {
    SetDesiredTemperature(f64),
    SetInsideTemperature(f64),
    EnableController(bool),
    RegisterMeasurement(String, Measurement),
    RegisterHeaterStateChange(String, HeaterState),
}

/// Struct to listen and adjust heater state based on a desired state.
pub struct Controller {
    eventloop: EventLoop,
    state: State,
    tx: Sender<Action>,
}

impl Controller {
    pub fn new(eventloop: EventLoop, tx: Sender<Action>) -> Self {
        Self {
            eventloop,
            state: State::default(),
            tx,
        }
    }

    /// Execute the controllers loop until completion. Run as a `Future` that
    /// must be polled. Best used with `tokio::spawn`.
    pub async fn run_until_completion(mut self) -> Result<()> {
        loop {
            match self.eventloop.poll().await {
                Ok(notification) => {
                    match notification {
                        Incoming(incoming) => match self.handle_incoming_message(incoming).await {
                            Ok(Some(action)) => {
                                if let Err(e) = self.tx.send(action).await {
                                    tracing::error!(error = %e, "Failed to send action");
                                }
                            }
                            Ok(None) => {}
                            Err(e) => {
                                tracing::error!(error = %e, "Error when handling incomming message");
                            }
                        },

                        // Do nothing for outgoing requests
                        Outgoing(_) => {}
                    };
                }
                Err(e) => tracing::error!(error = %e),
            }
        }
    }

    /// Handle incoming message.
    #[tracing::instrument(skip(self, message))]
    async fn handle_incoming_message(&mut self, message: Packet) -> Result<Option<Action>> {
        if let Packet::Publish(Publish { topic, payload, .. }) = message {
            match topic.as_ref() {
                b"temperature/set" => {
                    let desired_temperature = parse_float_payload(&payload)?;
                    Ok(Some(Action::SetDesiredTemperature(desired_temperature)))
                }
                b"temperature/inside" => {
                    let temperature = parse_float_payload(&payload)?;
                    Ok(Some(Action::SetInsideTemperature(temperature)))
                }
                b"temperature/auto" if payload.as_ref() == b"true" => {
                    tracing::info!(state = ?self.state, "Temperature control enabled");
                    self.state.enabled = true;
                    Ok(Some(Action::EnableController(true)))
                }
                b"temperature/auto" if payload.as_ref() == b"false" => {
                    tracing::info!(state = ?self.state, "Temperature control disabled");
                    self.state.enabled = false;
                    Ok(Some(Action::EnableController(false)))
                }
                _ if topic.as_ref().starts_with(b"measurement/") => {
                    let (place, measurement) = self
                        .parse_measurement(topic.as_ref(), payload.as_ref())
                        .await?;
                    Ok(Some(Action::RegisterMeasurement(place, measurement)))
                }
                _ if topic.as_ref().starts_with(b"shellies/") => {
                    let (heater_id, state) = self
                        .parse_heater_state_change_message(topic.as_ref(), payload.as_ref())
                        .await?;
                    Ok(Some(Action::RegisterHeaterStateChange(heater_id, state)))
                }

                _ => Ok(None),
            }
        } else {
            tracing::trace!(incoming = ?message, "Unhandled incoming message");
            Ok(None)
        }
    }

    /// Handle receiving a measurement reading.
    #[tracing::instrument(skip(self, topic, payload))]
    async fn parse_measurement(
        &mut self,
        topic: &[u8],
        payload: &[u8],
    ) -> Result<(String, Measurement)> {
        let re = regex::bytes::Regex::new(r#"measurement/(?<place>inside|outside)"#)
            .expect("invalid regex");
        let place = re
            .captures(topic.as_ref())
            .and_then(|m| m.name("place"))
            .and_then(|x| std::str::from_utf8(x.as_bytes()).ok())
            .ok_or_else(|| anyhow!("Received measurement from unknown place: '{:?}'", topic))?;

        let measurement = serde_json::from_slice::<Measurement>(payload)
            .map_err(|e| anyhow!("Failed to deserialize payload: {payload:?}. {e:?}"))?;

        Ok((place.to_string(), measurement))
    }

    /// Handle messages published about state changes to heaters.
    #[tracing::instrument(skip(self, topic, payload))]
    async fn parse_heater_state_change_message(
        &mut self,
        topic: &[u8],
        payload: &[u8],
    ) -> Result<(String, HeaterState)> {
        let re = regex::bytes::Regex::new(r#"shellies/shelly1-(?<id>[A-F0-9]{6})/relay/0"#)
            .expect("invalid regex");
        let heater_id = re
            .captures(topic.as_ref())
            .and_then(|m| m.name("id"))
            .and_then(|x| std::str::from_utf8(x.as_bytes()).ok())
            .ok_or_else(|| anyhow!("Received state from unknown heater: '{:?}'", topic))?;
        let state = std::str::from_utf8(payload)
            .context("payload is not utf8")
            .and_then(|s| HeaterState::from_str(s).context("payload is not a valid state"))?;

        Ok((heater_id.to_string(), state))
    }
}

/// An executor to handle the events being received and update the state.
#[derive(Debug)]
pub struct Executor {
    state: State,
    mqtt_client: AsyncClient,
    rx: Receiver<Action>,
    db: Arc<Mutex<Database>>,
}

impl Executor {
    pub fn new(mqtt_client: AsyncClient, db: Arc<Mutex<Database>>, rx: Receiver<Action>) -> Self {
        Self {
            state: State::default(),
            mqtt_client,
            db,
            rx,
        }
    }

    /// Run the executor until completion.
    pub async fn run_until_completion(mut self) -> Result<()> {
        while let Some(action) = self.rx.recv().await {
            tracing::debug!("Received action: {action:?}");
            if let Err(e) = self.handle_action(&action).await {
                tracing::error!(error = %e, action = ?action, "Failed to handle action");
            }
        }

        Ok(())
    }

    /// Handle an action received through the subscribed channel.
    #[tracing::instrument(skip(self))]
    async fn handle_action(&mut self, action: &Action) -> Result<()> {
        use Action::*;
        match action {
            SetDesiredTemperature(temp) => {
                self.state.desired_temperature = Some(*temp);
                self.check_temperature().await?;
            }
            SetInsideTemperature(temp) => {
                self.state.current_temperature = Some(*temp);
                self.check_temperature().await?;
            }
            EnableController(enabled) => {
                tracing::info!(state = ?self.state, "Temperature control enabled: {enabled}");
                self.state.enabled = *enabled;
                self.check_temperature().await?;
            }
            RegisterMeasurement(place, measurement) => {
                self.db
                    .lock()
                    .await
                    .insert_reading(
                        place,
                        *measurement.temperature() as i64,
                        *measurement.humidity() as i64,
                    )
                    .await?;
            }
            RegisterHeaterStateChange(heater_id, state) => {
                self.db
                    .lock()
                    .await
                    .insert_heater_state(heater_id, *state)
                    .await?;
            }
        }

        Ok(())
    }

    /// Set the heaters to either on or off.
    #[tracing::instrument(skip(self))]
    async fn set_heaters_state(&self, state: HeaterState) -> Result<()> {
        for heater in HEATERS.iter() {
            self.mqtt_client
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

    /// Check the current temperature against the desired temperature and
    /// update the heaters as needed.
    #[tracing::instrument(skip(self), fields(state = ?self.state))]
    async fn check_temperature(&self) -> Result<()> {
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
