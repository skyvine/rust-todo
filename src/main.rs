use clap::Parser;
use rust_todo::run;

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

    run(arguments.ip_address, arguments.port).await
}