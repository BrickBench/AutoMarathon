use anyhow::anyhow;
use std::{collections::HashMap, path::Path};

use sqlx::{
    migrate::MigrateDatabase, query, sqlite::Sqlite, types::time, QueryBuilder, SqlitePool,
};

use crate::{
    core::{event::Event, runner::Runner, stream::StreamState},
    integrations::therun::Run,
    web::streams::WebCommand,
    Directory,
};

use super::{event::RunnerEventState, participant::Participant};

pub struct ProjectDb {
    db: SqlitePool,
    directory: Directory,
}

#[derive(sqlx::FromRow)]
struct RunnerStreamUrl {
    resolution: String,
    url: String,
}

impl ProjectDb {
    pub async fn load(file: &Path, directory: Directory) -> anyhow::Result<Self> {
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
            "create table participants(
                        id integer primary key not null,
                        name text unique not null collate nocase,
                        location text,
                        pronouns text,
                        discord_id text,
                        photo blob,
                        host boolean not null
                    );"
        )
        .execute(&self.db)
        .await?;

        query!(
            "create table runners(
                        participant integer primary key not null,
                        stream text, 
                        therun text,
                        override_stream_url text,
                        stream_volume_percent integer not null,
                        foreign key(participant) references participants(id) on delete cascade
                    );"
        )
        .execute(&self.db)
        .await?;

        query!(
            "
            create table runner_stream_urls(
                runner integer not null,
                resolution text not null,
                url text not null,
                foreign key(runner) references runners(participant) on delete cascade
                );
            "
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
                    foreign key(runner) references runners(participant) on delete cascade
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
                best_possible real,
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
            "create table events(
                    id integer primary key not null,
                    name text unique not null collate nocase,
                    tournament integer,
                    game text,
                    category text,
                    console text,
                    complete boolean,
                    estimate integer,
                    therun_race_id text,
                    event_start_time integer,
                    timer_start_time integer,
                    timer_end_time integer,
                    preferred_layouts json not null,
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
                    foreign key(runner) references runners(participant) on delete cascade
                );"
        )
        .execute(&self.db)
        .await?;

        query!(
            "create table commentators_in_event(
                    event integer not null,
                    commentator integer not null, 
                    foreign key(event) references events(id) on delete cascade,
                    foreign key(commentator) references participants(id) on delete cascade
                );"
        )
        .execute(&self.db)
        .await?;

        query!(
            "create table streams(
                    event integer primary key not null unique,
                    obs_host text not null,
                    requested_layout text,
                    audible_runner integer,
                    foreign key(event) references events(id) on delete cascade
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
                    foreign key(runner) references runners(participant) on delete cascade
                );"
        )
        .execute(&self.db)
        .await?;

        query!(
            "create table custom_fields(
                    fkey text primary key not null unique,
                    value text
                );"
        )
        .execute(&self.db)
        .await?;

        query!(
            "create table discord_user_volumes(
                    discord_id text primary key not null unique,
                    volume_percent integer
                );"
        )
        .execute(&self.db)
        .await?;

        Ok(())
    }

    fn trigger_update(&self) {
        self.directory.web_actor.send(WebCommand::TriggerStateUpdate);
    }

    pub async fn add_participant(&self, participant: &mut Participant) -> anyhow::Result<()> {
        log::debug!("Creating new participant {}", participant.name);
        sqlx::query(
            "insert into participants(name, location, pronouns, discord_id, photo, host)
                    values(?, ?, ?, ?, ?, ?)",
        )
        .bind(&participant.name)
        .bind(&participant.location)
        .bind(&participant.pronouns)
        .bind(&participant.discord_id)
        .bind(&participant.photo)
        .bind(participant.host)
        .execute(&self.db)
        .await?;

        let id: i64 = sqlx::query_scalar("select last_insert_rowid()")
            .fetch_one(&self.db)
            .await?;

        participant.id = id;

        self.trigger_update();
        Ok(())
    }

    pub async fn get_participant(&self, id: i64) -> anyhow::Result<Participant> {
        Ok(sqlx::query_as("select * from participants where id = ?")
            .bind(id)
            .fetch_one(&self.db)
            .await?)
    }

    pub async fn update_participant(&self, participant: &Participant) -> anyhow::Result<()> {
        log::debug!("Updating participant {}", participant.name);
        sqlx::query(
            "update participants set
                name = ?,
                location = ?,
                pronouns = ?,
                discord_id = ?,
                photo = ?,
                host = ?
                where id = ?",
        )
        .bind(&participant.name)
        .bind(&participant.location)
        .bind(&participant.pronouns)
        .bind(&participant.discord_id)
        .bind(&participant.photo)
        .bind(participant.host)
        .bind(participant.id)
        .execute(&self.db)
        .await?;

        self.trigger_update();

        Ok(())
    }

    pub async fn get_participants(&self) -> anyhow::Result<Vec<Participant>> {
        Ok(sqlx::query_as("select * from participants")
            .fetch_all(&self.db)
            .await?)
    }

    pub async fn get_participant_by_discord_id(
        &self,
        discord_id: &str,
    ) -> anyhow::Result<Participant> {
        Ok(
            sqlx::query_as("select * from participants where discord_id = ?")
                .bind(discord_id)
                .fetch_one(&self.db)
                .await?,
        )
    }

    pub async fn get_participant_by_name(&self, name: &str) -> anyhow::Result<Participant> {
        Ok(sqlx::query_as("select * from participants where name = ?")
            .bind(name)
            .fetch_one(&self.db)
            .await?)
    }

    pub async fn delete_participant(&self, id: i64) -> anyhow::Result<()> {
        sqlx::query("delete from participants where id = ?")
            .bind(id)
            .execute(&self.db)
            .await?;
        self.trigger_update();
        Ok(())
    }

    pub async fn get_runners(&self) -> anyhow::Result<Vec<Runner>> {
        let mut runners: Vec<Runner> = sqlx::query_as("select * from runners")
            .fetch_all(&self.db)
            .await?;

        for runner in runners.iter_mut() {
            let stream_urls: Vec<RunnerStreamUrl> =
                sqlx::query_as("select resolution, url from runner_stream_urls where runner = ?")
                    .bind(runner.participant)
                    .fetch_all(&self.db)
                    .await?;

            runner.stream_urls = stream_urls
                .into_iter()
                .map(|r| (r.resolution, r.url))
                .collect();
        }

        Ok(runners)
    }

    pub async fn add_runner(&self, runner: &mut Runner) -> anyhow::Result<()> {
        log::debug!("Creating new runner {}", runner.participant);
        let mut tx = self.db.begin().await?;
        sqlx::query(
            "insert into runners(participant, stream, therun, override_stream_url, stream_volume_percent)
                    values(?, ?, ?, ?, ?)",
        )
        .bind(runner.participant)
        .bind(&runner.stream)
        .bind(&runner.therun)
        .bind(&runner.override_stream_url)
        .bind(runner.stream_volume_percent)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        self.trigger_update();
        Ok(())
    }

    pub async fn update_runner(&self, runner: &Runner) -> anyhow::Result<()> {
        log::debug!("Updating runner {}", runner.participant);

        let mut tx = self.db.begin().await?;

        sqlx::query(
            "update runners set
                stream = ?,
                therun = ?,
                override_stream_url = ?,
                stream_volume_percent = ?
                where participant = ?
  ",
        )
        .bind(&runner.stream)
        .bind(&runner.therun)
        .bind(&runner.override_stream_url)
        .bind(runner.stream_volume_percent)
        .bind(runner.participant)
        .execute(&mut *tx)
        .await?;

        sqlx::query("delete from runner_stream_urls where runner = ?")
            .bind(runner.participant)
            .execute(&mut *tx)
            .await?;

        if !runner.stream_urls.is_empty() {
            let mut builder =
                sqlx::QueryBuilder::new("insert into runner_stream_urls(runner, resolution, url)");
            let streams = runner.stream_urls.clone();
            builder.push_values(streams.iter(), |mut b, url_map| {
                b.push_bind(runner.participant)
                    .push_bind(url_map.0)
                    .push_bind(url_map.1);
            });
            builder.build().execute(&mut *tx).await?;
        }

        tx.commit().await?;
        self.trigger_update();

        Ok(())
    }

    pub async fn get_runner(&self, id: i64) -> anyhow::Result<Runner> {
        let mut runner: Runner = sqlx::query_as(
            "select * from runners 
                        where participant = ?
                        limit 1",
        )
        .bind(id)
        .fetch_one(&self.db)
        .await?;

        runner.participant_data = self.get_participant(runner.participant).await?;

        let stream_urls: Vec<RunnerStreamUrl> =
            sqlx::query_as("select resolution, url from runner_stream_urls where runner = ?")
                .bind(id)
                .fetch_all(&self.db)
                .await?;

        runner.stream_urls = stream_urls
            .into_iter()
            .map(|r| (r.resolution, r.url))
            .collect();

        Ok(runner)
    }

    pub async fn delete_runner(&self, runner: i64) -> anyhow::Result<()> {
        sqlx::query("delete from runners where participant = ?")
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
            .execute(&mut *tx)
            .await?;

        let mut builder =
            sqlx::QueryBuilder::new("insert into splits(run, name, pb_split_time, split_time, best_possible)");
        builder.push_values(run.splits.iter(), |mut b, split| {
            b.push_bind(runner)
                .push_bind(&split.name)
                .push_bind(split.pb_split_time)
                .push_bind(split.split_time)
                .push_bind(split.best_possible);
        });

        builder.build().execute(&mut *tx).await?;
        tx.commit().await?;

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
                .push_bind(*result.0)
                .push_bind(result.1.result.clone());
        });
        builder
    }

    fn create_event_commentators_builder(&self, event: &Event) -> QueryBuilder<Sqlite> {
        let mut builder =
            sqlx::QueryBuilder::new("insert into commentators_in_event(event, commentator)");

        builder.push_values(event.commentators.iter(), |mut b, result| {
            b.push_bind(event.id).push_bind(*result);
        });
        builder
    }

    pub async fn add_event(&self, event: &mut Event) -> anyhow::Result<()> {
        let mut tx = self.db.begin().await?;
        sqlx::query(
            "insert into events(name, tournament, game, category, console, complete, estimate, therun_race_id, 
                            event_start_time, is_relay, is_marathon, preferred_layouts) 
                values(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&event.name)
        .bind(event.tournament)
        .bind(&event.game)
        .bind(&event.category)
        .bind(&event.console)
        .bind(event.complete)
        .bind(event.estimate)
        .bind(&event.therun_race_id)
        .bind(event.timer_start_time.map(|t| t.unix_timestamp()))
        .bind(event.is_relay)
        .bind(event.is_marathon)
        .bind(serde_json::to_string(&event.preferred_layouts).unwrap())
        .execute(&mut *tx)
        .await?;

        let last_event_id: i64 = sqlx::query_scalar("select last_insert_rowid()")
            .fetch_one(&mut *tx)
            .await?;
        event.id = last_event_id;
        log::debug!("Event {} assigned ID {}", event.name, event.id);

        if !event.runner_state.is_empty() {
            let mut builder = self.create_event_runners_builder(event);
            builder.build().execute(&mut *tx).await?;
        }

        if !event.commentators.is_empty() {
            let mut builder = self.create_event_commentators_builder(event);
            builder.build().execute(&mut *tx).await?;
        }

        tx.commit().await?;
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

        let runner_state: Vec<RunnerEventState> =
            sqlx::query_as("select * from runners_in_event where event = ?")
                .bind(event_id)
                .fetch_all(&self.db)
                .await?;
        event.runner_state = runner_state.into_iter().map(|r| (r.runner, r)).collect();

        let commentators: Vec<i64> =
            sqlx::query_scalar("select commentator from commentators_in_event where event = ?")
                .bind(event_id)
                .fetch_all(&self.db)
                .await?;
        event.commentators = commentators;

        Ok(event)
    }

    pub async fn update_event_start_time(
        &self,
        event: i64,
        start_time: Option<time::OffsetDateTime>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "update events set
                    timer_start_time = ?
                    where id = ?",
        )
        .bind(start_time.map(|t| t.unix_timestamp()))
        .bind(event)
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
                    timer_end_time = ?
                    where id = ?",
        )
        .bind(end_time.map(|t| t.unix_timestamp()))
        .bind(event)
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
                    game = ?,
                    category = ?,
                    console = ?,
                    complete = ?,
                    estimate = ?,
                    therun_race_id = ?,
                    event_start_time = ?,
                    timer_start_time = ?,
                    timer_end_time = ?,
                    is_relay = ?,
                    is_marathon = ?,
                    preferred_layouts = ?
                    where id = ?",
        )
        .bind(&event.name)
        .bind(event.tournament)
        .bind(&event.game)
        .bind(&event.category)
        .bind(&event.console)
        .bind(event.complete)
        .bind(event.estimate)
        .bind(&event.therun_race_id)
        .bind(event.event_start_time.map(|t| t.unix_timestamp()))
        .bind(event.timer_start_time.map(|t| t.unix_timestamp()))
        .bind(event.timer_end_time.map(|t| t.unix_timestamp()))
        .bind(event.is_relay)
        .bind(event.is_marathon)
        .bind(serde_json::to_string(&event.preferred_layouts).unwrap())
        .bind(event.id)
        .execute(&mut *tx)
        .await?;

        sqlx::query("delete from runners_in_event where event = ?")
            .bind(event.id)
            .execute(&mut *tx)
            .await?;

        if !event.runner_state.is_empty() {
            let mut builder = self.create_event_runners_builder(event);
            builder.build().execute(&mut *tx).await?;
        }

        sqlx::query("delete from commentators_in_event where event = ?")
            .bind(event.id)
            .execute(&mut *tx)
            .await?;

        if !event.commentators.is_empty() {
            let mut builder = self.create_event_commentators_builder(event);
            builder.build().execute(&mut *tx).await?;
        }

        tx.commit().await?;
        self.trigger_update();

        Ok(())
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

    pub async fn get_stream_runners(&self, event: i64) -> anyhow::Result<HashMap<i64, i64>> {
        let query: Vec<(i64, i64)> = sqlx::query_as(
            "select stream_order, runner from runners_in_stream where event = ? order by stream_order",
        ).bind(event).fetch_all(&self.db).await?;

        Ok(query.into_iter().collect())
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
                        event, obs_host, 
                        requested_layout,
                        audible_runner
                    ) values(?, ?, ?, ?)",
        )
        .bind(state.event)
        .bind(&state.obs_host)
        .bind(&state.requested_layout)
        .bind(state.audible_runner)
        .execute(&mut *tx)
        .await?;

        sqlx::query("delete from runners_in_stream where event = ?")
            .bind(state.event)
            .execute(&mut *tx)
            .await?;

        if !state.stream_runners.is_empty() {
            let mut builder = sqlx::QueryBuilder::new(
                "insert into runners_in_stream(event, runner, stream_order)",
            );
            builder.push_values(
                state.stream_runners.iter().enumerate(),
                |mut b, (_, runner)| {
                    b.push_bind(state.event)
                        .push_bind(runner.1)
                        .push_bind(runner.0);
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

    pub async fn get_custom_fields(&self) -> anyhow::Result<HashMap<String, Option<String>>> {
        Ok(sqlx::query_as("select * from custom_fields")
            .fetch_all(&self.db)
            .await?
            .into_iter()
            .collect())
    }

    pub async fn add_custom_field(&self, key: &str, value: Option<&str>) -> anyhow::Result<()> {
        sqlx::query("insert or replace into custom_fields(fkey, value) values(?, ?)")
            .bind(key)
            .bind(value)
            .execute(&self.db)
            .await?;
        self.trigger_update();
        Ok(())
    }

    pub async fn get_custom_field(&self, key: &str) -> anyhow::Result<Option<String>> {
        Ok(
            sqlx::query_scalar("select value from custom_fields where fkey = ?")
                .bind(key)
                .fetch_optional(&self.db)
                .await?,
        )
    }

    pub async fn clear_custom_field(&self, key: &str) -> anyhow::Result<()> {
        sqlx::query("delete from custom_fields where fkey = ?")
            .bind(key)
            .execute(&self.db)
            .await?;
        self.trigger_update();
        Ok(())
    }

    pub async fn set_discord_user_volume(
        &self,
        discord_id: &str,
        volume_percent: i32,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "insert or replace into discord_user_volumes(discord_id, volume_percent) values(?, ?)",
        )
        .bind(discord_id)
        .bind(volume_percent)
        .execute(&self.db)
        .await?;
        self.trigger_update();
        Ok(())
    }

    pub async fn get_discord_user_volume(&self, discord_id: &str) -> anyhow::Result<Option<i32>> {
        Ok(sqlx::query_scalar(
            "select volume_percent from discord_user_volumes where discord_id = ?",
        )
        .bind(discord_id)
        .fetch_optional(&self.db)
        .await?)
    }
}
