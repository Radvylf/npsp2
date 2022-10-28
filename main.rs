mod login;
mod watch;
mod chat;

use std::sync::Arc;
use tokio::sync::Mutex;
use serde::Deserialize;
use std::collections::HashSet;
use std::time::SystemTime;

pub fn time() -> u128 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis()
}

pub struct Ids {
    p_200: HashSet<String>,
    p_202: HashSet<String>
}

#[derive(Deserialize)]
pub struct Config {
    np: UserConfig,
    sp: UserConfig,
    key: String
}

#[derive(Deserialize)]
pub struct UserConfig {
    email: String,
    password: String
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config: Config = serde_json::from_str(&std::fs::read_to_string("config.json")?)?;
    
    let config_arc = Arc::new(config);
    
    let (user_main, user_sandbox) = login::login(Arc::clone(&config_arc)).await?;
    
    let main_arc = Arc::new(user_main);
    let sandbox_arc = Arc::new(user_sandbox);
    
    let ids = Arc::new(Mutex::new(Ids {
        p_200: HashSet::new(),
        p_202: HashSet::new()
    }));
    
    let watch_0 = tokio::spawn(watch::watch_ws(0, Arc::clone(&ids), [Arc::clone(&main_arc), Arc::clone(&sandbox_arc)], Arc::clone(&config_arc)));
    let watch_1 = tokio::spawn(watch::watch_ws(1, Arc::clone(&ids), [Arc::clone(&main_arc), Arc::clone(&sandbox_arc)], Arc::clone(&config_arc)));
    
    let chat_main = tokio::spawn(chat::chat_ws("main", Arc::clone(&main_arc), Arc::clone(&ids)));
    let chat_sandbox = tokio::spawn(chat::chat_ws("sandbox", Arc::clone(&sandbox_arc), Arc::clone(&ids)));
    
    tokio::try_join!(watch_0, watch_1, chat_main, chat_sandbox)?;
    
    Ok(())
}
