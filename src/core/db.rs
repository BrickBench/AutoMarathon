use anyhow::anyhow;
use std::{i64, path::PathBuf};

use sqlx::{
    migrate::MigrateDatabase, query, sqlite::Sqlite, types::time, QueryBuilder, SqlitePool,
};

use crate::{
    core::{event::Event, runner::Runner, stream::StreamState},
    integrations::{obs::ObsLayout, therun::Run, web::WebCommand},
    Directory,
};

pub struct ProjectDb {
    db: SqlitePool,
    directory: Directory,
}

impl ProjectDb {
    pub async fn load(file: &PathBuf, directory: Directory) -> anyhow::Result<Self> {
        let url = format!("sqlite://{}", file.to_str().unwrap());
        Sqlite::create_database(&url).await?;

        let db = SqlitePool::connect(&url).await?;
        let proj = Self { db, directory };

        let table_exists =
            query!("SELECT name FROM sqlite_master WHERE type='table' AND name='runners'")
                .fetch_optional(&proj.db)
                .await?
                .is_some();

        if !table_exists {
            log::info!("Tables are not present, initalizing database...");
            proj.create_tables().await?;
        }

        Ok(proj)
    }

    pub async fn create_tables(&self) -> anyhow::Result<()> {
        query!(
            "create table runners(
                        id integer primary key not null,
                        name text unique not null collate nocase,
                        stream text, 
                        therun text,
                        cached_stream_url text,
                        location text,
                        photo blob,
                        volume_percent integer not null
                    );"
        )
        .execute(&self.db)
        .await?;

