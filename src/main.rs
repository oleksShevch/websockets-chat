mod auth;
mod chat;
mod handling_files;
mod shared;

use warp::Filter;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use env_logger;
use rusqlite::Connection;

use crate::chat::Clients;

#[tokio::main]
async fn main() {
    env_logger::init();

    let conn = Connection::open("users.db").expect("Failed to open the database");
    conn.execute(
        "CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT NOT NULL UNIQUE,
            password TEXT NOT NULL
        )",
        [],
    )
        .expect("Failed to create users table");

    let db = Arc::new(Mutex::new(conn));

    let register_db = db.clone();
    let login_db = db.clone();

    // Shared state to hold connected clients
    let clients: Clients = Arc::new(Mutex::new(HashMap::new()));
    let clients_filter = warp::any().map(move || clients.clone());

    let register_route = warp::path("register")
        .and(warp::post())
        .and(warp::body::json())
        .and(auth::with_db(register_db))
        .and_then(auth::handle_register);

    let login_route = warp::path("login")
        .and(warp::post())
        .and(warp::body::json())
        .and(auth::with_db(login_db))
        .and_then(auth::handle_login);

    let chat_route = warp::path("ws")
        .and(warp::ws())
        .and(warp::cookie::optional("session_token"))
        .and(clients_filter)
        .and_then(chat::handle_ws_auth);

    let download_file_route = warp::path("download")
        .and(warp::path::param::<String>()) // File ID
        .and_then(handling_files::handle_file_download);

    let root_route = warp::path::end()
        .map(|| warp::redirect::temporary(warp::http::Uri::from_static("/login.html")));

    let static_files = warp::fs::dir("./static");

    let cors = warp::cors()
        .allow_any_origin() // For development; specify allowed origins in production
        .allow_methods(vec!["GET", "POST", "OPTIONS"])
        .allow_headers(vec!["Content-Type"]);

    let routes = root_route
        .or(login_route)
        .or(register_route)
        .or(chat_route)
        .or(download_file_route)
        .or(static_files)
        .with(cors)
        .with(warp::log("chat_app"));

    println!("Server running at http://localhost:3030/");
    warp::serve(routes).run(([127, 0, 0, 1], 3030)).await;
}
