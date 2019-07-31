use std::process;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("usage: mio-echo-server HOST:PORT");
        process::exit(1);
    }

    if let Err(err) = mio_echo_server::run(&args[1]) {
        eprintln!("{}", err);
        process::exit(1);
    }
}
