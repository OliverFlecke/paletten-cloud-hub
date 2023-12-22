use tracing::{subscriber::set_global_default, Level, Subscriber};
use tracing_bunyan_formatter::{BunyanFormattingLayer, JsonStorageLayer};
use tracing_subscriber::{
    filter::Targets, fmt::MakeWriter, prelude::__tracing_subscriber_SubscriberExt,
    registry::LookupSpan, Registry,
};

/// Setup telemetry and output it to a given sink.
pub fn create_minimal_subscriber<Sink>(
    name: String,
    sink: Sink,
) -> impl Subscriber + Send + Sync + for<'span> LookupSpan<'span>
where
    Sink: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    let filter = Targets::new()
        .with_target(&name, Level::DEBUG)
        .with_default(Level::WARN);

    let formatting_layer = BunyanFormattingLayer::new(name, sink);

    Registry::default()
        .with(filter)
        .with(JsonStorageLayer)
        .with(formatting_layer)
}

pub fn init_subscriber(subscriber: impl Subscriber + Send + Sync) {
    set_global_default(subscriber).expect("Failed to setup log subscriber");
}
