mod schema;

use actix_web::{get, post, App, HttpResponse, HttpServer, Responder};
use diesel::prelude::*;
use dotenvy::dotenv;
use std::env;

/// An endpoint to checks that the server is up.
/// 
/// This endpoint always responds with status OK simply to verify that
/// the server is running and responding to requests.
#[get("/is_alive")]
async fn is_alive() -> impl Responder {
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
            return Err(ConnectionError::InvalidConnectionUrl(msg))
        }
    };

    PgConnection::establish(&database_url)
}

#[post("/register_account")]
async fn register_account(mut account_info: actix_web::web::Json<NewAccountInfo>) -> impl Responder {
    use self::schema::users::dsl::*;

    let connection = &mut establish_connection();

    if let Err(e) = connection {
        eprintln!("Unable to establish connection: {:?}", e);
        return HttpResponse::InternalServerError();
    }

    // move not allowed in .values() call below, avoid cloning by replacing
    let un = std::mem::replace(&mut account_info.name, String::default());
    let pw = std::mem::replace(&mut account_info.password, String::default());

    match diesel::insert_into(users)
        .values((username.eq(un), password.eq(pw)))
        .execute(connection.as_mut().unwrap()) {
            Ok(_) => HttpResponse::Ok(),
            Err(_) => HttpResponse::InternalServerError()
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
        assert!(response.status().is_success());
    }
}
