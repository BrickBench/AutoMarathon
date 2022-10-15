use std::{io, env, path::Path, fs::File, collections::HashMap};

use state::{Project, ProjectType, ProjectState};

mod user;
mod cmd;
mod state;

fn main() -> Result<(), String>{
    let args: Vec<String> = env::args().collect();
    let (project, state) = match args.len() {
        1 | 2 => return Err("Invalid args".to_owned()),
        3 => {
            let proj_path = Path::new(&args[1]);
            let proj_file = match File::open(proj_path) {
                Ok(file) => file,
                Err(why) => return Err(format!("Failed to open project file {}: {}", args[1], why)),
            };

            let state_path = Path::new(&args[2]);
            let state = match File::open(state_path) {
                Ok(file) => {
                    
                },
                Err(why) => {
                    println!("Project state file does not exist, creating...");
                    File::create(state_path).map_err(|err| format!("Failed to create state file {}: {}", args[2], err))?;
                    ProjectState {
                        running: false,
                        active_players: vec!(),
                        streams: HashMap::new() 
                    }
                }
            }

                            
            (Project {
                kind: ProjectType::Marathon, players: vec!()
            }, state)
        }
        _ => return Err("Too many args".to_owned())
    };

    loop {
        let mut cmd = String::new();
        io::stdin().read_line(&mut cmd).unwrap();

        if cmd == "exit" {
            break;
        }
    }

    Ok(())
}


#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::{state::{ProjectState, Project, ProjectType}, user::Player, cmd::{parse_cmd, Command}};

    #[test]
    fn test_nicks() {
        let project = Project {
            kind: ProjectType::Marathon,
            players: vec!(
                Player {name:"Joe".to_owned(),nicks:vec!("Joseph".to_owned(),"John".to_owned()), twitch: "test".to_owned() },
                Player {name:"William".to_owned(),nicks:vec!("Will".to_owned(),"Bill".to_owned()), twitch: "test2".to_owned() },
            )
        };

        assert_eq!(project.find_by_nick("Joe").unwrap().name, "Joe");
        assert_eq!(project.find_by_nick("john").unwrap().name, "Joe");
        assert!(project.find_by_nick("Jow").is_none());
    }

    #[test]
    fn test_cmds() {
        let project = Project {
            kind: ProjectType::Marathon,
            players: vec!(
                Player {name:"Joe".to_owned(),nicks:vec!("Joseph".to_owned(),"John".to_owned()), twitch: "test".to_owned() },
                Player {name:"William".to_owned(),nicks:vec!("Will".to_owned(),"Bill".to_owned()), twitch: "test2".to_owned() },
            )
        };


        assert_eq!(Command::Toggle(&project.players[0]), parse_cmd("!toggle john", &project).unwrap());

        assert_eq!(Command::Swap(&project.players[0], &project.players[1]), parse_cmd("!swap john bill", &project).unwrap());
        assert!(parse_cmd("!swap john sam", &project).is_err());
        assert!(parse_cmd("!swap john will bill", &project).is_err());
        assert!(parse_cmd("!swap bill will", &project).is_err());
       

        assert_eq!(Command::Set(vec!(&project.players[0], &project.players[1])), parse_cmd("!set john bill", &project).unwrap());
        assert!(parse_cmd("!set earl bill", &project).is_err());
        assert!(parse_cmd("!set bill will dill", &project).is_err());
    }

    #[test]
    fn test_cmd_apply() {
        let project = Project {
            kind: ProjectType::Race,
            players: vec!(
                Player {name:"Joe".to_owned(),nicks:vec!("Joseph".to_owned(),"John".to_owned()), twitch: "test".to_owned() },
                Player {name:"William".to_owned(),nicks:vec!("Will".to_owned(),"Bill".to_owned()), twitch: "test2".to_owned() },
            )
        };

        let mut state = ProjectState {
            running: false,
            active_players: vec!(),
            streams: HashMap::new() 
        };

        state = state.apply_cmd(&parse_cmd("!toggle joe", &project).unwrap()).unwrap();
        assert_eq!(state.active_players[0].name, "Joe");

        state = state.apply_cmd(&parse_cmd("!swap joe will", &project).unwrap()).unwrap();
        assert_eq!(state.active_players[0].name, "William");

        state = state.apply_cmd(&parse_cmd("!swap joe will", &project).unwrap()).unwrap();
        assert_eq!(state.active_players[0].name, "Joe");

        state = state.apply_cmd(&parse_cmd("!set joe will", &project).unwrap()).unwrap();
        assert_eq!(state.active_players[0].name, "Joe");
        assert_eq!(state.active_players[1].name, "William");

        state = state.apply_cmd(&parse_cmd("!swap joe will", &project).unwrap()).unwrap();
        assert_eq!(state.active_players[0].name, "William");
        assert_eq!(state.active_players[1].name, "Joe");

        state = state.apply_cmd(&parse_cmd("!toggle joe", &project).unwrap()).unwrap();
        assert_eq!(state.active_players[0].name, "William");

        assert!(state.apply_cmd(&parse_cmd("!refresh", &project).unwrap()).is_err());
    }
}
