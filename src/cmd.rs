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
    SetPlayerField(String, String, Option<String>),
    SetPlayerFields(String, HashMap<String, String>),
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
    SetPlayerField(&'a Player, String, Option<String>),
    SetPlayerFields(&'a Player, HashMap<String, String>),
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
        CommandSource::Toggle(runner) => project.find_or_err(runner).map(|u| Command::Toggle(u)),
        CommandSource::Swap(r1, r2)=> {
            let p1 = project.find_or_err(&r1)?;
            let p2 = project.find_or_err(&r2)?;

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
            .map(|e| project.find_or_err(&e))
            .collect::<Result<Vec<&Player>, Error>>()
            .map(|ps| Command::SetPlayers(ps)),
        CommandSource::SetPlayerField(runner, field, opt) => {
            let player = project.find_or_err(runner)?;

            Ok(Command::SetPlayerField(
                player,
                field.to_string(),
                opt.as_ref().map(|s| s.to_string()),
            ))
        }
        CommandSource::SetPlayerFields(player, fields) => {
            let player = project.find_or_err(player)?;

            Ok(Command::SetPlayerFields(player, fields.clone()))
        }
        CommandSource::Refresh(users) if users.len() != 0 => users.into_iter() 
            .map(|e| project.find_or_err(e))
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
