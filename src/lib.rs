mod schema;

use actix_web::{get, post, App, HttpResponse, HttpServer, Responder};
use diesel::prelude::*;
use dotenvy::dotenv;
use schema::users;
use serde_json::json;
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

// This struct is needed to set the return type in the query in user_exists, but the fields are not
// (yet) used. Ignore the dead code warning because the fields need to exist for diesel to validate
// the struct.
#[allow(dead_code)]
#[derive(Queryable, Selectable)]
#[diesel(check_for_backend(diesel::pg::Pg))]
struct User {
    id: i32,
    username: String,
    password: String,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct NewAccountInfo {
    name: String,
    password: String,
}

/// Open a new connection to the database. The DATABASE_URL environment variable must be defined and
/// point to a running database.
fn establish_connection() -> Result<PgConnection, ConnectionError> {
    let database_url = match env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            let msg = String::from("DATABASE_URL is not set, unable to establish connection");
            event!(Level::ERROR, msg);
            return Err(ConnectionError::InvalidConnectionUrl(msg))
        }
    };

    PgConnection::establish(&database_url)
}

/// Returns true if a user with the given name exists, false otherwise.
fn user_exists(name: &String, connection: &mut PgConnection) -> Result<bool, diesel::result::Error> {
    use self::schema::users::dsl::*;
    let query = users.filter(username.eq(name)).select(User::as_select());
    event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));
    Ok(!query.load(connection)?.is_empty())
}

/// Create an account with the given account info.
/// 
/// On a success, the user will be added to the database as a valid user and return HTTP OK.
/// 
/// In any error state this will create a log event at the ERROR level and return an appropriate
/// error HTTP response to send back to the client.
#[post("/register_account")]
async fn register_account(mut account_info: actix_web::web::Json<NewAccountInfo>) -> impl Responder {
    use self::schema::users::dsl::*;

    let request_id = Uuid::new_v4();
    let _enter_guard = span!(Level::ERROR, "Registering Account", %request_id).entered();

    let connection = establish_connection();

    match connection {
        Ok(mut conn) => {
            event!(Level::TRACE, "Established connection to database.");

            // move not allowed in .values() call below, avoid cloning by replacing
            let un = std::mem::take(&mut account_info.name);
            let pw = std::mem::take(&mut account_info.password);

            match user_exists(&un, &mut conn) {
                Ok(false) => {
                    let query = diesel::insert_into(users).values((username.eq(un), password.eq(pw)));

                    event!(Level::TRACE, "Running query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

                    let result = query.execute(&mut conn);
                    match result {
                        Ok(_) => {
                            event!(Level::TRACE, "Query succeeded");
                            HttpResponse::Ok().finish()
                        },
                        Err(e) => {
                            event!(Level::ERROR, "Query failed: {e}");
                            HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{request_id}")})))
                        }
                    }
                },

                Ok(true) => HttpResponse::Conflict().body(format!("{}", json!({"request_id": format!("{request_id}")}))),

                Err(e) => {
                    event!(Level::ERROR, "Unable to query database for existing users: {e}");
                    HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{request_id}")})))
                }
            }
        },

        Err(e) => {
            event!(Level::ERROR, "Unable to establish connection to database: {e}.");
            HttpResponse::InternalServerError().body(format!("{}", json!({"request_id": format!("{request_id}")})))
        }
    }
}

/// Run the server on the given IP address and port.
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
    use tracing::Level;
    use tracing_subscriber::fmt;

    #[ctor]
    unsafe fn global_init() {
        use super::schema::users::dsl::*;
        use super::RunQueryDsl;

        // Print log messages to help debug failed tests
        fmt().event_format(fmt::format().pretty()).with_max_level(Level::TRACE).init();

        // The tests depend on the state of the database, so ensure we always start with a clean slate.
        diesel::delete(users).execute(&mut super::establish_connection().expect("Unable to connect to test database."));
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

    #[actix_web::test]
    async fn duplicate_username_fails() {
        let name = String::from("duplicated-name");
        let password = String::from("duplicated-password");

        let app = test::init_service(App::new().service(super::register_account)).await;
        let first_request = test::TestRequest::post().uri("/register_account").set_json(super::NewAccountInfo {
            name:     name.clone(),
            password: password.clone(),
        }).to_request();

        let first_response = test::call_service(&app, first_request).await;
        if !first_response.status().is_success() {
            let formatted_response = format!("{first_response:?}");
            let body = first_response.into_body();
            panic!("Response indicated failure: {formatted_response}{body:?}")
        }

        let second_request = test::TestRequest::post().uri("/register_account").set_json(super::NewAccountInfo {
            name,
            password,
        }).to_request();
        let second_response = test::call_service(&app, second_request).await;
        if !second_response.status().is_client_error() {
            let formatted_response = format!("{second_response:?}");
            let body = second_response.into_body();
            panic!("Response indicated success: {formatted_response}{body:?}")
        }
    }
}
