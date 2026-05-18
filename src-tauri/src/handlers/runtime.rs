use openwhoop::{ble::tauri_blec::TauriBlecTransport, db::DatabaseHandler, WhoopDeviceWith};
use openwhoop_codec::constants::WhoopGeneration;
use tauri::AppHandle;

use crate::{
    config::normalize_whoop_address,
    error::AppResult,
    handlers::{
        daily_info::{daily_info_snapshot, DailyInfoSummary},
        heart_rate_stream_status_snapshot,
        latest_reading::latest_reading_label_snapshot,
        log_error,
        sync::{background_sync_status_snapshot, BackgroundSyncStatus},
        HeartRateStreamStatus,
    },
    state::DatabaseState,
    AppState,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedWhoopRuntimeStatus {
    selected_whoop_address: Option<String>,
    connected_device_address: Option<String>,
    connected: bool,
    battery_level: Option<u8>,
    latest_reading_label: Option<String>,
    daily_info: DailyInfoSummary,
    background_sync: BackgroundSyncStatus,
    heart_rate_stream: HeartRateStreamStatus,
}

#[tauri::command]
pub async fn get_saved_whoop_runtime_status(
    app: AppHandle,
    database_state: tauri::State<'_, DatabaseState>,
    state: tauri::State<'_, AppState>,
) -> AppResult<SavedWhoopRuntimeStatus> {
    saved_whoop_runtime_status_snapshot(database_state.inner(), state.inner())
        .await
        .map_err(|err| {
            log_error(
                &app,
                "runtime_status",
                format!("Unable to build saved WHOOP runtime status: {:?}", err),
            );
            err
        })
}

pub async fn saved_whoop_runtime_status_snapshot(
    database_state: &DatabaseState,
    state: &AppState,
) -> AppResult<SavedWhoopRuntimeStatus> {
    let selected_whoop_address = state.get_whoop_address()?;

    let connected_device_address = if let Ok(handler) = tauri_plugin_blec::get_handler() {
        if handler.is_connected() {
            Some(
                handler
                    .connected_device()
                    .await
                    .map_err(|err| err.to_string())
                    .and_then(|device| normalize_whoop_address(&device.address))?,
            )
        } else {
            None
        }
    } else {
        None
    };

    let connected = selected_whoop_address
        .as_deref()
        .zip(connected_device_address.as_deref())
        .is_some_and(|(selected_address, connected_address)| {
            selected_address.eq_ignore_ascii_case(connected_address)
        });

    let battery_level = if connected {
        read_connected_whoop_battery_level().await.ok().flatten()
    } else {
        None
    };

    let latest_reading_label = latest_reading_label_snapshot(database_state).await?;
    let daily_info = daily_info_snapshot(database_state, None).await?;

    Ok(SavedWhoopRuntimeStatus {
        selected_whoop_address,
        connected_device_address,
        connected,
        battery_level,
        latest_reading_label,
        daily_info,
        background_sync: background_sync_status_snapshot(state)?,
        heart_rate_stream: heart_rate_stream_status_snapshot(state)?,
    })
}

async fn read_connected_whoop_battery_level() -> AppResult<Option<u8>> {
    let handler = tauri_plugin_blec::get_handler().map_err(|err| err.to_string())?;
    if !handler.is_connected() {
        return Ok(None);
    }

    let connected_device = handler
        .connected_device()
        .await
        .map_err(|err| err.to_string())?;

    let generation = connected_device
        .services
        .iter()
        .find_map(|service| WhoopGeneration::from_service(*service))
        .unwrap_or(WhoopGeneration::Placeholder);

    if matches!(generation, WhoopGeneration::Placeholder) {
        return Ok(None);
    }

    let transport = TauriBlecTransport::connected(handler);
    let database = DatabaseHandler::new("sqlite::memory:").await;
    let mut whoop = WhoopDeviceWith::from_transport(transport, database, false, generation);

    whoop.connect().await.map_err(|err| err.to_string())?;
    whoop
        .get_battery_level()
        .await
        .map(Some)
        .map_err(|err| err.to_string().into())
}
