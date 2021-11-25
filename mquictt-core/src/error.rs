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
    Write(#[from] quinn::WriteError),
    /// QUIC error when reading from stream.
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
    #[error("Sub Request Tx Error : {0}")]
    PubDataTx(#[from] flume::SendError<bytes::Bytes>),
    // Unable to recv publish packets from the publisher.
    #[error("Pub Data Recv Error : {0}")]
    PubDataRx(#[from] flume::RecvError),
    /// Unable to register the subsriber with the corresponding publisher.
    #[error("Sub Request Tx Error : {0}")]
    SubReqTx(#[from] flume::SendError<flume::Sender<bytes::Bytes>>),
    /// Unable to receive subscription requests at the publisher end.
    #[error("Sub Request Tx Error : {0}")]
    SubReqRx(flume::RecvError),
    /// Unable to parse the config.
    #[error("Config Parse Error : {0}")]
    ConfigParse(#[from] serde_json::Error),
}