        query!(
            "create table runs(
                    runner integer primary key not null,
                    sob real,
                    best_possible real,
                    delta real,
                    started_at text,
                    current_comparison text,
                    pb real,
                    current_split_name text,
                    current_split_index integer,
                    foreign key(runner) references runners(id) on delete cascade
            );"
        )
        .execute(&self.db)
        .await?;

        query!(
            "create table splits(
                run integer not null,
                name text not null,
                pb_split_time real,
                split_time real,
                foreign key(run) references runs(runner) on delete cascade
            );"
        )
        .execute(&self.db)
        .await?;

        query!(
            "create table tournaments(
                        id integer primary key not null,
                        name text not null collate nocase,
                        format text not null
            );"
        )
        .execute(&self.db)
        .await?;

        query!(
            "create table layouts(
                        name text primary key not null collate nocase,
                        runner_count integer not null,
                        default_layout boolean not null
            );"
        )
        .execute(&self.db)
        .await?;

        query!(
            "create table nicknames(
                        nickname text primary key not null collate nocase,
                        runner integer not null, 
                        foreign key(runner) references runners(id) on delete cascade
            );"
        )
        .execute(&self.db)
        .await?;

        query!(
            "create table events(
                    id integer primary key not null,
                    name text unique not null collate nocase,
                    tournament integer,
                    therun_race_id text,
                    start_time integer,
                    end_time integer,
                    is_relay boolean not null, 
                    is_marathon boolean not null,
                    foreign key(tournament) references tournaments(id) on delete set null
                );"
        )
        .execute(&self.db)
        .await?;

        query!(
            "create table runners_in_event(
                    event integer not null,
                    runner integer not null, 
                    result json,
                    foreign key(event) references events(id) on delete cascade,
                    foreign key(runner) references runners(id) on delete cascade
                );"
        )
        .execute(&self.db)
        .await?;

        query!(
            "create table streams(
                    event integer primary key not null unique,
                    obs_host text not null,
                    active_commentators text not null,
                    ignored_commentators text not null,
                    requested_layout text,
                    audible_runner text,
                    foreign key(event) references events(id) on delete cascade
                    foreign key(requested_layout) references layouts(name) on delete cascade
                );"
        )
        .execute(&self.db)
        .await?;

        query!(
            "create table runners_in_stream(
                    event integer not null,
                    runner integer not null,
                    stream_order integer not null,
                    foreign key(event) references streams(event) on delete cascade,
                    foreign key(runner) references runners(id) on delete cascade
                );"
        )
        .execute(&self.db)
        .await?;

        Ok(())
    }

    fn trigger_update(&self) {
        self.directory.web_actor.send(WebCommand::SendStateUpdate);
    }

    pub async fn get_runners(&self) -> anyhow::Result<Vec<Runner>> {
        Ok(sqlx::query_as("select * from runners")
            .fetch_all(&self.db)
            .await?)
    }

    pub async fn add_runner(&self, runner: &mut Runner) -> anyhow::Result<()> {
        log::debug!("Creating new runner {}", runner.name);
        let mut tx = self.db.begin().await?;
        sqlx::query("insert into runners(name, stream, therun, location, volume_percent) values(?, ?, ?, ?, ?)")
            .bind(&runner.name)
            .bind(&runner.stream)
            .bind(&runner.therun)
            .bind(&runner.location)
            .bind(&runner.volume_percent)
            .execute(&mut *tx)
            .await?;

        let last_event_id: i64 = sqlx::query_scalar("select last_insert_rowid()")
            .fetch_one(&self.db)
            .await?;
        runner.id = last_event_id;

        if !runner.nicks.is_empty() {
            let mut builder = sqlx::QueryBuilder::new("insert into nicknames(nickname, runner)");
            builder.push_values(runner.nicks.iter(), |mut b, nick| {
                b.push_bind(nick).push_bind(runner.name.clone());
            });
            builder.build().execute(&mut *tx).await?;
        }
        tx.commit().await?;
        self.trigger_update();
        Ok(())
    }

    pub async fn update_runner(&self, runner: &Runner) -> anyhow::Result<()> {
        log::debug!("Updating runner {}", runner.name);

        let mut tx = self.db.begin().await?;

        sqlx::query(
            "update runners set
                    name = ?,
                    stream = ?,
                    therun = ?,
                    cached_stream_url = ?,
                    location = ?,
                    volume_percent = ?,
                    where id = ?",
        )
        .bind(&runner.name)
        .bind(&runner.stream)
        .bind(&runner.therun)
        .bind(&runner.cached_stream_url)
        .bind(&runner.location)
        .bind(&runner.volume_percent)
        .bind(&runner.id)
        .execute(&mut *tx)
        .await?;

        sqlx::query("delete from nicknames where runner = ?")
            .bind(&runner.id)
            .execute(&mut *tx)
            .await?;

        let mut builder = sqlx::QueryBuilder::new("insert into nicknames(nickname, runner)");
        builder.push_values(runner.nicks.iter(), |mut b, nick| {
            b.push_bind(nick).push_bind(runner.id);
        });
        builder.build().execute(&mut *tx).await?;

        tx.commit().await?;
        self.trigger_update();

        Ok(())
    }

    pub async fn get_runner(&self, id: i64) -> anyhow::Result<Runner> {
        let mut runner: Runner = sqlx::query_as(
            "select * from runners 
                        where id = ?
                        limit 1",
            /*"select * from runners r
            left join nicknames n on r.name = n.runner
            where r.name = '?' or n.nickname = '?'
            limit 1",*/
        )
        .bind(id)
        .fetch_one(&self.db)
        .await?;

        runner.nicks = sqlx::query_scalar("select nickname from nicknames where runner = ?")
            .bind(&runner.id)
            .fetch_all(&self.db)
            .await?;

        Ok(runner)
    }

    pub async fn get_name_for_runner(&self, id: i64) -> anyhow::Result<String> {
        Ok(sqlx::query_scalar("select name from runners where id = ?")
            .bind(id)
            .fetch_one(&self.db)
            .await?)
    }

    pub async fn find_runner(&self, name: &str) -> anyhow::Result<Runner> {
        log::debug!("Searching for runner {}", name);
        let mut runner: Runner = 
        /*sqlx::query_as(
            "select * from runners r
            left join nicknames n on r.name = n.runner
            where r.name = '?' or n.nickname = '?'
            limit 1",
        )*/
        sqlx::query_as("select * from runners where name = ?")
        .bind(name)
        .fetch_optional(&self.db)
        .await?
        .ok_or(anyhow!("Failed to find runner {}", name))?;

        runner.nicks = sqlx::query_scalar("select nickname from nicknames where runner = ?")
            .bind(&runner.id)
            .fetch_all(&self.db)
            .await?;

        Ok(runner)
    }

    pub async fn delete_runner(&self, runner: i64) -> anyhow::Result<()> {
        sqlx::query("delete from runners where id= ?")
            .bind(runner)
            .execute(&self.db)
            .await?;
        self.trigger_update();
        Ok(())
    }

    pub async fn clear_runner_run(&self, runner: i64) -> anyhow::Result<()> {
        sqlx::query("delete from runs where runner = ?")
            .bind(runner)
            .execute(&self.db)
            .await?;
        self.trigger_update();
        Ok(())
    }

    pub async fn set_runner_run_data(&self, runner: i64, run: &Run) -> anyhow::Result<()> {
        let mut tx = self.db.begin().await?;
        sqlx::query(
            "insert or replace into runs(
                runner, sob, best_possible, delta, 
                started_at, current_comparison, pb, 
                current_split_name, current_split_index)
                    values(?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(runner)
        .bind(run.sob)
        .bind(run.best_possible)
        .bind(run.delta)
        .bind(&run.started_at)
        .bind(&run.current_comparison)
        .bind(run.pb)
        .bind(&run.current_split_name)
        .bind(run.current_split_index)
        .execute(&mut *tx)
        .await?;

        sqlx::query("delete from splits where run = ?")
            .bind(runner)
            .execute(&self.db)
            .await?;

        let mut builder =
            sqlx::QueryBuilder::new("insert into splits(run, name, pb_split_time, split_time)");
        builder.push_values(run.splits.iter(), |mut b, split| {
            b.push_bind(runner)
                .push_bind(&split.name)
                .push_bind(split.pb_split_time)
                .push_bind(split.split_time);
        });

        builder.build().execute(&mut *tx).await?;
        tx.commit().await?;
        self.trigger_update();

        Ok(())
    }

    pub async fn get_runner_run_data(&self, runner: i64) -> anyhow::Result<Run> {
        let mut run: Run = sqlx::query_as("select * from runs where runner = ?")
            .bind(runner)
            .fetch_one(&self.db)
            .await?;

        run.splits = sqlx::query_as("select * from splits where run = ?")
            .bind(runner)
            .fetch_all(&self.db)
            .await?;
        Ok(run)
    }

    fn create_event_runners_builder(&self, event: &Event) -> QueryBuilder<Sqlite> {
        let mut builder =
            sqlx::QueryBuilder::new("insert into runners_in_event(event, runner, result)");

        builder.push_values(event.runner_state.iter(), |mut b, result| {
            b.push_bind(event.id)
                .push_bind(result.runner)
                .push_bind(result.result.clone());
        });
        builder
    }

    pub async fn add_event(&self, event: &mut Event) -> anyhow::Result<()> {
        sqlx::query(
            "insert into events(name, tournament, therun_race_id, is_relay, is_marathon) values(?, ?, ?, ?, ?)",
        )
        .bind(&event.name)
        .bind(&event.tournament)
        .bind(&event.therun_race_id)
        .bind(event.is_relay)
        .bind(event.is_marathon)
        .execute(&self.db)
        .await?;

        let last_event_id: i64 = sqlx::query_scalar("select last_insert_rowid()")
            .fetch_one(&self.db)
            .await?;
        event.id = last_event_id;

        if !event.runner_state.is_empty() {
            let mut builder = self.create_event_runners_builder(&event);
            builder.build().execute(&self.db).await?;
        }

        self.trigger_update();
        Ok(())
    }

    pub async fn get_event_ids(&self) -> anyhow::Result<Vec<i64>> {
        Ok(sqlx::query_scalar("select id from events")
            .fetch_all(&self.db)
            .await?)
    }

    pub async fn get_id_for_event(&self, name: &str) -> anyhow::Result<i64> {
        Ok(sqlx::query_scalar("select id from events where name = ?")
            .bind(name)
            .fetch_one(&self.db)
            .await?)
    }

    pub async fn get_event_names(&self) -> anyhow::Result<Vec<String>> {
        Ok(sqlx::query_scalar("select name from events")
            .fetch_all(&self.db)
            .await?)
    }

    pub async fn get_event(&self, event_id: i64) -> anyhow::Result<Event> {
        let mut event: Event = sqlx::query_as("select * from events where id = ? limit 1")
            .bind(event_id)
            .fetch_one(&self.db)
            .await?;

        event.runner_state = sqlx::query_as("select * from runners_in_event where event = ?")
            .bind(event_id)
            .fetch_all(&self.db)
            .await?;

        Ok(event)
    }

    pub async fn update_event_start_time(
        &self,
        event: i64,
        start_time: Option<time::OffsetDateTime>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "update events set
                    start_time = ?,
                    where id = ?",
        )
        .bind(start_time.map(|t| t.unix_timestamp()))
        .bind(&event)
        .execute(&self.db)
        .await?;

        self.trigger_update();
        Ok(())
    }

    pub async fn update_event_end_time(
        &self,
        event: i64,
        end_time: Option<time::OffsetDateTime>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "update events set
                    end_time = ?,
                    where id = ?",
        )
        .bind(end_time.map(|t| t.unix_timestamp()))
        .bind(&event)
        .execute(&self.db)
        .await?;

        self.trigger_update();
        Ok(())
    }

    pub async fn update_event(&self, event: &Event) -> anyhow::Result<()> {
        let mut tx = self.db.begin().await?;
        sqlx::query(
            "update events set
                    name = ?,
                    tournament = ?,
                    therun_race_id = ?,
                    start_time = ?,
                    end_time = ?,
                    is_relay = ?,
                    is_marathon = ?
                    where id = ?",
        )
        .bind(&event.name)
        .bind(&event.tournament)
        .bind(&event.therun_race_id)
        .bind(event.start_time.map(|t| t.unix_timestamp()))
        .bind(event.end_time.map(|t| t.unix_timestamp()))
        .bind(event.is_relay)
        .bind(event.is_marathon)
        .bind(event.id)
        .execute(&mut *tx)
        .await?;

        sqlx::query("delete from runners_in_event where event = ?")
            .bind(&event.id)
            .execute(&mut *tx)
            .await?;

        let mut builder = self.create_event_runners_builder(&event);
        builder.build().execute(&mut *tx).await?;
        tx.commit().await?;
        self.trigger_update();

        Ok(())
    }

    pub async fn get_runners_for_event(&self, event_id: i64) -> anyhow::Result<Vec<Runner>> {
        Ok(sqlx::query_as(
            "select * from runners
                        inner join runners_in_event on runners.id = runners_in_event.runner
                        where event = ?",
        )
        .bind(event_id)
        .fetch_all(&self.db)
        .await?)
    }

    pub async fn get_events_for_runner(&self, runner: i64) -> anyhow::Result<Vec<String>> {
        Ok(sqlx::query_scalar(
            "select e.name from events e
                        inner join runners_in_event r on e.id = r.event
                        where r.runner = ?",
        )
        .bind(runner)
        .fetch_all(&self.db)
        .await?)
    }

    pub async fn add_runner_to_event(&self, event_id: i64, runner: i64) -> anyhow::Result<()> {
        sqlx::query("insert into runners_in_event(event, runner) values(?, ?)")
            .bind(event_id)
            .bind(runner)
            .execute(&self.db)
            .await?;
        self.trigger_update();
        Ok(())
    }

    pub async fn delete_runner_from_event(&self, event_id: i64, runner: i64) -> anyhow::Result<()> {
        sqlx::query("delete from runners_in_event where event = ? and runner = ?")
            .bind(event_id)
            .bind(runner)
            .execute(&self.db)
            .await?;

        self.trigger_update();
        Ok(())
    }

    pub async fn delete_event(&self, event_id: i64) -> anyhow::Result<()> {
        sqlx::query("delete from events where id = ?")
            .bind(event_id)
            .execute(&self.db)
            .await?;

        self.trigger_update();
        Ok(())
    }

    pub async fn get_stream_count(&self) -> anyhow::Result<u32> {
        Ok(sqlx::query_scalar("select count(*) from streams")
            .fetch_one(&self.db)
            .await?)
    }

    pub async fn get_streamed_events(&self) -> anyhow::Result<Vec<i64>> {
        Ok(sqlx::query_scalar("select event from streams")
            .fetch_all(&self.db)
            .await?)
    }

    pub async fn get_streamed_event_names(&self) -> anyhow::Result<Vec<String>> {
        Ok(sqlx::query_scalar(
            "select e.name from events e
                                    inner join streams s on e.id = s.event",
        )
        .fetch_all(&self.db)
        .await?)
    }

    pub async fn get_stream_runners(&self, event: i64) -> anyhow::Result<Vec<i64>> {
        Ok(sqlx::query_scalar(
            "select runner from runners_in_stream where event = ? order by stream_order",
        )
        .bind(event)
        .fetch_all(&self.db)
        .await?)
    }

    pub async fn get_stream(&self, event_id: i64) -> anyhow::Result<StreamState> {
        let mut state: StreamState =
            sqlx::query_as("select * from streams where event = ? limit 1")
                .bind(event_id)
                .fetch_one(&self.db)
                .await?;

        state.stream_runners = self.get_stream_runners(event_id).await?;

        Ok(state)
    }

    pub async fn save_stream(&self, state: &StreamState) -> anyhow::Result<()> {
        let mut tx = self.db.begin().await?;
        sqlx::query(
            "insert or replace into streams(
                        event, obs_host, active_commentators,
                        ignored_commentators, requested_layout,
                        audible_runner
                    ) values(?, ?, ?, ?, ?, ?)",
        )
        .bind(&state.event)
        .bind(&state.obs_host)
        .bind(&state.active_commentators)
        .bind(&state.ignored_commentators)
        .bind(&state.requested_layout)
        .bind(&state.audible_runner)
        .execute(&mut *tx)
        .await?;

        sqlx::query("delete from runners_in_stream where event = ?")
            .bind(&state.event)
            .execute(&mut *tx)
            .await?;

        if !state.stream_runners.is_empty() {
            let mut builder = sqlx::QueryBuilder::new(
                "insert into runners_in_stream(event, runner, stream_order)",
            );
            builder.push_values(
                state.stream_runners.iter().enumerate(),
                |mut b, (i, runner)| {
                    b.push_bind(&state.event)
                        .push_bind(runner)
                        .push_bind(i as i32);
                },
            );
            builder.build().execute(&mut *tx).await?;
        }
        tx.commit().await?;
        self.trigger_update();
        Ok(())
    }

    pub async fn is_host_in_use(&self, obs_host: &str) -> anyhow::Result<bool> {
        Ok(sqlx::query_scalar(
            "select count(*) from streams 
                                    where obs_host = ?",
        )
        .bind(obs_host)
        .fetch_one(&self.db)
        .await?)
    }

    pub async fn get_event_by_obs_host(&self, obs_host: &str) -> anyhow::Result<i64> {
        sqlx::query_scalar(
            "select event from streams 
                                    where obs_host = ?",
        )
        .bind(obs_host)
        .fetch_optional(&self.db)
        .await?
        .ok_or(anyhow!("Failed to find event for host {}", obs_host))
    }

    pub async fn delete_stream(&self, event_id: i64) -> anyhow::Result<()> {
        sqlx::query("delete from streams where event = ?")
            .bind(event_id)
            .execute(&self.db)
            .await?;

        self.trigger_update();
        Ok(())
    }

    pub async fn create_layout(&self, layout: &ObsLayout) -> anyhow::Result<()> {
        sqlx::query("insert into layouts(name, runner_count, default_layout) values(?, ?, ?)")
            .bind(&layout.name)
            .bind(layout.runner_count)
            .bind(layout.default_layout)
            .execute(&self.db)
            .await?;

        self.trigger_update();
        Ok(())
    }

    pub async fn get_layouts(&self) -> anyhow::Result<Vec<ObsLayout>> {
        Ok(sqlx::query_as("select * from layouts")
            .fetch_all(&self.db)
            .await?)
    }

    pub async fn delete_layout(&self, layout: &str) -> anyhow::Result<()> {
        sqlx::query("delete from layouts where name = ?")
            .bind(layout)
            .execute(&self.db)
            .await?;

        self.trigger_update();
        Ok(())
    }
}
