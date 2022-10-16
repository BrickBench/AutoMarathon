use std::{fmt::Display, time::Duration};

use crate::{user::Player, state::Project};


#[derive(PartialEq, Debug)]
pub enum Command<'a> {
    Toggle(&'a Player),
    Swap(&'a Player, &'a Player),
    SetPlayers(Vec<&'a Player>),
    SetScore(&'a Player, Duration),
    Refresh(Vec<&'a Player>)
}

#[derive(Debug)]
pub enum CommandError{
    InvalidPlayer(String),
    InvalidCommand(String),
    UnknownCommand(String),
    InvalidTime(String)
}

impl Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
       match self {
            CommandError::InvalidPlayer(player) => write!(f, "Unknown player \"{}\"", player),
            CommandError::UnknownCommand(cmd) => write!(f, "Unknown command \"{}\"", cmd),
            CommandError::InvalidCommand(cmd) => write!(f, "Invalid command \"{}\"", cmd),
            CommandError::InvalidTime(cmd) => write!(f, "Invalid time \"{}\"", cmd)
        } 
    }
}

fn find_or_err<'a>(user: &'_ str, project: &'a Project) -> Result<&'a Player, CommandError> {
    project.find_by_nick(user).ok_or(CommandError::InvalidPlayer(user.to_owned()))
}

pub fn parse_cmds<'a>(full_cmd: &'_ str, project: &'a Project) -> Result<Vec<Command<'a>>, CommandError> {
    full_cmd.split("\n")
        .filter(|c| !c.is_empty())
        .map(|c| parse_cmd(c, project))
        .collect::<Result<Vec<Command>, CommandError>>()
}

pub fn parse_cmd<'a>(full_cmd: &'_ str, project: &'a Project) -> Result<Command<'a>, CommandError> {
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
                        Err(CommandError::InvalidCommand(full_cmd.to_owned()))
                    } else {
                        Ok(Command::Swap(p1, p2))
                    }
                }
                "set" => {
                    args_iter
                        .map(|e| find_or_err(e, project))
                        .into_iter()
                        .collect::<Result<Vec<&Player>, CommandError>>()
                        .map(|ps| Command::SetPlayers(ps))
                }
                "record" => {
                    let player = find_or_err(args[0], project)?;
                    let duration = Duration::from_millis(
                        args[1].parse::<u64>().map_err(|e| CommandError::InvalidTime(args[1].to_owned()))?);

                    Ok(Command::SetScore(player, duration))
                }
                "refresh" if args.len() != 0 => {
                    args_iter
                        .map(|e| find_or_err(e, project))
                        .into_iter()
                        .collect::<Result<Vec<&Player>, CommandError>>()
                        .map(|ps| Command::Refresh(ps))
                }
                "refresh" if args.len() == 0  => {
                    Ok(Command::Refresh(project.players.iter().collect()))
                }
                _ => Err(CommandError::UnknownCommand(cmd.to_owned()))
            }
        } 
        None => Err(CommandError::InvalidCommand(full_cmd.to_owned())),
    }
}

