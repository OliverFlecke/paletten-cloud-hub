use std::fmt::{Debug, Display};

use tokio::task::JoinError;

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
    let controller_task = tokio::spawn(controller.run_until_completion());
    let signal_task = tokio::signal::ctrl_c();

    tokio::select! {
        result = controller_task => report_exit("controller", result),
        result = signal_task => report_exit("closed by user", Ok(result)),
    };

    Ok(())
}

fn report_exit(task_name: &str, outcome: Result<Result<(), impl Debug + Display>, JoinError>) {
    match outcome {
        Ok(Ok(())) => tracing::info!("{} has exited", task_name),
        Ok(Err(e)) => {
            tracing::error!(
                error.cause_chain = ?e,
                error.message = %e,
                "{} failed",
                task_name
            )
        }
        Err(e) => {
            tracing::error!(
                error.cause_chain = ?e,
                error.message = %e,
                "{}' task failed to complete",
                task_name
            )
        }
    }
}
