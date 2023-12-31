use std::{
    fmt::{Debug, Display},
    sync::Arc,
};

use tokio::{sync::Mutex, task::JoinError};

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

    let database = {
        let db_connection_string = "sqlite:data/paletten.sqlite";
        let db_pool = db::create_db_pool(db_connection_string).await?;
        Arc::new(Mutex::new(db::Database::new(db_pool).await?))
    };

    let (mqtt_client, mqtt_eventloop) = controller::create_mqtt_handler();
    let (controller, executor) = controller::create(mqtt_client, mqtt_eventloop, database).await?;

    let controller_task = tokio::spawn(controller.run_until_completion());
    let executor_task = tokio::spawn(executor.run_until_completion());
    let signal_task = tokio::signal::ctrl_c();

    tokio::select! {
        result = controller_task => report_exit("controller", result),
        result = executor_task => report_exit("executor", result),
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
