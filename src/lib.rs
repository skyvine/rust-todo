use actix_web::{get, App, HttpResponse, HttpServer, Responder};

/// An endpoint to checks that the server is up.
/// 
/// This endpoint always responds with status OK simply to verify that
/// the server is running and responding to requests.
#[get("/is_alive")]
async fn is_alive() -> impl Responder {
    HttpResponse::Ok()
}

pub async fn run() -> std::io::Result<()> {
    HttpServer::new(|| {
        App::new()
        .service(is_alive)
    })
    .bind(("127.0.0.1", 8123))?
    .run()
    .await
}
