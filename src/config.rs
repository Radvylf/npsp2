use std::{collections::HashMap, fmt::{self, Display, Debug}};
use serde::Deserialize;
use std::error::Error;

pub struct Config {
    inner: UnlinkedConfig,
}

impl Config {
    pub fn get_api_key(&self) -> &str {
        &self.inner.api_key
    }

    pub fn get_sites(&self) -> &HashMap<String, SiteConfig> {
        &self.inner.sites
    }

    pub fn get_users(&self) -> &HashMap<String, UserConfig> {
        &self.inner.users
    }

    pub fn get_watch_sockets(&self) -> &HashMap<String, WatchSocketConfig> {
        &self.inner.watch_sockets
    }

    pub fn get_rooms(&self) -> &HashMap<String, RoomConfig> {
        &self.inner.rooms
    }

    pub fn get_route_configs(&self) -> HashMap<&str, RouteConfig<'_>> {
        self.inner.routes.iter().map(|(id, route)| (id.as_str(), RouteConfig {
            user: self.inner.users.get(&route.user).unwrap(),
            watch_socket: self.inner.watch_sockets.get(&route.watch_socket).unwrap(),
            room: self.inner.rooms.get(&route.room).unwrap(),

            force_user_client_for_watch_socket: route.force_user_client_for_watch_socket,
        })).collect()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnlinkedConfig {
    api_key: String,
    sites: HashMap<String, SiteConfig>,
    users: HashMap<String, UserConfig>,
    watch_sockets: HashMap<String, WatchSocketConfig>,
    rooms: HashMap<String, RoomConfig>,
    routes: HashMap<String, UnlinkedRouteConfig>,
}

impl UnlinkedConfig {
    pub fn link(self) -> Result<Config, ConfigLinkingError> {
        for (id, route) in &self.routes {
            if !self.users.contains_key(&route.user) {
                return Err(ConfigLinkingError {
                    message: format!("missing user `{}` in route `{}`", route.user, id)
                });
            }
        }

        Ok(Config {
            inner: self,
        })
    }
}

pub struct ConfigLinkingError {
    message: String
}

impl Display for ConfigLinkingError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Debug for ConfigLinkingError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Error({})",
            self.message
        )
    }
}

impl Error for ConfigLinkingError {}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteConfig {
    pub id: String,
    pub name: String,
    pub url: String,
    pub websocket_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserConfig {
    pub login_site: String,
    pub email: String,
    pub password: String,
}

#[derive(Deserialize)]
pub struct RoomConfig {
    pub server: String,
    pub id: String,
}

#[derive(Deserialize)]
pub struct WatchSocketConfig {
    pub site: String,
    #[serde(flatten)]
    pub config: WatchSocketConfigType,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum WatchSocketConfigType {
    Questions,
    Answers {
        question_id: String,
    }
}

pub struct RouteConfig<'a> {
    pub user: &'a UserConfig,
    pub watch_socket: &'a WatchSocketConfig,
    pub room: &'a RoomConfig,

    pub force_user_client_for_watch_socket: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnlinkedRouteConfig {
    user: String,
    watch_socket: String,
    room: String,

    #[serde(default)]
    force_user_client_for_watch_socket: bool,
}