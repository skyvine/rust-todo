mod core;
mod domain_types;
mod schema;
mod web_api;

use actix_web::{App, HttpServer, web};
use dotenvy::dotenv;

macro_rules! build_app {
    () => {
        App::new()
            .app_data(
                web::QueryConfig::default().error_handler(crate::web_api::query_error_handler),
            )
            .app_data(web::JsonConfig::default().error_handler(crate::web_api::json_error_handler))
            .service(crate::web_api::add_task)
            .service(crate::web_api::get_tasks)
            .service(crate::web_api::get_task_by_id)
            .service(crate::web_api::is_alive)
            .service(crate::web_api::login)
            .service(crate::web_api::logout)
            .service(crate::web_api::register_account)
            .service(crate::web_api::share_task)
            .service(crate::web_api::get_task_shares)
            .service(crate::web_api::unshare_task)
            .service(crate::web_api::update_task_by_id)
            .service(crate::web_api::get_user_by_id)
            .service(crate::web_api::whoami)
    };
}

// This is used by the test suite
#[allow(unused_imports)]
pub(crate) use build_app;

/// Run the server on the given IP address and port.
pub async fn run(ip_address: String, port: u16) -> std::io::Result<()> {
    dotenv().ok();
    HttpServer::new(|| build_app!())
        .bind((ip_address, port))?
        .run()
        .await
}
