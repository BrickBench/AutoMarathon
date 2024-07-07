use serde::{Deserialize, Serialize};
use sqlx::prelude::FromRow;


/// TheRun websocket return type
#[derive(Serialize, Deserialize)]
pub struct TheRunReturnJson {
    pub user: String,
    pub run: Run,
}

/// Data for an active LiveSplit run
#[allow(non_snake_case)]
#[derive(Debug, Serialize, Deserialize, Clone, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub pb: Option<f64>,
    pub sob: Option<f64>,
    pub best_possible: Option<f64>,
    pub delta: Option<f64>,
    pub started_at: String,
    pub current_comparison: String,
    pub current_split_name: String,
    pub current_split_index: i64,

    #[sqlx(skip)]
    pub splits: Vec<Split>,
}

/// A single LiveSplit split
#[allow(non_snake_case)]
#[derive(Debug, Serialize, Deserialize, Clone, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Split {
    pub name: String,
    pub pb_split_time: Option<f64>,
    pub split_time: Option<f64>,
}
