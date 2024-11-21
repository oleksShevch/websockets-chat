
use warp::Filter;
use warp::ws::{Message, WebSocket};
use std::sync::{Arc, Mutex};
use futures::{FutureExt, StreamExt};
use tokio::sync::mpsc::{self, UnboundedSender};
use uuid::Uuid;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::shared::SESSIONS;

pub type Clients = Arc<Mutex<HashMap<Uuid, Client>>>;

pub struct Client {
    pub sender: UnboundedSender<Message>,
}

#[derive(Debug)]
pub struct Unauthorized;

impl warp::reject::Reject for Unauthorized {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageType {
    System,
    User,
    Init,
    File,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub message_type: MessageType,
    pub content: String,
    pub sender_username: Option<String>,
    pub filename: Option<String>,
    pub file_id: Option<String>,
}

pub async fn handle_ws_auth(
    ws: warp::ws::Ws,
    session_token: Option<String>,
    clients: Clients,
) -> Result<Box<dyn warp::Reply + Send>, warp::Rejection> {
    if let Some(token) = session_token {
        // Check if the session token is valid
        let sessions = SESSIONS.lock().unwrap();
        if sessions.contains_key(&token) {
            // handle the WebSocket connection
            let reply = ws.on_upgrade(move |socket| handle_connection(socket, clients, token));
            Ok(Box::new(reply))
        } else {
            Err(warp::reject::custom(Unauthorized))
        }
    } else {
        Err(warp::reject::custom(Unauthorized))
    }
}

pub async fn handle_connection(ws: WebSocket, clients: Clients, session_token: String) {
    let username = {
        let sessions = SESSIONS.lock().unwrap();
        sessions
            .get(&session_token)
            .cloned()
            .unwrap_or("Unknown".to_string())
    };

    // Assign a unique ID to the client
    let client_id = Uuid::new_v4();
    println!("Client {} connected as {}", client_id, username);

    // Split the WebSocket into sender and receiver
    let (ws_tx, mut ws_rx) = ws.split();

    // Create a channel to send messages to the client
    let (tx, rx) = mpsc::unbounded_channel();
    let rx = tokio_stream::wrappers::UnboundedReceiverStream::new(rx);
    let send_to_client = rx.map(Ok).forward(ws_tx).map(|result| {
        if let Err(e) = result {
            eprintln!("WebSocket send error: {}", e);
        }
    });

    // Insert the sender into the shared clients list
    let client = Client { sender: tx.clone() };
    clients.lock().unwrap().insert(client_id, client);

    let init_message = ChatMessage {
        message_type: MessageType::Init,
        content: String::new(), // No content needed for init message
        sender_username: Some(username.clone()),
        filename: None,
        file_id: None,
    };
    let _ = tx.send(Message::text(serde_json::to_string(&init_message).unwrap()));

    let welcome_message = format!("Welcome to the chat, {}!", username);
    let _ = tx.send(Message::text(
        serde_json::to_string(&ChatMessage {
            message_type: MessageType::System,
            content: welcome_message,
            sender_username: None,
            filename: None,
            file_id: None,
        })
            .unwrap(),
    ));

    let join_message = format!("{} has joined the chat.", username);
    let join_chat_msg = ChatMessage {
        message_type: MessageType::System,
        content: join_message,
        sender_username: None,
        filename: None,
        file_id: None,
    };
    let serialized = serde_json::to_string(&join_chat_msg).unwrap();
    broadcast_to_all(&clients, serialized).await;

    let clients_clone = clients.clone();
    let username_clone = username.clone();
    let receive_from_client = async move {
        while let Some(result) = ws_rx.next().await {
            match result {
                Ok(msg) => {
                    if msg.is_text() {
                        let msg_text = msg.to_str().unwrap_or("").to_string();

                        // Try to parse the message as a ChatMessage
                        if let Ok(chat_msg) = serde_json::from_str::<ChatMessage>(&msg_text) {
                            match chat_msg.message_type {
                                MessageType::User => {
                                    // Broadcast text message
                                    let serialized = serde_json::to_string(&chat_msg).unwrap();
                                    broadcast_to_all(&clients_clone, serialized).await;
                                }
                                MessageType::File => {
                                    // Handle file message
                                    handle_file_message(chat_msg, &clients_clone).await;
                                }
                                _ => {}
                            }
                        } else {
                            // If parsing fails, treat it as a regular text message
                            let chat_msg = ChatMessage {
                                message_type: MessageType::User,
                                content: msg_text,
                                sender_username: Some(username_clone.clone()),
                                filename: None,
                                file_id: None,
                            };
                            let serialized = serde_json::to_string(&chat_msg).unwrap();
                            broadcast_to_all(&clients_clone, serialized).await;
                        }
                    }
                }
                Err(e) => {
                    eprintln!("WebSocket error (client {}): {}", client_id, e);
                    break;
                }
            }
        }

        // Client disconnected, remove from the list
        clients_clone.lock().unwrap().remove(&client_id);
        println!("Client {} disconnected", client_id);

        // Notify all clients that a user has left
        let leave_message = format!("{} has left the chat.", username_clone);
        let leave_chat_msg = ChatMessage {
            message_type: MessageType::System,
            content: leave_message,
            sender_username: None,
            filename: None,
            file_id: None,
        };
        let serialized = serde_json::to_string(&leave_chat_msg).unwrap();
        broadcast_to_all(&clients_clone, serialized).await;
    };

    // Run both sending and receiving concurrently
    tokio::spawn(send_to_client);
    tokio::spawn(receive_from_client);
}

pub async fn broadcast_to_all(clients: &Clients, message: String) {
    let clients_lock = clients.lock().unwrap();
    for (_, client) in clients_lock.iter() {
        let _ = client.sender.send(Message::text(message.clone()));
    }
}

pub async fn handle_file_message(chat_msg: ChatMessage, clients: &Clients) {
    use tokio::fs;
    use uuid::Uuid;
    use base64;

    // Ensure the necessary fields are present
    if let (Some(content), Some(filename), Some(sender_username)) = (
        Some(chat_msg.content),
        chat_msg.filename.clone(),
        chat_msg.sender_username.clone(),
    ) {
        // Decode the Base64 file content
        let file_data = match base64::decode(&content) {
            Ok(data) => data,
            Err(e) => {
                eprintln!("Failed to decode file content: {}", e);
                return;
            }
        };

        // Generate a unique file ID
        let file_id = Uuid::new_v4().to_string();

        // Save the file to the uploads directory
        let filepath = format!("./uploads/{}_{}", file_id, filename);

        if let Err(e) = fs::create_dir_all("./uploads").await {
            eprintln!("Failed to create uploads directory: {}", e);
            return;
        }

        if let Err(e) = fs::write(&filepath, &file_data).await {
            eprintln!("Failed to save file {}: {}", filepath, e);
            return;
        }

        // Broadcast the file message to all clients
        let file_message = ChatMessage {
            message_type: MessageType::File,
            content: String::new(), // No content needed
            sender_username: Some(sender_username),
            filename: Some(filename),
            file_id: Some(file_id),
        };

        let serialized = serde_json::to_string(&file_message).unwrap();
        broadcast_to_all(clients, serialized).await;
    }
}
