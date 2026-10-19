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
            .service(crate::web_api::health)
            .service(crate::web_api::create_auth_key)
            .service(crate::web_api::delete_auth_key)
            .service(crate::web_api::create_user)
            .service(crate::web_api::put_share)
            .service(crate::web_api::get_task_shares)
            .service(crate::web_api::delete_share)
            .service(crate::web_api::update_task_by_id)
            // `/users/me` is registered before `/users/{id}`, because the router
            // matches in registration order and `me` is not an id.
            .service(crate::web_api::get_current_user)
            .service(crate::web_api::get_user)
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
