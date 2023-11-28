use std::collections::HashMap;

use crate::{
    error::Error,
    obs::{Layout, LayoutFile},
    player::Player,
    state::Project,
};

/// String-based verson of Command, used for argument checking before command parsing
#[derive(PartialEq, Debug)]
pub enum CommandSource {
    Toggle(String),
    Swap(String, String),
    SetPlayers(Vec<String>),
    SetPlayerFields(String, HashMap<String, Option<String>>),
    SetEventFields(HashMap<String, Option<String>>),
    Refresh(Vec<String>),
    Layout(String),
    Commentary(Vec<String>),
    CommentaryIgnore(Vec<String>),
    GetState,
}

/// A command that modifies the project state
#[derive(PartialEq, Debug)]
pub enum Command<'a> {
    Toggle(&'a Player),
    Swap(&'a Player, &'a Player),
    SetPlayers(Vec<&'a Player>),
    SetPlayerFields(&'a Player, HashMap<String, Option<String>>),
    SetEventFields(HashMap<String, Option<String>>),
    Refresh(Vec<&'a Player>),
    Layout(u32, &'a Layout),
    SetCommentaryIgnore(Vec<String>),
    SetCommentary(Vec<String>),
    GetState,
}


pub fn parse_cmd<'a>(
    cmd: &CommandSource,
    project: &'a Project,
    layouts: &'a LayoutFile,
) -> Result<Command<'a>, Error> {
    match cmd {
        CommandSource::Toggle(runner) => project.find_player(runner).map(|u| Command::Toggle(u)),
        CommandSource::Swap(r1, r2)=> {
            let p1 = project.find_player(&r1)?;
            let p2 = project.find_player(&r2)?;

            if p1 == p2 {
                Err(Error::CommandError(
                    "Cannot swap same player to itself.".to_string(),
                ))
            } else {
                Ok(Command::Swap(p1, p2))
            }
        },
        CommandSource::SetPlayers(players) => players
            .into_iter()
            .map(|e| project.find_player(&e))
            .collect::<Result<Vec<&Player>, Error>>()
            .map(|ps| Command::SetPlayers(ps)),
        CommandSource::SetPlayerFields(player, fields) => {
            let player = project.find_player(player)?;

            Ok(Command::SetPlayerFields(player, fields.clone()))
        }
        CommandSource::Refresh(users) if users.len() != 0 => users.into_iter() 
            .map(|e| project.find_player(e))
            .collect::<Result<Vec<&Player>, Error>>()
            .map(|ps| Command::Refresh(ps)),
        CommandSource::Refresh(users) if users.len() == 0 => Ok(Command::Refresh(project.players.iter().collect())),
        CommandSource::Layout(layout) => layouts
            .layouts
            .iter()
            .find(|l| &l.name == layout)
            .map(|l| Command::Layout(l.players as u32, l))
            .ok_or(Error::LayoutError(layout.to_string())),
        CommandSource::GetState => Ok(Command::GetState),
        CommandSource::Commentary(commentary) => Ok(Command::SetCommentary(
            commentary.to_vec()
        )),
        CommandSource::CommentaryIgnore(commentary_ignore) => Ok(Command::SetCommentaryIgnore(
            commentary_ignore.to_vec()
        )),
        _ => Err(Error::CommandError(format!("Unknown command {:?}", cmd))),
    }
}
