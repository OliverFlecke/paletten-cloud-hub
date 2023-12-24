CREATE TABLE IF NOT EXISTS history (
    timestamp DATETIME NOT NULL,
    location TEXT NOT NULL,
    temperature TINYINT NOT NULL,
    humidity TINYINT NOT NULL
);
CREATE INDEX IF NOT EXISTS history_timestamp_index ON history (timestamp);

CREATE TABLE IF NOT EXISTS heater_history (
    timestamp DATETIME NOT NULL,
    shelly_id TEXT NOT NULL,
    is_active BOOLEAN NOT NULL
);
CREATE INDEX IF NOT EXISTS heater_history_timestamp_index ON heater_history (
    timestamp
);
