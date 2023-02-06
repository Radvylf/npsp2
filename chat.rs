use std::{error::Error, fmt};
use std::sync::Arc;
use tokio::sync::Mutex;
use html_parser::{Dom, Node};
use serde::Deserialize;
use std::time::Duration;
use http::uri::Uri;
use url::Url;
use std::collections::{HashSet, HashMap};

use futures::StreamExt;
use tokio_tungstenite::tungstenite::{self, protocol::Message};

use crate::{time, Ids, login::User};

#[derive(Deserialize)]
struct WsAuth {
    url: String
}

#[derive(Deserialize)]
struct Events {
    time: u64,
    events: Vec<Event>
}

#[derive(Debug, Deserialize)]
struct Event {
    event_type: u8,
    message_id: Option<u64>,
    content: Option<String>
}

#[derive(Debug, Deserialize)]
struct RoomData {
    e: Option<Vec<Event>>
}

#[derive(Debug)]
struct MissingAckBack {}

impl Error for MissingAckBack {}

impl fmt::Display for MissingAckBack {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Missing information in ack_back HTML")
    }
}

async fn ack_back(user: Arc<User>, ack: Arc<Mutex<HashSet<u64>>>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    async fn find_script(dom: &Dom) -> Option<String> {
        fn search_node(node: &Node) -> Option<String> {
            match node {
                Node::Element(element) => {
                    if element.name == "script" && element.children.len() == 1 {
                        match &element.children[0] {
                            Node::Text(script) => script.contains("var chat = StartChat").then_some(script.to_string()),
                            _ => None
                        }
                    } else {
                        for child in &element.children {
                            let result = search_node(&child);

                            if result.is_some() {
                                return result;
                            }
                        }

                        None
                    }
                }
                _ => None
            }
        }
        
        for child in &dom.children {
            let result = search_node(&child);

            if let Some(script) = result {
                return Some(script);
            }
        }
        
        None
    }
    
    let html = user.client.get("https://chat.stackexchange.com/rooms/240").send().await?.error_for_status()?.text().await?;
    let dom = Dom::parse(&html)?;
    let script = find_script(&dom).await.ok_or(MissingAckBack {})?;
    
    let suffix = &script[script.find("var chat = StartChat").unwrap() + 20..];
    let args = &suffix[..suffix.find(");").ok_or(MissingAckBack {})?];
    let ids_dict = args.rsplit_once('\n').ok_or(MissingAckBack {})?.1.trim_start();
    
    if ids_dict != "{}" {
        let ids = ids_dict[1..ids_dict.len() - 1].split(',').map(|p| p.split_once(':').unwrap().0).collect::<Vec<&str>>();
        
        for id in ids {
            if ack.lock().await.insert(id.parse::<u64>()?) {
                user.client.post("https://chat.stackexchange.com/messages/ack").form(&[
                    ("id", &id.to_string()),
                    ("fkey", &user.fkey)
                ]).send().await?.error_for_status()?;

                println!("ack_back {}", id);
            }
        }
    }
    
    Ok(())
}

fn urls_from_dom(dom: &Dom) -> Vec<Url> {
    fn search_node(node: &Node, urls: &mut Vec<Url>) {
        match node {
            Node::Element(element) => {
                if element.name == "a" && element.attributes.contains_key("href") {
                    if let Ok(url) = Url::parse("https://chat.stackexchange.com/rooms/240/sandbox").unwrap().join(element.attributes.get("href").unwrap().as_ref().unwrap()) {
                        urls.push(url);
                    }
                } else {
                    for child in &element.children {
                        search_node(&child, urls);
                    }
                }
            }
            _ => ()
        }
    }
    
    let mut urls: Vec<Url> = Vec::new();
    
    for child in &dom.children {
        search_node(&child, &mut urls);
    }
    
    urls
}

fn url_ids(urls: &Vec<Url>, site: &str) -> HashSet<String> {
    let mut ids: HashSet<String> = HashSet::new();
    
    for url in urls {
        if url.domain().map_or(false, |domain| domain == site) {
            let path = url.path_segments().unwrap().collect::<Vec<&str>>();
            
            if path.len() != 0 {
                match (path[0], path.len()) {
                    ("questions", 2) | ("questions", 3) | ("q", 2) | ("q", 3) => {
                        ids.insert(path[1].to_owned());
                    }
                    ("questions", 4) => {
                        ids.insert(path[1].to_owned());
                        ids.insert(path[3].to_owned());
                    }
                    _ => ()
                }
            }
        }
    }
    
    ids
}

async fn known_ids(events: &Vec<Event>, ids: Arc<Mutex<Ids>>) {
    for event in events {
        if event.event_type == 1 && event.content.is_some() {
            let dom = Dom::parse(&event.content.as_ref().unwrap()).unwrap();
            
            let urls = urls_from_dom(&dom);
            
            for id in url_ids(&urls, "codegolf.stackexchange.com") {
                ids.lock().await.p_200.insert(id);
            }
            
            for id in url_ids(&urls, "codegolf.meta.stackexchange.com") {
                ids.lock().await.p_202.insert(id);
            }
        }
    }
}

