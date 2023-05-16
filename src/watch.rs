use std::sync::Arc;
use tokio::sync::Mutex;
use serde::Deserialize;
use std::time::Duration;

use futures::{StreamExt, SinkExt};
use tokio_tungstenite::tungstenite::protocol::Message;

use crate::{time, Ids, login::User, Config};

async fn post(room_id: u64, text: String, user: Arc<User>) {
    async fn try_post(room_id: u64, text: &String, user: Arc<User>) -> Option<u8> {
        let response = user.client.post(format!("https://chat.stackexchange.com/chats/{}/messages/new", room_id)).form(&[
            ("text", text),
            ("fkey", &user.fkey)
        ]).send().await.unwrap();
        
        if !response.status().is_success() {
            if response.status().as_u16() == 409 {
                let text = response.text().await.unwrap();
                
                if text.starts_with("You can perform this action again in ") {
                    return Some(text[37..].split_once(' ').unwrap().0.parse::<u8>().unwrap());
                }
            }
        }

        None
    }
    
    match try_post(room_id, &text, Arc::clone(&user)).await {
        Some(cooldown) => {
            println!("cooldown: {}s", cooldown);
            
            tokio::time::sleep(Duration::from_millis((cooldown as u64) * 1000 + 2000)).await;
            
            try_post(room_id, &text, Arc::clone(&user)).await.xor(Some(0)).unwrap();
        }
        None => ()
    }
}

#[derive(Deserialize)]
struct APIAnything {
    items: Vec<serde_json::Value>
}

