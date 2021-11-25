/// A single enum for all the errors that can hapeen in `mquictt`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An IO error occured, happens when creating a socket or reading a file.
    #[error("Io Error : {0}")]
    Io(#[from] std::io::Error),
    /// QUIC endpoint error.
    #[error("Endpoint Error : {0}")]
    Endpoint(#[from] quinn::EndpointError),
    /// QUIC connection error, happens while reading/writing to connection. Different from
    /// [`Connect`] which happens when trying to connect for the first time.
    ///
    /// [`Connect`]: Error::Connect
    #[error("Server Connect Error : {0}")]
    Connection(#[from] quinn::ConnectionError),
    /// QUIC error when connecting to the server.
    #[error("Client Connect Error : {0}")]
    Connect(#[from] quinn::ConnectError),
    /// QUIC error when writing to stream.
    #[error("Write Error : {0}")]
    /// QUIC error when reading from stream.
    Write(#[from] quinn::WriteError),
    #[error("Read Error : {0}")]
    Read(#[from] quinn::ReadError),
    /// Connection was abruptly broken.
    #[error("Connection broken")]
    ConnectionBroken,
    /// MQTT packet parsing error.
    #[error("MQTT Error : {0}")]
    MQTT(mqttbytes::Error),
    /// AUthentication error.
    #[error("Rustls Error : {0}")]
    Rustls(#[from] rustls::TLSError),
    /// Missing certificate.
    #[error("Missing Tls Certificate Error")]
    MissingCertificate,
    // Unable to send publish packets to the subscriber.
    #[error("Pub Notif Tx Error : {0}")]
    PubNotifTx(#[from] flume::TrySendError<()>),
    // Unable to recv publish packets from the publisher.
    #[error("Pub Notif Recv Error : {0}")]
    PubNotifRx(#[from] flume::RecvError),
    /// Unable to parse the config.
    #[error("Config Parse Error : {0}")]
    ConfigParse(#[from] serde_json::Error),
    /// Error when interacting with database
    #[error("DB Error : {0}")]
    Sled(#[from] sled::Error)
}
