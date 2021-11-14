use serde::Deserialize;

#[derive(Deserialize, Clone)]
pub struct Config {
    pub auth: Option<Auth>,
}

#[derive(Deserialize, Clone)]
pub struct Auth {
    pub ca_cert_file: String,
    pub cert_file: String,
    pub key_file: String,
}
