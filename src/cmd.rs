use std::{time::Duration, collections::HashMap};

use crate::{player::Player, state::{Project, ProjectState}, obs::{Layout, LayoutFile}, error::Error};


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
    GetState
}

fn find_or_err<'a>(user: &'_ str, project: &'a Project) -> Result<&'a Player, Error> {
    project.find_by_nick(user).ok_or(Error::PlayerError(user.to_owned()))
}

pub fn parse_cmds<'a>(full_cmd: &'_ str, project: &'a Project, layouts: &'a LayoutFile, state: &'_ ProjectState::<'a>) -> Result<Vec<Command<'a>>, Error> {
    full_cmd.split("\n")
        .filter(|c| !c.is_empty())
        .map(|c| parse_cmd(c, project, layouts, state))
        .collect::<Result<Vec<Command>, Error>>()
}

pub fn parse_cmd<'a>(full_cmd: &'_ str, project: &'a Project, layouts: &'a LayoutFile, state: &'_ ProjectState::<'a>) -> Result<Command<'a>, Error> {
    let mut elements = full_cmd.split(" ");
    match elements.next() {
        Some(cmd) => {
            let args: _ = elements.collect::<Vec<&str>>();
            let args_iter = args.iter();
            match cmd.to_lowercase().replace("!", "").as_str() {
                "toggle" if args.len() == 1 => {
                    find_or_err(args[0], project).map(|u| Command::Toggle(u))
                }
                "swap" if args.len() == 2 => {
                    let p1 = find_or_err(args[0], project)?;
                    let p2 = find_or_err(args[1], project)?;
                    
                    if p1 == p2 {
                        Err(Error::CommandError("Cannot swap same player to itself.".to_string()))
                    } else {
                        Ok(Command::Swap(p1, p2))
                    }
                }
                "set" => {
                    args_iter
                        .map(|e| find_or_err(e, project))
                        .into_iter()
                        .collect::<Result<Vec<&Player>, Error>>()
                        .map(|ps| Command::SetPlayers(ps))
                }
                "record" => {
                    let player = find_or_err(args[0], project)?;
                    let duration = Duration::from_millis(
                        args[1].parse::<u64>().map_err(|_| Error::ParseError(args[1].to_owned()))?);

                    Ok(Command::SetPlayerField(player, "event_pb".to_string(), Some(duration.as_millis().to_string())))
                }
                "set_field" => {
                    let player = find_or_err(args[0], project)?;

                    Ok(Command::SetPlayerField(player, args[1].to_string(), args.get(2).map(|s| s.to_string())))
                }
                "set_fields" => {
                    let player = find_or_err(args[0], project)?;
                    let fields_str = full_cmd.replacen(cmd, "", 1).replacen(args[0], "", 1);
                    let fields = serde_json::from_str::<HashMap<String, String>>(&fields_str)?;

                    Ok(Command::SetPlayerFields(player, fields))
                }
                "refresh" if args.len() != 0 => {
                    args_iter
                        .map(|e| find_or_err(e, project))
                        .into_iter()
                        .collect::<Result<Vec<&Player>, Error>>()
                        .map(|ps| Command::Refresh(ps))
                }
                "refresh" if args.len() == 0  => {
                    Ok(Command::Refresh(state.active_players.clone()))
                }
                "layout" if args.len() == 1 => {
                    layouts.layouts.iter().find(|l| l.name == args[0])
                        .map(|l| Command::Layout(state.active_players.len().try_into().unwrap(), l))
                        .ok_or(Error::LayoutError(args[0].to_owned()))
                }
                "layout" if args.len() == 2 => {
                    let idx = args[0].parse::<u32>()
                        .map_err(|_| Error::CommandError(full_cmd.to_string()))?;

                    layouts.layouts.iter().find(|l| l.name == args[1])
                        .map(|l| Command::Layout(idx, l))
                        .ok_or(Error::LayoutError(args[0].to_owned()))
                },
                "get_state" => {
                    Ok(Command::GetState)
                },
                "commentary" => {
                    Ok(Command::SetCommentary(args[1..].iter().map(|a| a.to_string()).collect()))
                }
                "ignore_commentary" => {
                    Ok(Command::SetCommentaryIgnore(args[1..].iter().map(|a| a.to_string()).collect()))
                }
                _ => Err(Error::CommandError("Unknown command ".to_owned() + cmd))
            }
        } 
        None => Err(Error::CommandError(full_cmd.to_owned())),
    }
}

