use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match args.as_slice() {
        [_, command, value] if command == "parse-duration" => {
            match agent_forge_core::parse_duration_ms(value) {
                Ok(ms) => {
                    println!("{ms}");
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("invalid duration: {error:?}");
                    ExitCode::from(2)
                }
            }
        }
        _ => {
            eprintln!("usage: agent-forge-core parse-duration <duration>");
            ExitCode::from(2)
        }
    }
}