async fn wait_for_api(id: &str, is_answer: bool, site: &str, user: Arc<User>, config: Arc<Config>) -> u128 {
    let start = time();
    
    async fn is_on_api(id: &str, is_answer: bool, site: &str, user: Arc<User>, config: Arc<Config>) -> bool {
        let response: APIAnything = serde_json::from_str(&(user.client.get(format!("https://api.stackexchange.com/2.3/{}/{}?site={}&key={}", if is_answer { "answers" } else { "questions" }, id, site, config.key)).send().await.unwrap().error_for_status().unwrap().text().await.unwrap())).unwrap();
        
        !response.items.is_empty()
    }
    
    for _ in 0..4 {
        if is_on_api(id, is_answer, site, Arc::clone(&user), Arc::clone(&config)).await {
            return time() - start;
        }
        
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    
    for _ in 0..4 {
        if is_on_api(id, is_answer, site, Arc::clone(&user), Arc::clone(&config)).await {
            return time() - start;
        }
        
        tokio::time::sleep(Duration::from_millis(1000)).await;
    }
    
    panic!("Took too long to wait_for_api");
}

#[derive(Debug, Deserialize)]
struct WatchData {
    action: String,
    data: String
}

#[derive(Deserialize)]
struct Question {
    id: String
}

#[derive(Deserialize)]
struct Update {
    a: String
}

#[derive(Deserialize)]
struct AnswerAdd {
    answerid: u64
}

async fn connect_watch_ws(id: usize, ids: Arc<Mutex<Ids>>, user_main: Arc<User>, user_sandbox: Arc<User>, config: Arc<Config>, kill_offset: bool) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut ws_stream = tokio_tungstenite::connect_async("wss://qa.sockets.stackexchange.com/").await?.0;
    
    ws_stream.send(Message::Text("200-questions-newest".to_owned())).await?;
    ws_stream.send(Message::Text("202-questions-newest".to_owned())).await?;
    ws_stream.send(Message::Text("202-question-2140".to_owned())).await?;
    
    // PLDI
    
    ws_stream.send(Message::Text("716-questions-newest".to_owned())).await?;
    ws_stream.send(Message::Text("717-questions-newest".to_owned())).await?;
    
    println!("watch_{}: open", id);
    
    let duration = tokio::time::sleep(if kill_offset {
        Duration::from_millis(720000)
    } else {
        Duration::from_millis(1440000)
    });
    
    let mut watch = tokio::spawn(async move {
        while let Some(msg_r) = ws_stream.next().await {
            let msg = msg_r.unwrap();

            match msg {
                Message::Text(string) => {
                    let data: WatchData = serde_json::from_str(&string).unwrap();

                    if data.action == "hb" {
                        ws_stream.send(Message::Text("pong".to_owned())).await.unwrap();
                    } else {
                        match data.action.as_str() {
                            "200-questions-newest" => {
                                let question: Question = serde_json::from_str(&data.data).unwrap();

                                println!("watch_{}: 200-questions-newest: {}", id, question.id);
                                
                                if ids.lock().await.p_200.insert(question.id.clone()) {
                                    let wait_ms = wait_for_api(&question.id, false, "codegolf", Arc::clone(&user_main), Arc::clone(&config)).await;
                                    
                                    post(240, format!("https://codegolf.stackexchange.com/q/{}", question.id), Arc::clone(&user_main)).await;
                                    
                                    println!("watch_{}: did post in {}ms", id, wait_ms);
                                }
                            }
                            "202-questions-newest" => {
                                let question: Question = serde_json::from_str(&data.data).unwrap();

                                println!("watch_{}: 202-questions-newest: {}", id, question.id);

                                if ids.lock().await.p_202.insert(question.id.clone()) {
                                    let wait_ms = wait_for_api(&question.id, false, "codegolf.meta", Arc::clone(&user_main), Arc::clone(&config)).await;
                                    
                                    post(240, format!("https://codegolf.meta.stackexchange.com/q/{}", question.id), Arc::clone(&user_main)).await;
                                    
                                    println!("watch_{}: did post in {}ms", id, wait_ms);
                                }
                            }
                            "202-question-2140" => {
                                let update: Update = serde_json::from_str(&data.data).unwrap();

                                match update.a.as_str() {
                                    "answer-add" => {
                                        let answer: AnswerAdd = serde_json::from_str(&data.data).unwrap();

                                        println!("watch_{}: 202-question-2104 answer-add: {}", id, answer.answerid);

                                        if ids.lock().await.p_202.insert(answer.answerid.to_string()) {
                                            let wait_ms = wait_for_api(&answer.answerid.to_string(), true, "codegolf.meta", Arc::clone(&user_sandbox), Arc::clone(&config)).await;
                                            
                                            post(240, format!("https://codegolf.meta.stackexchange.com/a/{}", answer.answerid), Arc::clone(&user_sandbox)).await;

                                            println!("watch_{}: did post in {}ms", id, wait_ms);
                                        }
                                    },
                                    _ => ()
                                }
                            }
                            // PLDI
                            "716-questions-newest" => {
                                let question: Question = serde_json::from_str(&data.data).unwrap();

                                println!("watch_{}: 716-questions-newest: {}", id, question.id);
                                
                                if ids.lock().await.p_716.insert(question.id.clone()) {
                                    let wait_ms = wait_for_api(&question.id, false, "languagedesign", Arc::clone(&user_main), Arc::clone(&config)).await;
                                    
                                    post(146046, format!("https://languagedesign.stackexchange.com/q/{}", question.id), Arc::clone(&user_main)).await;
                                    
                                    println!("watch_{}: did post in {}ms", id, wait_ms);
                                }
                            }
                            "717-questions-newest" => {
                                let question: Question = serde_json::from_str(&data.data).unwrap();

                                println!("watch_{}: 717-questions-newest: {}", id, question.id);

                                if ids.lock().await.p_717.insert(question.id.clone()) {
                                    let wait_ms = wait_for_api(&question.id, false, "languagedesign.meta", Arc::clone(&user_main), Arc::clone(&config)).await;
                                    
                                    post(146046, format!("https://languagedesign.meta.stackexchange.com/q/{}", question.id), Arc::clone(&user_main)).await;
                                    
                                    println!("watch_{}: did post in {}ms", id, wait_ms);
                                }
                            }
                            _ => panic!("Unknown data.action: {}", data.action)
                        }
                    }
                }
                _ => ()
            }
        }
    });
    
    tokio::select!(
        _ = duration => {
            println!("watch_{}: close (alive over {} mins)", id, if kill_offset { 12 } else { 24 });
            
            watch.abort();
        }
        watch_r = &mut watch => {
            println!("watch_{}: close (stream closed)", id);
            
            watch_r.unwrap();
        }
    );
    
    Ok(())
}

#[derive(Deserialize)]
struct APIQuestions {
    items: Vec<APIQuestion>
}

#[derive(Deserialize)]
struct APIQuestion {
    creation_date: u128,
    question_id: u64
}

#[derive(Deserialize)]
struct APIAnswers {
    items: Vec<APIAnswer>
}

#[derive(Deserialize)]
struct APIAnswer {
    creation_date: u128,
    answer_id: u64
}

