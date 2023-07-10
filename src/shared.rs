use serde_json;
use serde::{Serialize, Deserialize};
use std::io::Error;
use tokio_util::codec::{ LinesCodec, Decoder};
use futures::{ Stream, StreamExt};
use std::io::ErrorKind;


///Messages that Client sends to Server
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ClientMessage {
    AddPlayer(String),
    RemovePlayer,
    Chat(String),
    GetChatLog,
    
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ServerMessage {
    LoginStatus(LoginStatus),
    Chat(ChatLine),
    ChatLog(Vec<ChatLine>),
    Logout,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ChatLine {
    Text(String),
    GameEvent(String),
    Connection(String),
    Disconnection(String),
}


#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub enum LoginStatus {
    Logged,
    InvalidPlayerName,
    AlreadyLogged,
    PlayerLimit,
}

pub struct MessageDecoder<S> {
    pub stream: S
}
// TODO custom error message type
impl<S> MessageDecoder<S>
where S : Stream<Item=Result<<LinesCodec as Decoder>::Item
                           , <LinesCodec as Decoder>::Error>> 
        + StreamExt 
        + Unpin, {
    pub fn new(stream: S) -> Self {
        MessageDecoder { stream } 
    }
    pub async fn next<M>(&mut self) -> Result<M, Error>
    where
        M: for<'b> serde::Deserialize<'b> {
        match self.stream.next().await  {
            Some(msg) => {
                match msg {
                    Ok(msg) => {
                        serde_json::from_str::<M>(&msg)
                        .map_err(
                            |err| Error::new(ErrorKind::InvalidData, format!(
                                "failed to decode a type {} from the socket stream: {}"
                                    , std::any::type_name::<M>(), err))) 
                    },
                    Err(e) => {
                        Err(Error::new(ErrorKind::InvalidData, format!(
                            "an error occurred while processing messages from the socket: {}", e)))
                    }
                }
            },
            None => { // The stream has been exhausted.
                Err(Error::new(ErrorKind::ConnectionAborted, 
                        "received an unknown message. Connection rejected"))
            }
        }
    }

}

pub fn encode_message<M>(message: M) -> String
where M: for<'b> serde::Serialize {
    serde_json::to_string(&message).expect("failed to serialize a message to json")

}

