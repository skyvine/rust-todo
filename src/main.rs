use rust_todo::run;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    run(String::from("127.0.0.1"), 8123).await
}