pub async fn find_known_ids(user: Arc<User>, ids: Arc<Mutex<Ids>>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let events: Events = serde_json::from_str(&(user.client.post("https://chat.stackexchange.com/chats/240/events").form(&[
        ("since", "0"),
        ("mode", "Messages"),
        ("msgCount", "100"),
        ("fkey", &user.fkey)
    ]).send().await?.error_for_status()?.text().await?))?;
    
    known_ids(&events.events, Arc::clone(&ids)).await;
    
    Ok(())
}

async fn connect_chat_ws(log_id: &str, user: Arc<User>, ids: Arc<Mutex<Ids>>, ack: Arc<Mutex<HashSet<u64>>>, kill_offset: bool) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let ws_auth: WsAuth = serde_json::from_str(&(user.client.post("https://chat.stackexchange.com/ws-auth").form(&[
        ("roomid", "240"),
        ("fkey", &user.fkey)
    ]).send().await?.error_for_status()?.text().await?))?;
    
    let events: Events = serde_json::from_str(&(user.client.post("https://chat.stackexchange.com/chats/240/events").form(&[
        ("since", "0"),
        ("mode", "Messages"),
        ("msgCount", "100"),
        ("fkey", &user.fkey)
    ]).send().await?.error_for_status()?.text().await?))?;
    
    known_ids(&events.events, Arc::clone(&ids)).await;
    
    let ws_auth_uri = format!("{}?l={}", ws_auth.url, events.time).parse::<Uri>()?;
    
    let request = tungstenite::handshake::client::Request::builder()
        .method("GET")
        .header("Host", ws_auth_uri.host().unwrap())
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", tungstenite::handshake::client::generate_key())
        .header("Origin", "https://chat.stackexchange.com")
        .uri(ws_auth_uri)
        .body(())?;
    
    let mut ws_stream = tokio_tungstenite::connect_async(request).await?.0;
    
    println!("{}: open", log_id);
    
    let duration = tokio::time::sleep(if kill_offset {
        Duration::from_millis(3600000)
    } else {
        Duration::from_millis(7200000)
    });
    
    let ping: Arc<Mutex<u128>> = Arc::new(Mutex::new(time()));
    
    let pong = {
        let ping = ping.clone();
        
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(15000));
            
            loop {
                interval.tick().await;
                
                if time() - *ping.lock().await > 45000 {
                    break;
                }
            }
        })
    };
    
    let mut chat = {
        let user = user.clone();
        let ping = ping.clone();
        let log_id = log_id.to_owned();
        
        tokio::spawn(async move {
            while let Some(msg_r) = ws_stream.next().await {
                let msg = msg_r.unwrap();

                match msg {
                    Message::Text(string) => {
                        let data: HashMap<String, RoomData> = serde_json::from_str(&string).unwrap();

                        *ping.lock().await = time();

                        for room in data {
                            if let Some(events) = room.1.e {
                                for event in events {
                                    match event.event_type {
                                        8 | 18 => {
                                            if ack.lock().await.insert(event.message_id.unwrap()) {
                                                user.client.post("https://chat.stackexchange.com/messages/ack").form(&[
                                                    ("id", &event.message_id.unwrap().to_string()),
                                                    ("fkey", &user.fkey)
                                                ]).send().await.unwrap().error_for_status().unwrap();
                                                
                                                println!("{}: ack {}", log_id, event.message_id.unwrap());
                                            }
                                        }
                                        _ => ()
                                    }
                                }
                            }
                        }
                    }
                    _ => ()
                }
            }
        })
    };
    
    tokio::select!(
        _ = duration => {
            println!("{}: close (alive over {} hours)", log_id, if kill_offset { 1 } else { 2 });
            
            chat.abort();
        }
        _ = pong => {
            println!("{}: close (ping too high - {}s - or disconn)", log_id, (time() - *ping.lock().await) / 1000);
            
            chat.abort();
        }
        chat_r = &mut chat => {
            println!("{}: close (stream closed)", log_id);
            
            chat_r.unwrap();
        }
    );
    
    Ok(())
}

pub async fn chat_ws(log_id: &str, user: Arc<User>, ids: Arc<Mutex<Ids>>) {
    let ack: Arc<Mutex<HashSet<u64>>> = Arc::new(Mutex::new(HashSet::new()));
    
    let mut first = true;
    
    loop {
        ack_back(Arc::clone(&user), Arc::clone(&ack)).await.unwrap();
        
        connect_chat_ws(log_id, Arc::clone(&user), Arc::clone(&ids), Arc::clone(&ack), log_id == "sandbox" && first).await.unwrap();
        
        first = false;

        tokio::time::sleep(Duration::from_millis(2000)).await;
    }
}
