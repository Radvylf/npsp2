use std::{error::Error, fmt};
use std::sync::Arc;
use html_parser::{Dom, Node};
use serde::{Serialize, Deserialize};

use crate::config::UserConfig;
use crate::time;

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Debug)]
struct MissingFkey {}

impl Error for MissingFkey {}

impl fmt::Display for MissingFkey {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Missing fkey")
    }
}

#[derive(Debug)]
struct LoginError {
    description: String
}

impl Error for LoginError {}

impl fmt::Display for LoginError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Failed to log in; {}", self.description)
    }
}

#[derive(Debug)]
struct MissingUserId {}

impl Error for MissingUserId {}

impl fmt::Display for MissingUserId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Missing user ID")
    }
}

fn extract_fkey(html: &str) -> Result<String> {
    fn search_node(node: &Node) -> Option<String> {
        match node {
            Node::Element(element) => {
                if element.name == "input" && element.attributes.get("name").map_or(false, |attr| attr.as_ref().map_or(false, |name| name == "fkey")) {
                    element.attributes.get("value").map(|value| value.clone().unwrap_or("".to_owned()))
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
    
    let dom = Dom::parse(html)?;
    
    for child in dom.children {
        let result = search_node(&child);

        if let Some(fkey) = result {
            return Ok(fkey);
        }
    }

    Err(Box::new(MissingFkey {}))
}

fn contains_logout(html: &str) -> Result<bool> {
    fn search_node(node: &Node) -> bool {
        match node {
            Node::Element(element) => {
                if element.name == "a" && element.attributes.get("href").map_or(false, |attr| attr.as_ref().map_or(false, |href| href.ends_with("logout"))) {
                    true
                } else {
                    element.children.iter().any(search_node)
                }
            }
            _ => false
        }
    }
    
    let dom = Dom::parse(html)?;
    
    Ok(dom.children.iter().any(search_node))
}

fn extract_user_id(html: &str) -> Result<String> {
    fn search_node(node: &Node) -> Option<String> {
        match node {
            Node::Element(element) => {
                if element.name == "a" && element.attributes.get("href").map_or(false, |attr| attr.as_ref().map_or(false, |href| href.starts_with("/users/"))) {
                    Some(element.attributes.get("href").unwrap().as_ref().unwrap()[7..].split('/').next().unwrap().to_owned())
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
    
    let dom = Dom::parse(html)?;
    
    for child in &dom.children {
        let result = search_node(&child);

        if let Some(user_id) = result {
            return Ok(user_id);
        }
    }

    Err(Box::new(MissingUserId {})) // research
}

async fn try_login(client: &reqwest::Client, email: &str, password: &str) -> Result<Credentials> {
    let fkey = extract_fkey(&client.get("https://codegolf.stackexchange.com/users/login").send().await?.error_for_status()?.text().await?)?;
    
    let is_login_ok = client.post("https://codegolf.stackexchange.com/users/login-or-signup/validation/track").form(&[
        ("email", email),
        ("password", password),
        ("isSignup", "false"),
        ("isLogin", "true"),
        ("isPassword", "false"),
        ("isAddLogin", "false"),
        ("hasCaptcha", "false"),
        ("ssrc", "head"),
        ("submitButton", "Log in"),
        ("fkey", &fkey)
    ]).send().await?.error_for_status()?.text().await?;
    
    if is_login_ok != "Login-OK" {
        dbg!(is_login_ok);
        
        return Err(Box::new(LoginError {
            description: "No 'Login-OK'".to_owned()
        }));
    }
    
    let login_two = client.post("https://codegolf.stackexchange.com/users/login?ssrc=head&returnurl=https%3a%2f%2fcodegolf.stackexchange.com%2f").form(&[
        ("email", email),
        ("password", password),
        ("ssrc", "head"),
        ("fkey", &fkey)
    ]).send().await?.error_for_status()?.text().await?;
    
    if !contains_logout(&login_two)? {
        dbg!(login_two);
        
        return Err(Box::new(LoginError {
            description: "No 'logout'; possibly CAPTCHA'd".to_owned()
        }));
    }
    
    // client.post("https://codegolf.stackexchange.com/users/login/universal/request").send().await?.error_for_status()?;
    
    let user = client.get("https://chat.stackexchange.com/chats/join/favorite").send().await?.error_for_status()?.text().await?;
    
    let user_id = extract_user_id(&user)?;
    let logged_in_fkey = extract_fkey(&user)?;
    
    Ok(Credentials {
        time: time(),
        user_id: user_id,
        fkey: logged_in_fkey
    })
}

#[derive(Serialize, Deserialize)]
struct Credentials {
    time: u128,
    user_id: String,
    fkey: String
}

#[derive(Debug)]
struct OutdatedCredentials {}

impl Error for OutdatedCredentials {}

impl fmt::Display for OutdatedCredentials {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Outdated credentials")
    }
}

async fn retrieve_credentials(user_id: &str) -> Result<Credentials> {
    let json = tokio::fs::read_to_string(format!("tmp/{}-credentials.json", user_id)).await?;
    
    let credentials: Credentials = serde_json::from_str(&json)?;
    
    let time = time();
    
    if time < credentials.time || time - credentials.time > 7200000 {
        return Err(Box::new(OutdatedCredentials {}));
    }
    
    return Ok(credentials);
}

pub struct User {
    pub client: reqwest::Client,
    pub fkey: String
}

pub async fn log_in(user_id: &str, user_config: &UserConfig) -> Result<User> {
    let credentials = retrieve_credentials(user_id).await.ok();

    let cookie_store = if credentials.is_some() {
        Arc::new(reqwest_cookie_store::CookieStoreMutex::new(reqwest_cookie_store::CookieStore::load_json(std::io::BufReader::new(std::fs::File::open(format!("tmp/{}-cookies.json", user_id))?))?))
    } else {
        Arc::new(reqwest_cookie_store::CookieStoreMutex::new(reqwest_cookie_store::CookieStore::default()))
    };

    let client = reqwest::ClientBuilder::new().user_agent("Mozilla/5.0 (compatible; NPSP/2.0; +https://chat.stackexchange.com/rooms/240/the-nineteenth-byte)").cookie_store(true).cookie_provider(Arc::clone(&cookie_store)).gzip(true).build()?;

    let fkey;

    if let Some(credentials) = credentials {
        fkey = credentials.fkey;
        
        println!("login: found from {} mins ago", (time() - credentials.time) / 60000);
    } else {
        let login = try_login(&client, &user_config.email, &user_config.password).await?;
        
        tokio::fs::create_dir_all("tmp").await?;
        tokio::fs::write(format!("tmp/{}-credentials.json", user_id), serde_json::to_string(&login)?).await?;
        
        fkey = login.fkey;
        
        cookie_store.lock().unwrap().save_json(&mut std::io::BufWriter::new(std::fs::File::create(format!("tmp/{}-cookies.json", user_id))?))?;
        
        println!("login: successful");
    }
    
    Ok(User {
        client: client,
        fkey: fkey
    })
}