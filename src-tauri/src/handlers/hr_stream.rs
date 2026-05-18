use chrono::Local;
use openwhoop_codec::{constants::WhoopGeneration, HistoryReading, WhoopData, WhoopPacket};
use openwhoop_entities::activities;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use tauri::{AppHandle, Emitter, Manager};

use crate::{
    error::{AppError, AppResult},
    handlers::{
        log_error, log_info, stress_stream::stop_stress_stream_internal,
        whoop_manager::read_persisted_whoop_store,
    },
    internals::{ensure_connected_saved_whoop, frame_whoop_command},
    now_unix_ms,
    state::DatabaseState,
    AppState,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeartRateStreamStatus {
    pub running: bool,
    pub generation: Option<WhoopGeneration>,
    pub last_sample_at_ms: Option<u64>,
    pub last_bpm: Option<u8>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct HeartRateSample {
    unix: u32,
    bpm: u8,
    received_at_ms: u64,
}

pub struct HeartRateStreamController {
    running: bool,
    generation: Option<WhoopGeneration>,
    next_seq: u8,
    last_sample_at_ms: Option<u64>,
    last_bpm: Option<u8>,
    last_error: Option<String>,
    last_activity_strain_refresh_minute: Option<u64>,
}

impl Default for HeartRateStreamController {
    fn default() -> Self {
        Self {
            running: false,
            // device_address: None,
            generation: None,
            next_seq: 0,
            last_sample_at_ms: None,
            last_bpm: None,
            last_error: None,
            last_activity_strain_refresh_minute: None,
        }
    }
}

impl HeartRateStreamController {
    pub fn is_running(&self) -> bool {
        self.running
    }
}

#[tauri::command]
pub fn get_heart_rate_stream_status(
    state: tauri::State<'_, AppState>,
) -> AppResult<HeartRateStreamStatus> {
    heart_rate_stream_status_snapshot(state.inner())
}

#[tauri::command]
pub async fn start_heart_rate_stream(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> AppResult<HeartRateStreamStatus> {
    let store = read_persisted_whoop_store(&app)?;
    let Some((address, generation)) = store.whoop_and_generation() else {
        return Err(AppError::from("Whoop device not selected"));
    };

    log_info(
        &app,
        "heart_rate_stream",
        format!(
            "Start heart-rate stream requested address={} generation={}",
            address, generation
        ),
    );

    stop_stress_stream_internal(&app, state.inner()).await?;
    stop_heart_rate_stream_internal(&app, state.inner()).await?;
    ensure_connected_saved_whoop(state.inner(), &address).await?;

    let handler = tauri_plugin_blec::get_handler()?;
    let callback_app = app.clone();
    if let Err(error) = handler
        .subscribe(
            generation.data_from_strap(),
            Some(generation.service()),
            move |bytes: Vec<u8>| handle_heart_rate_notification(&callback_app, generation, bytes),
        )
        .await
        .map_err(|err| err.to_string())
    {
        log_error(
            &app,
            "heart_rate_stream",
            format!("Unable to subscribe to WHOOP HR data characteristic: {error}"),
        );
        return Err(AppError::from(error));
    }

    let callback_app = app.clone();
    if let Err(error) = handler
        .subscribe(
            generation.cmd_from_strap(),
            Some(generation.service()),
            move |bytes: Vec<u8>| handle_heart_rate_notification(&callback_app, generation, bytes),
        )
        .await
        .map_err(|err| err.to_string())
    {
        let _ = handler.unsubscribe(generation.data_from_strap()).await;
        log_error(
            &app,
            "heart_rate_stream",
            format!("Unable to subscribe to WHOOP HR command characteristic: {error}"),
        );
        return Err(AppError::from(error));
    }

    state.inner().update_heart_rate_stream(|controller| {
        controller.running = true;
        controller.generation = Some(generation);
        controller.next_seq = 0;
        controller.last_sample_at_ms = None;
        controller.last_bpm = None;
        controller.last_error = None;
        controller.last_activity_strain_refresh_minute = None;
    })?;

    if let Err(error) = send_connected_whoop_command(
        state.inner(),
        generation,
        WhoopPacket::toggle_realtime_hr(true),
    )
    .await
    {
        let _ = handler.unsubscribe(generation.data_from_strap()).await;
        let _ = handler.unsubscribe(generation.cmd_from_strap()).await;

        state.inner().update_heart_rate_stream(|controller| {
            controller.running = false;
            controller.generation = None;
            controller.next_seq = 0;
            controller.last_error = Some(error.clone());
            controller.last_activity_strain_refresh_minute = None;
        })?;

        return Err(AppError::from(error));
    }

    log_info(
        &app,
        "heart_rate_stream",
        "Heart-rate stream started successfully.",
    );
    heart_rate_stream_status_snapshot(state.inner())
}

fn next_heart_rate_stream_seq(state: &AppState) -> AppResult<u8> {
    let seq = state.get_heart_rate_stream(|s| s.next_seq)?;
    state.update_heart_rate_stream(|c| c.next_seq = seq.wrapping_add(1))?;
    Ok(seq)
}

pub fn heart_rate_stream_status_snapshot(state: &AppState) -> AppResult<HeartRateStreamStatus> {
    state.get_heart_rate_stream(|controller| HeartRateStreamStatus {
        running: controller.running,
        generation: controller.generation,
        last_sample_at_ms: controller.last_sample_at_ms,
        last_bpm: controller.last_bpm,
        last_error: controller.last_error.clone(),
    })
}

pub(crate) async fn stop_heart_rate_stream_internal(
    app: &AppHandle,
    state: &AppState,
) -> AppResult<HeartRateStreamStatus> {
    let (running, generation) = state.get_heart_rate_stream(|c| (c.running, c.generation))?;

    if !running {
        return heart_rate_stream_status_snapshot(state);
    }

    log_info(
        app,
        "heart_rate_stream",
        "Stopping the live heart-rate stream.",
    );

    if let Some(generation) = generation {
        if let Ok(handler) = tauri_plugin_blec::get_handler() {
            if handler.is_connected() {
                if let Err(error) = send_connected_whoop_command(
                    state,
                    generation,
                    WhoopPacket::toggle_realtime_hr(false),
                )
                .await
                {
                    let _ = state.update_heart_rate_stream(move |controller| {
                        controller.last_error = Some(error);
                        controller.last_activity_strain_refresh_minute = None;
                    });
                }
            }

            let _ = handler.unsubscribe(generation.data_from_strap()).await;
            let _ = handler.unsubscribe(generation.cmd_from_strap()).await;
        }

        state.update_heart_rate_stream(|controller| {
            controller.running = false;
            controller.generation = None;
            controller.next_seq = 0;
            controller.last_sample_at_ms = None;
            controller.last_bpm = None;
            controller.last_activity_strain_refresh_minute = None;
        })?;
    }

    heart_rate_stream_status_snapshot(state)
}

async fn send_connected_whoop_command(
    state: &AppState,
    generation: WhoopGeneration,
    packet: WhoopPacket,
) -> Result<(), String> {
    let handler = tauri_plugin_blec::get_handler().map_err(|err| err.to_string())?;
    let seq = next_heart_rate_stream_seq(state)?;
    let data = frame_whoop_command(packet.with_seq(seq), generation)?;

    handler
        .send_data(
            generation.cmd_to_strap(),
            Some(generation.service()),
            &data,
            tauri_plugin_blec::models::WriteType::WithoutResponse,
        )
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn stop_heart_rate_stream(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> AppResult<HeartRateStreamStatus> {
    stop_heart_rate_stream_internal(&app, state.inner())
        .await
        .map(|status| {
            log_info(
                &app,
                "heart_rate_stream",
                "Heart-rate stream stop request completed.",
            );
            status
        })
        .map_err(|err| {
            log_error(
                &app,
                "heart_rate_stream",
                format!("Unable to stop the heart-rate stream: {:?}", err),
            );
            err
        })
}

fn handle_heart_rate_notification(app: &AppHandle, generation: WhoopGeneration, bytes: Vec<u8>) {
    const HEART_RATE_STREAM_EVENT: &str = "heart-rate-stream-sample";

    let packet = match generation {
        WhoopGeneration::Gen4 => WhoopPacket::from_data(bytes),
        WhoopGeneration::Gen5 => WhoopPacket::from_data_maverick(bytes),
        WhoopGeneration::Placeholder => return,
    };

    let decoded = match packet {
        Ok(packet) => match generation {
            WhoopGeneration::Gen4 => WhoopData::from_packet_gen4(packet),
            WhoopGeneration::Gen5 => WhoopData::from_packet_gen5(packet),
            WhoopGeneration::Placeholder => return,
        },
        Err(_) => return,
    };

    let WhoopData::RealtimeHr { unix, bpm } = (match decoded {
        Ok(decoded) => decoded,
        Err(_) => return,
    }) else {
        return;
    };

    let sample = HeartRateSample {
        unix,
        bpm,
        received_at_ms: now_unix_ms(),
    };
    let sample_at_ms = u64::from(unix).saturating_mul(1000);
    let app_state = app.state::<AppState>();
    let _ = app_state.inner().update_heart_rate_stream(|controller| {
        controller.last_sample_at_ms = Some(sample_at_ms);
        controller.last_bpm = Some(bpm);
        controller.last_error = None;
    });
    let _ = app.emit(HEART_RATE_STREAM_EVENT, sample);

    let persist_app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = persist_realtime_heart_rate_sample(&persist_app, unix, bpm).await {
            log_error(
                &persist_app,
                "heart_rate_stream",
                format!("Unable to persist realtime heart-rate sample: {error}"),
            );
        }
    });
}

async fn persist_realtime_heart_rate_sample(
    app: &AppHandle,
    unix: u32,
    bpm: u8,
) -> Result<(), String> {
    let database = app.state::<DatabaseState>().inner().database();
    database
        .create_reading(HistoryReading {
            unix: u64::from(unix).saturating_mul(1000),
            bpm,
            rr: Vec::new(),
            imu_data: Vec::new(),
            sensor_data: None,
        })
        .await
        .map_err(|err| err.to_string())?;

    maybe_refresh_unfinished_activity_strain(app, unix).await
}

async fn maybe_refresh_unfinished_activity_strain(
    app: &AppHandle,
    unix: u32,
) -> Result<(), String> {
    let minute_bucket = u64::from(unix) / 60;
    let app_state = app.state::<AppState>();
    let should_refresh = app_state
        .inner()
        .get_heart_rate_stream(|controller| {
            controller
                .last_activity_strain_refresh_minute
                .is_none_or(|last_minute| minute_bucket > last_minute)
        })
        .map_err(String::from)?;

    if !should_refresh {
        return Ok(());
    }

    let database = app.state::<DatabaseState>().inner().database();
    let Some(unfinished_activity) = database
        .get_unfinished_activity()
        .await
        .map_err(|err| err.to_string())?
    else {
        app_state
            .inner()
            .update_heart_rate_stream(|controller| {
                controller.last_activity_strain_refresh_minute = Some(minute_bucket);
            })
            .map_err(String::from)?;
        return Ok(());
    };

    let simulated_activity = openwhoop::types::activities::ActivityPeriod {
        to: Some(Local::now().naive_local()),
        ..unfinished_activity.clone()
    };

    let Some(strain) = database
        .calculate_strain_for_activity(simulated_activity)
        .await
        .map_err(|err| err.to_string())?
    else {
        app_state
            .inner()
            .update_heart_rate_stream(|controller| {
                controller.last_activity_strain_refresh_minute = Some(minute_bucket);
            })
            .map_err(String::from)?;
        return Ok(());
    };

    let activity = activities::Entity::find()
        .filter(activities::Column::Start.eq(unfinished_activity.from))
        .one(database.connection())
        .await
        .map_err(|err| err.to_string())?
        .ok_or_else(|| "Unfinished activity missing during realtime strain update".to_owned())?;

    let mut activity_model: activities::ActiveModel = activity.into();
    activity_model.strain = Set(Some(strain));
    activity_model.synced = Set(false);
    activity_model
        .update(database.connection())
        .await
        .map_err(|err| err.to_string())?;

    app_state
        .inner()
        .update_heart_rate_stream(|controller| {
            controller.last_activity_strain_refresh_minute = Some(minute_bucket);
        })
        .map_err(|err| format!("{:?}", err))?;

    Ok(())
}
