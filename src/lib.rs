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
            .service(crate::web_api::resource!(
                "/auth_keys",
                POST => crate::web_api::create_auth_key,
                DELETE => crate::web_api::delete_auth_key,
            ))
            .service(crate::web_api::resource!(
                "/tasks",
                GET => crate::web_api::get_tasks,
                POST => crate::web_api::add_task,
            ))
            .service(crate::web_api::resource!(
                "/tasks/{id}",
                GET => crate::web_api::get_task_by_id,
                PATCH => crate::web_api::update_task_by_id,
            ))
            .service(crate::web_api::resource!(
                "/tasks/{id}/shares",
                GET => crate::web_api::get_task_shares,
            ))
            .service(crate::web_api::resource!(
                "/tasks/{id}/shares/{username}",
                PUT => crate::web_api::put_share,
                DELETE => crate::web_api::delete_share,
            ))
            .service(crate::web_api::resource!(
                "/users",
                POST => crate::web_api::create_user,
            ))
            // `/users/me` is registered before `/users/{id}`, because the router
            // matches in registration order and `me` is not an id.
            .service(crate::web_api::resource!(
                "/users/me",
                GET => crate::web_api::get_current_user,
            ))
            .service(crate::web_api::resource!(
                "/users/{id}",
                GET => crate::web_api::get_user,
            ))
            .service(crate::web_api::resource!(
                "/health",
                GET => crate::web_api::health,
            ))
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
