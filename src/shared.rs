use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use lazy_static::lazy_static;

pub type Sessions = Arc<Mutex<HashMap<String, String>>>;

lazy_static! {
    pub static ref SESSIONS: Sessions = Arc::new(Mutex::new(HashMap::new()));
}
