use sqlx::prelude::FromRow;

#[derive(PartialEq, Eq, Debug, Clone)]
pub enum TournamentFormat {
    Bracket,
    Ladder
}

#[derive(FromRow, PartialEq, Eq, Debug, Clone)]
pub struct Tournament {
    name: String,
    format: TournamentFormat,
}
