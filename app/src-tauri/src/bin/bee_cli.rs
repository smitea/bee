use std::io;

fn main() {
    let result = app_lib::cli::database_path().and_then(|database_path| {
        let args = std::env::args().skip(1).collect::<Vec<_>>();
        let stdout = io::stdout();
        let mut output = stdout.lock();
        app_lib::cli::run(&args, &database_path, &mut output)
    });
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
