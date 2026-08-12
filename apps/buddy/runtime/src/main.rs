use std::io::{stdin, stdout, BufReader};

fn main() {
    if run_cli_command() {
        return;
    }

    let reader = BufReader::new(stdin().lock());
    let writer = stdout();

    if let Err(error) = lexora_buddy_runtime::server::serve(
        reader,
        writer,
        lexora_buddy_runtime::runtime::RuntimeApplication::initialize().unwrap_or_else(|error| {
            eprintln!("{error}");
            std::process::exit(1);
        }),
    ) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run_cli_command() -> bool {
    macro_rules! run_output_command {
        ($command:expr) => {
            if let Some(result) = $command {
                match result {
                    Ok(output) => println!("{output}"),
                    Err(error) => {
                        eprintln!("{error}");
                        std::process::exit(1);
                    }
                }
                return true;
            }
        };
    }

    run_output_command!(lexora_buddy_runtime::run_headless_command_from_env());
    run_output_command!(lexora_buddy_runtime::run_affective_state_command_from_env());

    false
}
