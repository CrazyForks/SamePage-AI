#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Some(result) = lexora_buddy_lib::run_headless_command_from_env() {
        match result {
            Ok(output) => {
                println!("{output}");
                return;
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }

    if let Some(result) = lexora_buddy_lib::run_codex_smoke_command_from_env() {
        match result {
            Ok(output) => {
                println!("{output}");
                return;
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }

    if let Some(result) = lexora_buddy_lib::run_window_smoke_check_from_env() {
        match result {
            Ok(()) => return,
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }

    if let Some(result) = lexora_buddy_lib::run_native_pet_smoke_command_from_env() {
        match result {
            Ok(output) => {
                println!("{output}");
                return;
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }

    if let Some(result) = lexora_buddy_lib::run_native_pet_drag_replay_command_from_env() {
        match result {
            Ok(output) => {
                println!("{output}");
                return;
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }

    if let Some(result) = lexora_buddy_lib::run_affective_state_command_from_env() {
        match result {
            Ok(output) => {
                println!("{output}");
                return;
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }

    if let Some(result) = lexora_buddy_lib::run_choreography_dev_fixture_command_from_env() {
        match result {
            Ok(()) => return,
            Err(error) => {
                eprintln!("{}", error.message());
                std::process::exit(error.exit_code());
            }
        }
    }

    if let Some(result) =
        lexora_buddy_lib::run_startup_recoverable_choreography_list_command_from_env()
    {
        match result {
            Ok(output) => {
                println!("{output}");
                return;
            }
            Err(error) => {
                eprintln!("{}", error.message());
                std::process::exit(error.exit_code());
            }
        }
    }

    if let Some(result) =
        lexora_buddy_lib::run_startup_recoverable_choreography_replay_command_from_env()
    {
        match result {
            Ok(output) => {
                println!("{output}");
                return;
            }
            Err(error) => {
                eprintln!("{}", error.message());
                std::process::exit(error.exit_code());
            }
        }
    }

    if let Some(result) =
        lexora_buddy_lib::run_startup_recoverable_choreography_replay_next_command_from_env()
    {
        match result {
            Ok(output) => {
                println!("{output}");
                return;
            }
            Err(error) => {
                eprintln!("{}", error.message());
                std::process::exit(error.exit_code());
            }
        }
    }

    if let Some(result) = lexora_buddy_lib::run_native_pet_sidecar_from_env() {
        match result {
            Ok(()) => return,
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }

    lexora_buddy_lib::run()
}
