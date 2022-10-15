use std::{io, path::{Path, PathBuf}, fs::{File, read_to_string, self}, collections::HashMap};

use clap::{Parser, Subcommand};
use cmd::parse_cmds;
use state::{Project, ProjectType, ProjectState, ProjectStateSerializer};
use user::Player;

use crate::state::ProjectTypeState;

mod user;
mod cmd;
mod state;

#[derive(Parser, Debug)]
#[command(name = "AutoMarathon")]
#[command(author = "javster101")]
#[command(version = "0.1")]
#[command(about = "An automation tool for speedrunning events.", long_about = None)]
struct Args {
    #[command(subcommand)]
    command: RunType,
}

#[derive(Subcommand, Debug)]
enum RunType {
    /// Create and initialize a new project. 
    /// The output .json file will need to be manually edited to fill in details for players.
    Create {
        #[arg(short = 't', long)]
        project_type: ProjectType,

        /// List of players to initialize.
        #[arg(short = 'p', long, use_value_delimiter = true, value_delimiter = ',')]
        players: Vec<String>,

        project_file: PathBuf,
    },

    /// Run a project file.
    Run {

        /// Location for project state save file.
        #[arg(short, long)]
        save_file: PathBuf,

        project_file: PathBuf,
    }
}

fn main() -> Result<(), String> {

    let args = Args::parse();
    
    match &args.command {
        RunType::Create { project_type, players, project_file } => {
            let new_project = Project {
                kind: project_type.to_owned(),
                players: players.iter().map(|p| Player {
                    name: p.to_owned(),
                    nicks: vec!(),
                    twitch: "".to_owned(),
                }).collect()
            };

            let new_json = serde_json::to_string_pretty(&new_project).unwrap();
            
            fs::write(project_file, new_json).map_err(|e| format!("Failed to write project file: {}", e))?;
    
            Ok(())
        },
        RunType::Run { save_file, project_file} => {
            let project: Project = match read_to_string(&project_file) {
                Ok(file) => {
                    serde_json::from_str::<Project>(&file).map_err(|e| format!("Failed to parse project file: {}", e))?
                },
                Err(why) => return Err(format!("Failed to open project file {}: {}", &project_file.to_str().unwrap(), why)),
            };


            let mut state: ProjectState = match read_to_string(save_file) {
                Ok(file) => {
                    let save_res = serde_json::from_str::<ProjectStateSerializer>(&file).map_err(|e| format!("Failed to parse state file: {}", e));
                    match save_res {
                        Ok(save) => ProjectState::from_save_state(&save, &project).map_err(|e| e.to_string())?,
                        Err(_) => ProjectState {
                            running: false,
                            active_players: vec!(),
                            streams: HashMap::new(),
                            type_state: ProjectTypeState::MarathonState { runner_times: HashMap::new() }, 
                        }
                    }
                },
                Err(_) => {
                    println!("Project state file does not exist, creating...");
                    File::create(save_file).map_err(|err| format!("Failed to create state file {}: {}", save_file.to_str().unwrap(), err))?;
                    ProjectState {
                        running: false,
                        active_players: vec!(),
                        streams: HashMap::new(),
                        type_state: ProjectTypeState::MarathonState { runner_times: HashMap::new() }, 
                    }
                }
            };

            

            loop {
                let mut cmd_str = String::new();
                io::stdin().read_line(&mut cmd_str).unwrap();

                if cmd_str.starts_with("exit") {
                    break;
                }

                let cmd = parse_cmds(&cmd_str, &project);

                match &cmd {
                    Ok(cmd) => {
                        let state_res = state.apply_cmds(cmd);
                        match state_res {
                            Ok(new_state) => {
                                state = new_state;
                            }
                            Err(err) => println!("State applying error: {}", err.to_string()),
                        }
                    },
                    Err(err) => {
                        println!("Command error: {}", err.to_string());
                    }
                }
            }

            println!("Saving project state file...");


            let state_save = serde_json::to_string_pretty(&state.to_save_state()).unwrap();
            fs::write(save_file, state_save).map_err(|e| format!("Failed to write save file: {}", e))?;

            Ok(())

        },
    }

   
    }


#[cfg(test)]
mod tests {
    use std::{collections::HashMap, time::Duration};

    use crate::{state::{ProjectState, Project, ProjectType, ProjectTypeState, ProjectStateSerializer}, user::Player, cmd::{parse_cmd, Command}};

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
       

        assert_eq!(Command::SetPlayers(vec!(&project.players[0], &project.players[1])), parse_cmd("!set john bill", &project).unwrap());
        assert!(parse_cmd("!set earl bill", &project).is_err());
        assert!(parse_cmd("!set bill will dill", &project).is_err());

        assert_eq!(Command::SetScore(&project.players[0], Duration::from_millis(222)), parse_cmd("!record joe 222", &project).unwrap());
        assert!(parse_cmd("!record joe 22e", &project).is_err()); 
    }

    #[test]
    fn test_cmd_apply() {
        let project = Project {
            kind: ProjectType::Marathon,
            players: vec!(
                Player {name:"Joe".to_owned(),nicks:vec!("Joseph".to_owned(),"John".to_owned()), twitch: "test".to_owned() },
                Player {name:"William".to_owned(),nicks:vec!("Will".to_owned(),"Bill".to_owned()), twitch: "test2".to_owned() },
            )
        };

        let mut state = ProjectState {
            running: false,
            active_players: vec!(),
            streams: HashMap::new(),
            type_state: ProjectTypeState::MarathonState { runner_times: HashMap::new() }, 
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

    #[test]
    fn test_serialization() {
        let project = Project {
            kind: ProjectType::Marathon,
            players: vec!(
                Player {name:"Joe".to_owned(),nicks:vec!("Joseph".to_owned(),"John".to_owned()), twitch: "test".to_owned() },
                Player {name:"William".to_owned(),nicks:vec!("Will".to_owned(),"Bill".to_owned()), twitch: "test2".to_owned() },
            )
        };

        let saved_project = serde_json::to_string(&project).unwrap();
        let regen_project: Project = serde_json::from_str(&saved_project).unwrap();

        assert_eq!(project, regen_project);

        let mut state = ProjectState {
            running: false,
            active_players: vec!(),
            streams: HashMap::new(),
            type_state: ProjectTypeState::MarathonState { runner_times: HashMap::new() }, 
        };

        state = state.apply_cmd(&parse_cmd("!toggle joe", &project).unwrap()).unwrap();

        let saved_state = serde_json::to_string(&state.to_save_state()).unwrap();
        let regen_state_serial: ProjectStateSerializer = serde_json::from_str(&saved_state).unwrap();
        let regen_state = ProjectState::from_save_state(&regen_state_serial, &project).unwrap();

        assert_eq!(state, regen_state);
    }
}
