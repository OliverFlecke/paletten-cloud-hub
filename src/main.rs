use std::{
    fmt::{Debug, Display},
    sync::Arc,
};

use tokio::{sync::Mutex, task::JoinError};

use crate::controller::Controller;

mod controller;
mod db;
pub mod models;
mod telemetry;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let subscriber =
        telemetry::create_minimal_subscriber("paletten_cloud_hub".to_string(), std::io::stdout);
    telemetry::init_subscriber(subscriber);

    tracing::info!("Starting hub");

    let db_connection_string = "sqlite:data/paletten.sqlite";
    let db_pool = db::create_db_pool(db_connection_string).await?;
    let database = Arc::new(Mutex::new(db::Database::new(db_pool).await?));

    let controller = Controller::new(controller::create_mqtt_handler(), database);

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
