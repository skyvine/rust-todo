use clap::Parser;
use rust_todo::run;
use tracing_subscriber::{fmt, EnvFilter};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Arguments {
    /// The IP address to listen on
    #[arg(short, long, default_value_t = String::from("127.0.0.1"))]
    ip_address: String,

    #[arg(short, long, default_value_t = 8000)]
    /// The port to listen on
    port: u16,
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let arguments = Arguments::parse();
    fmt().json().with_env_filter(EnvFilter::from_default_env()).init();
    println!("Running on {}:{}", arguments.ip_address, arguments.port);
    run(arguments.ip_address, arguments.port).await
}
