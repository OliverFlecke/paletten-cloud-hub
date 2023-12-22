#![feature(let_chains)]

use crate::controller::Controller;

mod controller;
mod telemetry;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let subscriber =
        telemetry::create_minimal_subscriber("paletten_cloud_hub".to_string(), std::io::stdout);
    telemetry::init_subscriber(subscriber);

    tracing::info!("Starting hub");

    let controller = Controller::new().await;
    controller.run_until_completion().await?;

    Ok(())
}
