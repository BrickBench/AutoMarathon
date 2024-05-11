use std::path::PathBuf;

use sqlx::{migrate::MigrateDatabase, query, sqlite::Sqlite, types::chrono, SqlitePool};

use crate::{obs::ObsLayout, runner::Runner, event::Event, stream::StreamState};

pub struct ProjectDb {
    db: SqlitePool,
}

impl ProjectDb {
    pub async fn init(file: &PathBuf) -> anyhow::Result<Self> {
        let url = format!("sqlite://{}", file.to_str().unwrap());
        Sqlite::create_database(&url).await?;

        let db = SqlitePool::connect(&url).await?;
        query!(
            "create table runners(
                        name text primary key not null collate nocase,
                        stream text, 
                        therun text,
                        cached_stream_url text
                    );"
        )
        .execute(&db)
        .await?;

        query!(
            "create table layouts(
                        name text primary key not null collate nocase,
                        runner_count integer not null,
                        default_layout boolean not null
                    );"
        )
        .execute(&db)
        .await?;

        query!(
            "create table nicknames(
                        nickname text primary key not null collate nocase,
                        runner text not null, 
                        foreign key(runner) references runners(name) on delete cascade
            );"
        )
        .execute(&db)
        .await?;

        query!(
            "create table events(
                    name text primary key unique not null collate nocase,
                    therun_race_id text,
                    start_time integer,
                    end_time integer,
                    is_relay boolean not null, 
                    is_marathon boolean not null
                );"
        )
        .execute(&db)
        .await?;

        query!(
            "create table runners_in_event(
                    event text not null,
                    runner text not null, 
                    foreign key(event) references events(name) on delete cascade,
                    foreign key(runner) references runners(name) on delete cascade
                );"
        )
        .execute(&db)
        .await?;

        query!(
            "create table streams(
                    event text primary key not null unique,
                    stream_host text not null,
                    active_commentators text not null,
                    ignored_commentators text not null,
                    requested_layout text,
                    foreign key(event) references events(name) on delete cascade
                    foreign key(requested_layout) references layouts(name) on delete cascade
                );"
        )
        .execute(&db)
        .await?;

        query!(
            "create table runners_in_stream(
                    event text not null,
                    runner string not null,
                    stream_order integer not null,
                    foreign key(event) references streams(event) on delete cascade,
                    foreign key(runner) references runners(name) on delete cascade
                );"
        )
        .execute(&db)
        .await?;

        Ok(ProjectDb { db })
    }

    pub async fn load(file: &PathBuf) -> anyhow::Result<Self> {
        let url = format!("sqlite://{}", file.to_str().unwrap());
        Sqlite::create_database(&url).await?;

        let db = SqlitePool::connect(&url).await?;
        Ok(ProjectDb { db })
    }

    pub async fn get_runners(&self) -> anyhow::Result<Vec<Runner>> {
        Ok(sqlx::query_as("select * from runners")
            .fetch_all(&self.db)
            .await?)
    }

    pub async fn add_runner(&self, runner: &Runner, nicks: &[&str]) -> anyhow::Result<()> {
        let mut tx = self.db.begin().await?;
        sqlx::query("insert into runners(name, stream, therun) values(?, ?, ?)")
            .bind(&runner.name)
            .bind(&runner.stream)
            .bind(&runner.therun)
            .execute(&mut *tx)
            .await?;

        for nick in nicks {
            sqlx::query("insert into nicknames(nickname, runner) values(?, ?)")
                .bind(&nick)
                .bind(&runner.name)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn update_runner_stream_url(&self, runner: &Runner) -> anyhow::Result<()> {
        Ok(sqlx::query("update runners set stream = ? where name = ?")
            .bind(&runner.stream)
            .bind(&runner.name)
            .execute(&self.db)
            .await
            .map(|_| ())?)
    }

    pub async fn find_runner(&self, name: &str) -> anyhow::Result<Runner> {
        Ok(sqlx::query_as(
            "select * from runners
                        left join nicknames on runners.id = nicknames.runner
                        where display_name = ?
                        or nickname = ?",
        )
        .bind(name)
        .bind(name)
        .fetch_one(&self.db)
        .await?)
    }

    pub async fn add_event(&self, event: &Event) -> anyhow::Result<()> {
        Ok(sqlx::query(
            "insert into events(name, therun_race_id, is_relay, is_marathon) values(?, ?, ?, ?)",
        )
        .bind(&event.name)
        .bind(&event.therun_race_id)
        .bind(event.is_relay)
        .bind(event.is_marathon)
        .execute(&self.db)
        .await
        .map(|_| ())?)
    }

    pub async fn get_event_names(&self) -> anyhow::Result<Vec<String>> {
        Ok(sqlx::query_scalar("select name from events")
            .fetch_all(&self.db)
            .await?)
    }

    pub async fn get_event(&self, event_id: &str) -> anyhow::Result<Event> {
        Ok(
            sqlx::query_as("select * from events where name = ? limit 1")
                .bind(event_id)
                .fetch_one(&self.db)
                .await?,
        )
    }

    pub async fn set_event_start_time(
        &self,
        event_id: &str,
        time: Option<chrono::DateTime<chrono::Utc>>,
    ) -> anyhow::Result<()> {
        Ok(
            sqlx::query("update events set start_time = ? where name = ?")
                .bind(time.map(|t| t.timestamp()))
                .bind(event_id)
                .execute(&self.db)
                .await
                .map(|_| ())?,
        )
    }

    pub async fn set_event_end_time(
        &self,
        event_id: &str,
        time: Option<chrono::DateTime<chrono::Utc>>,
    ) -> anyhow::Result<()> {
        Ok(sqlx::query("update events set end_time = ? where name = ?")
            .bind(time.map(|t| t.timestamp()))
            .bind(event_id)
            .execute(&self.db)
            .await
            .map(|_| ())?)
    }

    pub async fn get_runners_for_event(&self, event_id: &str) -> anyhow::Result<Vec<Runner>> {
        Ok(sqlx::query_as(
            "select * from runners
                        left join runners_in_event on runners.id = runners_in_event.runner
                        where event = ?",
        )
        .bind(event_id)
        .fetch_all(&self.db)
        .await?)
    }

    pub async fn add_runner_to_event(&self, event_id: &str, runner: &str) -> anyhow::Result<()> {
        Ok(
            sqlx::query("insert into runners_in_event(event, runner) values(?, ?)")
                .bind(event_id)
                .bind(runner)
                .execute(&self.db)
                .await
                .map(|_| ())?,
        )
    }

    pub async fn remove_stream(&self, event_id: &str) -> anyhow::Result<()> {
        Ok(sqlx::query("delete from streams where event = ?")
            .bind(event_id)
            .execute(&self.db)
            .await
            .map(|_| ())?)
    }

    pub async fn get_stream_count(&self) -> anyhow::Result<u32> {
        Ok(sqlx::query_scalar("select count(*) from streams")
            .fetch_one(&self.db)
            .await?)
    }
    
    pub async fn get_streamed_events(&self) -> anyhow::Result<Vec<String>> {
        Ok(sqlx::query_scalar("select event from streams")
            .fetch_all(&self.db)
            .await?)
    }

    pub async fn save_stream(&self, state: &StreamState) -> anyhow::Result<()> {
        let mut tx = self.db.begin().await?;
        sqlx::query("insert or replace into streams(
                        event, stream_host, active_commentators,
                        ignored_commentators, requested_layout
                    ) values(?, ?, ?)")
            .bind(&state.event)
            .bind(&state.obs_host)
            .bind(&state.active_commentators)
            .bind(&state.ignored_commentators)
            .bind(&state.requested_layout)
            .execute(&mut *tx)
            .await?;

        sqlx::query("delete from runners_in_stream where event = ?")
            .bind(&state.event)
            .execute(&mut *tx)
            .await?;

        for (i, runner) in state.stream_runners.iter().enumerate() {
            sqlx::query(
                "insert into 
                            runners_in_stream(event, runner, stream_order) 
                            values(?, ?, ?)",
            )
            .bind(&state.event)
            .bind(&runner.name)
            .bind(i as i32)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn get_stream(&self, event_id: &str) -> anyhow::Result<StreamState> {
        let mut state: StreamState =
            sqlx::query_as("select * from streams where event = ? limit 1")
                .bind(event_id)
                .fetch_one(&self.db)
                .await?;

        state.stream_runners = sqlx::query_as(
            "select *
                from runners r
                inner join runners_in_stream s on r.id = s.runner
                where s.event = ?
                order by s.stream_order",
        )
        .bind(event_id)
        .fetch_all(&self.db)
        .await?;

        Ok(state)
    }

    pub async fn get_layouts(&self) -> anyhow::Result<Vec<ObsLayout>> {
        Ok(sqlx::query_as("select * from layouts")
            .fetch_all(&self.db)
            .await?)
    }
}
