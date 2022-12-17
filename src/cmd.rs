use std::time::Duration;

use crate::{user::Player, state::{Project, ProjectState}, obs::{Layout, ObsConfiguration}, error::Error};


#[derive(PartialEq, Debug)]
pub enum Command<'a> {
    Toggle(&'a Player),
    Swap(&'a Player, &'a Player),
    SetPlayers(Vec<&'a Player>),
    SetScore(&'a Player, Duration),
    Refresh(Vec<&'a Player>),
    Layout(u32, &'a Layout)
}

fn find_or_err<'a>(user: &'_ str, project: &'a Project) -> Result<&'a Player, Error> {
    project.find_by_nick(user).ok_or(Error::PlayerError(user.to_owned()))
}

pub fn parse_cmds<'a>(full_cmd: &'_ str, project: &'a Project, obs: &'a ObsConfiguration, state: &'_ ProjectState) -> Result<Vec<Command<'a>>, Error> {
    full_cmd.split("\n")
        .filter(|c| !c.is_empty())
        .map(|c| parse_cmd(c, project, obs, state))
        .collect::<Result<Vec<Command>, Error>>()
}

pub fn parse_cmd<'a>(full_cmd: &'_ str, project: &'a Project, obs: &'a ObsConfiguration, state: &'_ ProjectState) -> Result<Command<'a>, Error> {
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

                    Ok(Command::SetScore(player, duration))
                }
                "refresh" if args.len() != 0 => {
                    args_iter
                        .map(|e| find_or_err(e, project))
                        .into_iter()
                        .collect::<Result<Vec<&Player>, Error>>()
                        .map(|ps| Command::Refresh(ps))
                }
                "refresh" if args.len() == 0  => {
                    Ok(Command::Refresh(project.players.iter().collect()))
                }
                "layout" if args.len() == 1 => {
                    obs.layouts.get(args[0])
                        .map(|l| Command::Layout(state.active_players.len().try_into().unwrap(), l))
                        .ok_or(Error::LayoutError(args[0].to_owned()))
                }
                "layout" if args.len() == 2 => {
                    let idx = args[0].parse::<u32>()
                        .map_err(|_| Error::CommandError(full_cmd.to_string()))?;

                    obs.layouts.get(args[1])
                        .map(|l| Command::Layout(idx, l))
                        .ok_or(Error::LayoutError(args[0].to_owned()))
                }
                _ => Err(Error::CommandError(cmd.to_owned()))
            }
        } 
        None => Err(Error::CommandError(full_cmd.to_owned())),
    }
}

