use chrono::NaiveDateTime;
use openwhoop::db::ActivityHeartRateStats;
use openwhoop::types::activities::{ActivityPeriod, ActivityType};
use openwhoop_entities::activities;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use strum::IntoEnumIterator;
use tauri::AppHandle;

use crate::{
    error::{AppError, AppResult},
    handlers::log_error,
    state::DatabaseState,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityTypeOption {
    value: String,
    label: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnfinishedActivitySummary {
    period_id: chrono::NaiveDate,
    start: chrono::NaiveDateTime,
    activity: String,
    strain: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityHeartRateStatsSummary {
    min_hr: u8,
    max_hr: u8,
    avg_hr: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnfinishedActivityMetricsSummary {
    activity: Option<UnfinishedActivitySummary>,
    heart_rate_stats: Option<ActivityHeartRateStatsSummary>,
}

#[tauri::command]
// TODO: see if there is better way to do this
pub async fn get_activity_types(app: AppHandle) -> AppResult<Vec<ActivityTypeOption>> {
    activity_type_options().map_err(|err| {
        log_error(
            &app,
            "activity_types",
            format!("Unable to enumerate activity types: {:?}", err),
        );
        err
    })
}

#[tauri::command]
pub async fn get_unfinished_activity(
    app: AppHandle,
    database_state: tauri::State<'_, DatabaseState>,
) -> AppResult<Option<UnfinishedActivitySummary>> {
    database_state
        .database()
        .get_unfinished_activity()
        .await
        .map(|activity| activity.map(map_unfinished_activity))
        .map_err(|err| {
            let err = AppError::from(err);
            log_error(
                &app,
                "activity_types",
                format!("Unable to load unfinished activity: {:?}", err),
            );
            err
        })
}

#[tauri::command]
pub async fn refresh_unfinished_activity_metrics(
    app: AppHandle,
    database_state: tauri::State<'_, DatabaseState>,
) -> AppResult<UnfinishedActivityMetricsSummary> {
    let database = database_state.database();
    let activity = database.get_unfinished_activity().await.map_err(|err| {
        let err = AppError::from(err);
        log_error(
            &app,
            "activity_types",
            format!("Unable to reload unfinished activity metrics: {:?}", err),
        );
        err
    })?;

    let heart_rate_stats = match activity.clone() {
        Some(activity_period) => database
            .get_heart_rate_stats_for_activity(activity_period)
            .await
            .map_err(|err| {
                let err = AppError::from(err);
                log_error(
                    &app,
                    "activity_types",
                    format!(
                        "Unable to load heart-rate stats for unfinished activity: {:?}",
                        err
                    ),
                );
                err
            })?
            .map(map_activity_heart_rate_stats),
        None => None,
    };

    Ok(UnfinishedActivityMetricsSummary {
        activity: activity.map(map_unfinished_activity),
        heart_rate_stats,
    })
}

#[tauri::command]
pub async fn update_activity(
    app: AppHandle,
    database_state: tauri::State<'_, DatabaseState>,
    original_start: String,
    next_start: String,
    next_end: String,
    activity_type: Option<String>,
) -> AppResult<()> {
    update_activity_by_start(
        database_state.inner(),
        &original_start,
        &next_start,
        &next_end,
        activity_type.as_deref(),
    )
    .await
    .map_err(|err| {
        log_error(
            &app,
            "activity_types",
            format!(
                "Unable to update activity original_start={} next_start={} next_end={}: {:?}",
                original_start, next_start, next_end, err
            ),
        );
        err
    })
}

#[tauri::command]
pub async fn create_activity(
    app: AppHandle,
    database_state: tauri::State<'_, DatabaseState>,
    from: String,
    to: String,
    activity_type: Option<String>,
) -> AppResult<()> {
    create_activity_period(database_state.inner(), &from, &to, activity_type.as_deref())
        .await
        .map_err(|err| {
            log_error(
                &app,
                "activity_types",
                format!(
                    "Unable to create activity from={} to={} activity_type={:?}: {:?}",
                    from, to, activity_type, err
                ),
            );
            err
        })
}

#[tauri::command]
pub async fn start_activity(
    app: AppHandle,
    database_state: tauri::State<'_, DatabaseState>,
    from: String,
    activity_type: Option<String>,
) -> AppResult<bool> {
    start_activity_period(database_state.inner(), &from, activity_type.as_deref())
        .await
        .map_err(|err| {
            log_error(
                &app,
                "activity_types",
                format!(
                    "Unable to start activity from={} activity_type={:?}: {:?}",
                    from, activity_type, err
                ),
            );
            err
        })
}

#[tauri::command]
pub async fn delete_activity(
    app: AppHandle,
    database_state: tauri::State<'_, DatabaseState>,
    original_start: String,
) -> AppResult<()> {
    delete_activity_by_start(database_state.inner(), &original_start)
        .await
        .map_err(|err| {
            log_error(
                &app,
                "activity_types",
                format!(
                    "Unable to delete activity original_start={}: {:?}",
                    original_start, err
                ),
            );
            err
        })
}

fn activity_type_options() -> AppResult<Vec<ActivityTypeOption>> {
    Ok(ActivityType::iter()
        .map(|activity_type| {
            let label = activity_type.to_string();
            ActivityTypeOption {
                value: label.clone(),
                label,
            }
        })
        .collect())
}

fn map_unfinished_activity(activity: ActivityPeriod) -> UnfinishedActivitySummary {
    UnfinishedActivitySummary {
        period_id: activity.period_id,
        start: activity.from,
        activity: activity.activity.to_string(),
        strain: activity.strain,
    }
}

fn map_activity_heart_rate_stats(stats: ActivityHeartRateStats) -> ActivityHeartRateStatsSummary {
    ActivityHeartRateStatsSummary {
        min_hr: stats.min_hr,
        max_hr: stats.max_hr,
        avg_hr: stats.avg_hr,
    }
}

async fn update_activity_by_start(
    database_state: &DatabaseState,
    original_start: &str,
    next_start: &str,
    next_end: &str,
    activity_type: Option<&str>,
) -> AppResult<()> {
    let database = database_state.database();
    let original_start = parse_datetime(original_start)?;
    let next_start = parse_datetime(next_start)?;
    let next_end = parse_datetime(next_end)?;

    if next_end <= next_start {
        return Err(crate::error::AppError::from(
            "Activity end time must be after the start time.",
        ));
    }

    let activity_type = activity_type
        .unwrap_or("Activity")
        .parse::<ActivityType>()
        .map_err(|_| AppError::from("invalid activity"))?;

    let activity = activities::Entity::find()
        .filter(activities::Column::Start.eq(original_start))
        .one(database.connection())
        .await?
        .ok_or_else(|| {
            crate::error::AppError::from(format!("Activity not found for {original_start}"))
        })?;

    let mut activity_model: activities::ActiveModel = activity.into();
    activity_model.start = Set(next_start);
    activity_model.end = Set(Some(next_end));
    activity_model.activity = Set(activity_type.to_string());
    activity_model.synced = Set(false);
    activity_model.update(database.connection()).await?;

    Ok(())
}

async fn create_activity_period(
    database_state: &DatabaseState,
    from: &str,
    to: &str,
    activity_type: Option<&str>,
) -> AppResult<()> {
    let database = database_state.database();
    let from = parse_datetime(from)?;
    let to = parse_datetime(to)?;

    if to <= from {
        return Err(crate::error::AppError::from(
            "Activity end time must be after the start time.",
        ));
    }

    let activity = activity_type
        .unwrap_or("Activity")
        .parse::<ActivityType>()
        .map_err(|_| AppError::from("invalid activity"))?;

    database
        .create_activity(ActivityPeriod {
            period_id: from.date(),
            from,
            to: Some(to),
            activity,
            strain: None,
        })
        .await
        .map_err(AppError::from)?;

    Ok(())
}

async fn start_activity_period(
    database_state: &DatabaseState,
    from: &str,
    activity_type: Option<&str>,
) -> AppResult<bool> {
    let database = database_state.database();

    if database
        .get_unfinished_activity()
        .await
        .map_err(AppError::from)?
        .is_some()
    {
        return Ok(false);
    }

    let from = parse_datetime(from)?;
    let period_id = database
        .get_latest_sleep()
        .await
        .map_err(AppError::from)?
        .map(|sleep| sleep.id)
        .ok_or_else(|| AppError::from("Cannot start activity before any sleep has been synced."))?;
    let activity = activity_type
        .unwrap_or("Activity")
        .parse::<ActivityType>()
        .map_err(|_| AppError::from("invalid activity"))?;

    database
        .create_activity(ActivityPeriod {
            period_id,
            from,
            to: None,
            activity,
            strain: None,
        })
        .await
        .map_err(AppError::from)?;

    Ok(true)
}

async fn delete_activity_by_start(
    database_state: &DatabaseState,
    original_start: &str,
) -> AppResult<()> {
    let database = database_state.database();
    let original_start = parse_datetime(original_start)?;

    let activity = activities::Entity::find()
        .filter(activities::Column::Start.eq(original_start))
        .one(database.connection())
        .await?
        .ok_or_else(|| {
            crate::error::AppError::from(format!("Activity not found for {original_start}"))
        })?;

    let activity_model: activities::ActiveModel = activity.into();
    activity_model.delete(database.connection()).await?;
    Ok(())
}

fn parse_datetime(value: &str) -> AppResult<NaiveDateTime> {
    for format in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
    ] {
        if let Ok(parsed) = NaiveDateTime::parse_from_str(value, format) {
            return Ok(parsed);
        }
    }

    Err(crate::error::AppError::from(format!(
        "Invalid activity start timestamp '{value}'"
    )))
}
