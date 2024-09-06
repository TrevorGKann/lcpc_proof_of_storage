use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerHost {
    pub server_name: Option<String>,
    pub server_ip: String,
    pub server_port: u16,
}

impl fmt::Display for ServerHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(name) = &self.server_name {
            write!(f, "Server: \"{}\" at {}:{}", name.to_string(), self.server_ip, self.server_port)
        } else {
            write!(f, "Server: {}:{}", self.server_ip, self.server_port)
        }
    }
}