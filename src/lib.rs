use actix_web::{get, post, App, HttpResponse, HttpServer, Responder};

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

#[post("/register_account")]
async fn register_account(account_info: actix_web::web::Json<NewAccountInfo>) -> impl Responder {
    HttpResponse::Ok()
}

pub async fn run(ip_address: String, port: u16) -> std::io::Result<()> {
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
