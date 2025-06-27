mod schema;

use actix_web::{get, post, App, HttpResponse, HttpServer, Responder};
use diesel::prelude::*;
use dotenvy::dotenv;
use std::env;
use tracing::{event, span, Level};
use uuid::Uuid;

/// An endpoint to checks that the server is up.
/// 
/// This endpoint always responds with status OK simply to verify that
/// the server is running and responding to requests.
#[get("/is_alive")]
async fn is_alive() -> impl Responder {
    event!(Level::TRACE, "Responding to is_alive check");
    HttpResponse::Ok()
}

#[derive(serde::Deserialize, serde::Serialize)]
struct NewAccountInfo {
    name: String,
    password: String,
}

fn establish_connection() -> Result<PgConnection, ConnectionError> {
    let database_url = match env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            let msg = String::from("DATABASE_URL is not set, unable to establish connection");
            event!(Level::ERROR, msg);
            return Err(ConnectionError::InvalidConnectionUrl(msg))
        }
    };

    let connection = PgConnection::establish(&database_url);
    event!(Level::TRACE, "Established connection to database.");
    connection
}

#[post("/register_account")]
async fn register_account(mut account_info: actix_web::web::Json<NewAccountInfo>) -> impl Responder {
    use self::schema::users::dsl::*;

    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Registering Account", %request_id).entered();

    let mut connection = establish_connection();

    if let Err(_) = connection {
        event!(Level::ERROR, "Unable to establish connection to database.");
        return HttpResponse::InternalServerError().body(format!("Request ID: {request_id}"));
    }

    // move not allowed in .values() call below, avoid cloning by replacing
    let un = std::mem::take(&mut account_info.name);
    let pw = std::mem::take(&mut account_info.password);

    let query = diesel::insert_into(users).values((username.eq(un), password.eq(pw)));

    event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

    let result = query.execute(connection.as_mut().unwrap());
    match result {
        Ok(_) => {
            event!(Level::TRACE, "Query succeeded");
            HttpResponse::Ok().finish()
        },
        Err(e) => {
            event!(Level::ERROR, "Query failed: {e}");
            HttpResponse::InternalServerError().body(format!("Request ID: {request_id}"))
        }
    }
}

pub async fn run(ip_address: String, port: u16) -> std::io::Result<()> {
    dotenv().ok();
    HttpServer::new(|| {
            App::new()
            .service(is_alive)
        })
    .bind((ip_address, port))?
    .run()
    .await
}

#[cfg(test)]
mod tests {
    use actix_web::{body::MessageBody, test, App};
    use ctor::ctor;
    use tracing_subscriber::fmt;

    #[ctor]
    unsafe fn global_init() {
        fmt().event_format(fmt::format().pretty()).init();
    }

    #[actix_web::test]
    async fn is_alive() {
        let app = test::init_service(App::new().service(super::is_alive)).await;
        let request = test::TestRequest::get().uri("/is_alive").to_request();
        let response = test::call_service(&app, request).await;
        assert!(response.status().is_success());
        assert_eq!(response.into_body().size(), actix_web::body::BodySize::Sized(0));
    }

    #[actix_web::test]
    async fn regitering_account_is_successful() {
        let app = test::init_service(App::new().service(super::register_account)).await;
        let request = test::TestRequest::post().uri("/register_account").set_json(super::NewAccountInfo {
            name:     String::from("new-name"),
            password: String::from("new-password")
        }).to_request();
        let response = test::call_service(&app, request).await;

        if !response.status().is_success() {
            let formatted_response = format!("{response:?}");
            let body = response.into_body();
            panic!("Response indicated failure: {formatted_response}{body:?}")
        }
    }
}