async fn post_from_api(down_since: u128, ids: Arc<Mutex<Ids>>, user_main: Arc<User>, user_sandbox: Arc<User>, config: Arc<Config>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let qs_200: APIQuestions = serde_json::from_str(&(user_main.client.get(format!("https://api.stackexchange.com/2.3/questions?pagesize=12&order=desc&sort=creation&site=codegolf&filter=!bBWABX77YE7)Qj&key={}", config.key)).send().await?.error_for_status()?.text().await?))?;
    let qs_202: APIQuestions = serde_json::from_str(&(user_main.client.get(format!("https://api.stackexchange.com/2.3/questions?pagesize=12&order=desc&sort=creation&site=codegolf.meta&filter=!bBWABX77YE7)Qj&key={}", config.key)).send().await?.error_for_status()?.text().await?))?;
    let as_sandbox: APIAnswers = serde_json::from_str(&(user_sandbox.client.get(format!("https://api.stackexchange.com/2.3/questions/2140/answers?pagesize=12&order=desc&sort=creation&site=codegolf.meta&filter=!-)QWsc3sXhrz&key={}", config.key)).send().await?.error_for_status()?.text().await?))?;
    // PLDI
    let qs_716: APIQuestions = serde_json::from_str(&(user_main.client.get(format!("https://api.stackexchange.com/2.3/questions?pagesize=12&order=desc&sort=creation&site=languagedesign&filter=!bBWABX77YE7)Qj&key={}", config.key)).send().await?.error_for_status()?.text().await?))?;
    let qs_717: APIQuestions = serde_json::from_str(&(user_main.client.get(format!("https://api.stackexchange.com/2.3/questions?pagesize=12&order=desc&sort=creation&site=languagedesign.meta&filter=!bBWABX77YE7)Qj&key={}", config.key)).send().await?.error_for_status()?.text().await?))?;
    
    for q in qs_200.items {
        if q.creation_date * 1000 > down_since - 20000 {
            println!("api: qs_200: {}", q.question_id);
            
            if ids.lock().await.p_200.insert(q.question_id.to_string()) {
                post(240, format!("https://codegolf.stackexchange.com/q/{}", q.question_id), Arc::clone(&user_main)).await;
            }
        }
    }
    
    for q in qs_202.items {
        if q.creation_date * 1000 > down_since - 20000 {
            println!("api: qs_202: {}", q.question_id);
            
            if ids.lock().await.p_202.insert(q.question_id.to_string()) {
                post(240, format!("https://codegolf.meta.stackexchange.com/q/{}", q.question_id), Arc::clone(&user_main)).await;
            }
        }
    }
    
    for a in as_sandbox.items {
        if a.creation_date * 1000 > down_since - 20000 {
            println!("api: as_sandbox: {}", a.answer_id);
            
            if ids.lock().await.p_202.insert(a.answer_id.to_string()) {
                post(240, format!("https://codegolf.meta.stackexchange.com/a/{}", a.answer_id), Arc::clone(&user_sandbox)).await;
            }
        }
    }
    
    // PLDI
    
    for q in qs_716.items {
        if q.creation_date * 1000 > down_since - 20000 {
            println!("api: qs_716: {}", q.question_id);
            
            if ids.lock().await.p_716.insert(q.question_id.to_string()) {
                post(146046, format!("https://languagedesign.stackexchange.com/q/{}", q.question_id), Arc::clone(&user_main)).await;
            }
        }
    }
    
    for q in qs_717.items {
        if q.creation_date * 1000 > down_since - 20000 {
            println!("api: qs_717: {}", q.question_id);
            
            if ids.lock().await.p_717.insert(q.question_id.to_string()) {
                post(146046, format!("https://languagedesign.meta.stackexchange.com/q/{}", q.question_id), Arc::clone(&user_main)).await;
            }
        }
    }
    
    Ok(())
}

pub async fn watch_ws(id: usize, ids: Arc<Mutex<Ids>>, users: [Arc<User>; 2], config: Arc<Config>) {
    let mut first = true;
    
    loop {
        post_from_api(time() - 1200000, Arc::clone(&ids), Arc::clone(&users[0]), Arc::clone(&users[1]), Arc::clone(&config)).await.unwrap();
        
        connect_watch_ws(id, Arc::clone(&ids), Arc::clone(&users[0]), Arc::clone(&users[1]), Arc::clone(&config), id == 1 && first).await.unwrap();
        
        first = false;

        tokio::time::sleep(Duration::from_millis(2000)).await;
    }
}
