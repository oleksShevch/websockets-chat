use warp::Filter;
use std::sync::{Arc, Mutex};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use warp::http::StatusCode;
use log::{info, error};
use uuid::Uuid;
use warp::Reply;

use crate::shared::SESSIONS;

#[derive(Deserialize, Debug)]
pub struct UserRegister {
    pub username: String,
    pub password: String,
}

#[derive(Deserialize, Debug)]
pub struct UserLogin {
    pub username: String,
    pub password: String,
}

#[derive(Serialize, Debug)]
pub struct ResponseMessage {
    pub message: String,
}

pub fn with_db(
    db: Arc<Mutex<Connection>>,
) -> impl Filter<Extract = (Arc<Mutex<Connection>>,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || db.clone())
}

pub async fn handle_register(
    user: UserRegister,
    db: Arc<Mutex<Connection>>,
) -> Result<impl warp::Reply, warp::Rejection> {
    use bcrypt::{hash, DEFAULT_COST};

    info!("Received registration request: {:?}", user);

    if user.username.trim().is_empty() || user.password.trim().is_empty() {
        let json = warp::reply::json(&ResponseMessage {
            message: "Username and password cannot be empty.".into(),
        });
        return Ok(warp::reply::with_status(
            json,
            StatusCode::BAD_REQUEST,
        ));
    }

    let hashed_password = match hash(&user.password, DEFAULT_COST) {
        Ok(h) => h,
        Err(e) => {
            error!("Password hashing failed: {:?}", e);
            let json = warp::reply::json(&ResponseMessage {
                message: "Password hashing failed.".into(),
            });
            return Ok(warp::reply::with_status(
                json,
                StatusCode::INTERNAL_SERVER_ERROR,
            ));
        }
    };

    let conn = match db.lock() {
        Ok(lock) => lock,
        Err(e) => {
            error!("Failed to acquire DB lock: {:?}", e);
            let json = warp::reply::json(&ResponseMessage {
                message: "Internal server error.".into(),
            });
            return Ok(warp::reply::with_status(
                json,
                StatusCode::INTERNAL_SERVER_ERROR,
            ));
        }
    };

    let result = conn.execute(
        "INSERT INTO users (username, password) VALUES (?1, ?2)",
        params![user.username, hashed_password],
    );

    match result {
        Ok(_) => {
            info!("User '{}' registered successfully", user.username);
            let json = warp::reply::json(&ResponseMessage {
                message: "User registered successfully.".into(),
            });
            Ok(warp::reply::with_status(json, StatusCode::OK))
        }
        Err(e) => {
            error!("Database insertion error: {:?}", e);
            let message = if e.to_string().contains("UNIQUE constraint failed") {
                "Username already exists.".into()
            } else {
                "User registration failed.".into()
            };
            let json = warp::reply::json(&ResponseMessage { message });
            Ok(warp::reply::with_status(json, StatusCode::BAD_REQUEST))
        }
    }
}

pub async fn handle_login(
    user: UserLogin,
    db: Arc<Mutex<Connection>>,
) -> Result<Box<dyn warp::Reply>, warp::Rejection> {
    use warp::hyper::header::SET_COOKIE;
    use bcrypt::verify;

    info!("Received login request: {:?}", user);

    if user.username.trim().is_empty() || user.password.trim().is_empty() {
        let json = warp::reply::json(&ResponseMessage {
            message: "Username and password cannot be empty.".into(),
        });
        return Ok(Box::new(warp::reply::with_status(
            json,
            StatusCode::BAD_REQUEST,
        )));
    }

    let conn = match db.lock() {
        Ok(lock) => lock,
        Err(e) => {
            error!("Failed to acquire DB lock: {:?}", e);
            let json = warp::reply::json(&ResponseMessage {
                message: "Internal server error.".into(),
            });
            return Ok(Box::new(warp::reply::with_status(
                json,
                StatusCode::INTERNAL_SERVER_ERROR,
            )));
        }
    };

    let mut stmt = match conn.prepare("SELECT password FROM users WHERE username = ?1") {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to prepare statement: {:?}", e);
            let json = warp::reply::json(&ResponseMessage {
                message: "User not found.".into(),
            });
            return Ok(Box::new(warp::reply::with_status(
                json,
                StatusCode::UNAUTHORIZED,
            )));
        }
    };

    let mut rows = match stmt.query(params![user.username]) {
        Ok(r) => r,
        Err(e) => {
            error!("Failed to execute query: {:?}", e);
            let json = warp::reply::json(&ResponseMessage {
                message: "User not found.".into(),
            });
            return Ok(Box::new(warp::reply::with_status(
                json,
                StatusCode::UNAUTHORIZED,
            )));
        }
    };

    if let Ok(Some(row_result)) = rows.next() {
        let stored_password: String = match row_result.get(0) {
            Ok(p) => p,
            Err(e) => {
                error!("Failed to get password from row: {:?}", e);
                let json = warp::reply::json(&ResponseMessage {
                    message: "Error retrieving password.".into(),
                });
                return Ok(Box::new(warp::reply::with_status(
                    json,
                    StatusCode::INTERNAL_SERVER_ERROR,
                )));
            }
        };
        if verify(&user.password, &stored_password).unwrap_or(false) {
            info!("User '{}' logged in successfully", user.username);

            // Generate a session token
            let session_token = Uuid::new_v4().to_string();

            // Store the session token
            SESSIONS
                .lock()
                .unwrap()
                .insert(session_token.clone(), user.username.clone());

            // Set the session token as a cookie
            let cookie = format!("session_token={}; HttpOnly; SameSite=Strict", session_token);

            let json = warp::reply::json(&ResponseMessage {
                message: "Login successful.".into(),
            });

            let response = warp::reply::with_header(
                warp::reply::with_status(json, StatusCode::OK),
                SET_COOKIE,
                cookie,
            );

            return Ok(Box::new(response));
        }
    }

    info!("Invalid login attempt for user '{}'", user.username);
    let json = warp::reply::json(&ResponseMessage {
        message: "Invalid username or password.".into(),
    });
    Ok(Box::new(warp::reply::with_status(
        json,
        StatusCode::UNAUTHORIZED,
    )))
}
