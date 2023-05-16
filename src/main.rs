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

pub enum Site {
    CodeGolf,
    // PLDI
    PLDI
}

pub struct Ids {
    p_200: HashSet<String>,
    p_202: HashSet<String>,
    // PLDI
    p_716: HashSet<String>,
    p_717: HashSet<String>
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
        p_202: HashSet::new(),
        // PLDI
        p_716: HashSet::new(),
        p_717: HashSet::new()
    }));
    
    chat::find_known_ids(240, &Site::CodeGolf, Arc::clone(&main_arc), Arc::clone(&ids)).await?;
    // PLDI
    chat::find_known_ids(146046, &Site::PLDI, Arc::clone(&main_arc), Arc::clone(&ids)).await?;
    
    let watch_0 = tokio::spawn(watch::watch_ws(0, Arc::clone(&ids), [Arc::clone(&main_arc), Arc::clone(&sandbox_arc)], Arc::clone(&config_arc)));
    let watch_1 = tokio::spawn(watch::watch_ws(1, Arc::clone(&ids), [Arc::clone(&main_arc), Arc::clone(&sandbox_arc)], Arc::clone(&config_arc)));
    
    let chat_main_240 = tokio::spawn(chat::chat_ws(240, &Site::CodeGolf, "main", Arc::clone(&main_arc), Arc::clone(&ids)));
    // PLDI
    let chat_main_146046 = tokio::spawn(chat::chat_ws(146046, &Site::PLDI, "main", Arc::clone(&main_arc), Arc::clone(&ids)));
    let chat_sandbox = tokio::spawn(chat::chat_ws(240, &Site::CodeGolf, "sandbox", Arc::clone(&sandbox_arc), Arc::clone(&ids)));
    
    tokio::try_join!(watch_0, watch_1, chat_main_240, chat_main_146046, chat_sandbox)?;
    
    Ok(())
}
