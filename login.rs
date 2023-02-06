use std::{error::Error, fmt};
use std::sync::Arc;
use html_parser::{Dom, Node};
use serde::{Serialize, Deserialize};

use crate::{time, Config};

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

fn extract_fkey(html: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
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

fn contains_logout(html: &str) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
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

fn extract_user_id(html: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
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

async fn try_login(client: &reqwest::Client, email: &str, password: &str) -> Result<UserCredentials, Box<dyn std::error::Error + Send + Sync>> {
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
    
    Ok(UserCredentials {
        user_id: user_id,
        fkey: logged_in_fkey
    })
}

#[derive(Serialize, Deserialize)]
struct Credentials {
    time: u128,
    main: UserCredentials,
    sandbox: UserCredentials
}

#[derive(Serialize, Deserialize)]
struct UserCredentials {
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

async fn retrieve_credentials() -> Result<Credentials, Box<dyn std::error::Error + Send + Sync>> {
    let json = tokio::fs::read_to_string("tmp/credentials.json").await?;
    
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

pub async fn login(config: Arc<Config>) -> Result<(User, User), Box<dyn std::error::Error + Send + Sync>> {
    let credentials = retrieve_credentials().await.ok();
    
    let cookie_stores;
    
    if credentials.is_some() {
        cookie_stores = (
            Arc::new(reqwest_cookie_store::CookieStoreMutex::new(reqwest_cookie_store::CookieStore::load_json(std::io::BufReader::new(std::fs::File::open("tmp/cookies-main.json")?))?)),
            Arc::new(reqwest_cookie_store::CookieStoreMutex::new(reqwest_cookie_store::CookieStore::load_json(std::io::BufReader::new(std::fs::File::open("tmp/cookies-sandbox.json")?))?))
        );
    } else {
        cookie_stores = (
            Arc::new(reqwest_cookie_store::CookieStoreMutex::new(reqwest_cookie_store::CookieStore::default())),
            Arc::new(reqwest_cookie_store::CookieStoreMutex::new(reqwest_cookie_store::CookieStore::default()))
        );
    }
    
    let client_main = reqwest::ClientBuilder::new().user_agent("Mozilla/5.0 (compatible; NPSP/2.0; +https://chat.stackexchange.com/rooms/240/the-nineteenth-byte)").cookie_store(true).cookie_provider(Arc::clone(&cookie_stores.0)).gzip(true).build()?;
    let client_sandbox = reqwest::ClientBuilder::new().user_agent("Mozilla/5.0 (compatible; NPSP/2.0; +https://chat.stackexchange.com/rooms/240/the-nineteenth-byte)").cookie_store(true).cookie_provider(Arc::clone(&cookie_stores.1)).gzip(true).build()?;
    
    let fkeys;
    
    if let Some(credentials) = credentials {
        fkeys = (credentials.main.fkey, credentials.sandbox.fkey);
        
        println!("login: found from {} mins ago", (time() - credentials.time) / 60000);
    } else {
        let login_main = try_login(&client_main, &config.np.email, &config.np.password).await?;
        let login_sandbox = try_login(&client_sandbox, &config.sp.email, &config.sp.password).await?;
        
        fkeys = (login_main.fkey.clone(), login_sandbox.fkey.clone());
        
        tokio::fs::create_dir_all("tmp").await?;
        tokio::fs::write("tmp/credentials.json", serde_json::to_string(&Credentials {
            time: time(),
            main: login_main,
            sandbox: login_sandbox
        })?).await?;
        
        cookie_stores.0.lock().unwrap().save_json(&mut std::io::BufWriter::new(std::fs::File::create("tmp/cookies-main.json")?))?;
        cookie_stores.1.lock().unwrap().save_json(&mut std::io::BufWriter::new(std::fs::File::create("tmp/cookies-sandbox.json")?))?;
        
        println!("login: successful");
    }
    
    Ok((
        User {
            client: client_main,
            fkey: fkeys.0
        },
        User {
            client: client_sandbox,
            fkey: fkeys.1
        }
    ))
}
