mod database;
mod domain_types;
mod schema;
mod web_api;

use actix_web::{App, HttpServer};
use dotenvy::dotenv;

/// Run the server on the given IP address and port.
pub async fn run(ip_address: String, port: u16) -> std::io::Result<()> {
    dotenv().ok();
    HttpServer::new(|| {
            App::new()
            .service(web_api::is_alive)
            .service(web_api::register_account)
            .service(web_api::login)
            .service(web_api::whoami)
        })
    .bind((ip_address, port))?
    .run()
    .await
}

