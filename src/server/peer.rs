use anyhow::Context as _;
use std::net::SocketAddr;
use tokio::sync::mpsc::UnboundedSender;
use tracing::{info, warn, error, trace, debug};
use async_trait::async_trait;
use crate::{ protocol::{
                client,
                server::{self, 
                    Msg,
                    ServerGameContext,
                    ServerNextContextData,
                    LoginStatus,
                    Intro,
                    Home,
                    SelectRole,
                    Game,
                },
                GameContext,
                GameContextId,
                AsyncMessageReceiver,
                MessageError,
                encode_message,
                ToContext,
                Tx,



            },
             server::{Answer, ServerHandle},
             game::Role

};


pub struct Peer {
    pub context: ServerGameContext,
}
impl Peer {
    pub fn new(start_context: ServerGameContext) -> Peer {
        Peer{ context: start_context }
    }
}




macro_rules! nested_contexts {
    (
        pub type $type:ident = GameContext <
        $(
            $( #[$meta:meta] )*
            $vis:vis enum $name:ident {
                $($tt:tt)*
            },
        )*
        >

    ) => {
        $(
            $( #[$meta] )*
            $vis enum $name {
                $($tt)*
            }
        )*

        pub type $type = GameContext <
            $($name,)*

        >;

    }
}
nested_contexts!{
pub type ContextCmd = GameContext <
        #[derive(Debug)]
        pub enum IntroCmd {
            SetUsername(String),

        },
        #[derive(Debug)]
        pub enum HomeCmd {

        },
        #[derive(Debug)]
        pub enum SelectRoleCmd {
            SelectRole(Role),
            GetRole(Answer<Option<Role>>),
        },
        #[derive(Debug)]
        pub enum GameCmd {

        },
    >
}
macro_rules! impl_from_inner_command {
($( $src: ident => $dst_pat: ident $(,)?)+ => $dst: ty) => {
    $(
    impl From<$src> for $dst {
        fn from(src: $src) -> Self {
            Self::$dst_pat(src)
        }
    }
    )*
    };
}
impl_from_inner_command! {
    IntroCmd       => Intro ,
    HomeCmd        => Home,
    SelectRoleCmd  => SelectRole, 
    GameCmd        => Game,
    => ContextCmd
}

#[derive(Debug)]
pub enum ToPeer {
    Send(server::Msg),
    GetAddr(Answer<SocketAddr>),
    GetContextId(Answer<GameContextId>),
    GetUsername(Answer<String>),
    Close(String),
    NextContext(ServerNextContextData), 
    //GetRole(Answer<Option<Role>>),
    GetConnectionStatus(Answer<ConnectionStatus>),
    ContextCmd(ContextCmd),

}
#[derive(Debug, Clone)]
pub struct PeerHandle{
    pub tx: UnboundedSender<ToPeer>,
    //pub context : ServerGameContextHandle,
}

use crate::server::details::fn_send;
use crate::server::details::fn_send_and_wait_responce;
impl PeerHandle {
    pub fn new(tx: UnboundedSender<ToPeer>, context: GameContextId) -> Self {
        PeerHandle{tx}// context: ServerGameContextHandle::from(context)}
    }
   
    fn_send!(
        ToPeer => tx =>
        //close(reason: String);
        send(msg: server::Msg);
        next_context(for_server: ServerNextContextData);
    );
    fn_send_and_wait_responce!(
        ToPeer => tx =>
        get_context_id() -> GameContextId;
        get_username() -> String;
        //get_role() -> Option<Role>;
    );
    
}

#[derive(Debug)]
pub enum ConnectionStatus {
    NotLogged,
    Connected,
    WaitReconnection
}      
pub struct Connection {
    //pub status: ConnectionStatus,
    pub addr     : SocketAddr,
    pub to_socket: Tx,
    pub server: ServerHandle,
}

impl Connection {
    pub fn new(addr: SocketAddr, socket_tx: Tx, world_handle: ServerHandle) -> Self {
        Connection{//status: ConnectionStatus::NotLogged,
         addr, to_socket: socket_tx, server: world_handle}
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.server.drop_player(self.addr);
    }
}


macro_rules! fn_send_to_context {
    ($cmd: expr  => $( $fname: ident(&self, to_peer: $sink_ty: ty, $($vname:ident : $type: ty $(,)?)*); )+) => {
        paste::item! {
            $(pub fn $fname(&self, tx: $sink_ty,  $($vname: $type,)*){
                let _ = tx.send(ToPeer::ContextCmd(ContextCmd::from($cmd::[<$fname:camel>]($($vname, )*))));
            }
            )*
        }
    }
}

macro_rules! fn_send_and_wait_responce_for_context {
    ($cmd: expr =>  $( $fname: ident(&self, to_peer: $sink_ty: ty, $($vname:ident : $type: ty $(,)?)*) -> $ret: ty; )+) => {
        paste::item! {
            $( pub async fn $fname(&self, to_peer: $sink_ty, $($vname: $type,)*) -> $ret {
                let (tx, rx) = tokio::sync::oneshot::channel();
                let _ = to_peer.send(ToPeer::ContextCmd(ContextCmd::from(
                            $cmd::[<$fname:camel>]($($vname, )* tx))));
                rx.await.expect(concat!("failed to process ", stringify!($fname)))
            }
            )*
        }
    }
}
macro_rules! impl_try_from_for_inner {
    ($vis:vis type $name:ident = $ctx: ident < 
        $( $($self_:ident)?:: $vname:ident => $enum_pat:ident , )*
    >;

    ) => {
        $vis type $name  = $ctx <
            $($vname,)*
        >;
        $(
        impl<'a> std::convert::TryFrom<&'a $name> for &'a $vname {
            type Error = String;
            fn try_from(other: &'a $name) -> Result<Self, Self::Error> {
                    match other {
                        $name::$enum_pat(v) => Ok(v),
                        _ => Err(concat!("The game context must be '", stringify!($enum_pat), "'").into()),
                    }
            }
        }
        )*
    }
}
#[derive(Debug, Clone)]
pub struct IntroHandle;
impl IntroHandle{
    fn_send_to_context!{
        IntroCmd => 
            set_username(&self, to_peer: &UnboundedSender<ToPeer>, username: String);
    } 

}
#[derive(Debug, Clone)]
pub struct HomeHandle;
#[derive(Debug, Clone)]
pub struct SelectRoleHandle;
impl SelectRoleHandle{

    fn_send_to_context!{
        SelectRoleCmd => 
            select_role(&self, to_peer: &UnboundedSender<ToPeer>, role: Role);
    } 
    fn_send_and_wait_responce_for_context!{
        SelectRoleCmd => 
            get_role(&self, to_peer: &UnboundedSender<ToPeer>,) -> Option<Role>;
    }
}

#[derive(Debug, Clone)]
pub struct GameHandle;

impl_try_from_for_inner!{
    pub type ServerGameContextHandle = GameContext<
         self::IntroHandle          => Intro, 
         self::HomeHandle           => Home, 
         self::SelectRoleHandle     => SelectRole, 
         self::GameHandle           => Game,
    >;
}


// GameContextIf -> ServerGameContextHandle
use crate::details::impl_from;
impl_from!{ impl From () GameContext<(), (), (), () >  for ServerGameContextHandle {
                       Intro(_)      => Intro(IntroHandle{})
                       Home(_)       => Home(HomeHandle{})
                       SelectRole(_) => SelectRole(SelectRoleHandle{})
                       Game(_)       => Game(GameHandle{})
        }
}




// TODO internal commands by contexts?
#[async_trait]
impl<'a> AsyncMessageReceiver<ToPeer, &'a Connection> for Peer {
    async fn message(&mut self, msg: ToPeer, state:  &'a Connection) -> Result<(), MessageError>{
        match msg {
            ToPeer::Close(reason)  => {
                // TODO thiserror errorkind 
                // //self.world_handle.broadcast(self.addr, server::Msg::(ChatLine::Disconnection(
        //                    self.username)));
            }
            ToPeer::Send(msg) => {
                let _ = state.to_socket.send(encode_message(msg));
            },
            ToPeer::GetAddr(to) => {
                let _ = to.send(state.addr);
            },
            ToPeer::GetContextId(to) => {
                let _ = to.send(GameContextId::from(&self.context));

            },
            ToPeer::GetUsername(to) => { 
                trace!("get username message");
                use ServerGameContext as C;
                let n = match &self.context {
                    C::Intro(i) => i.username.as_ref()
                        .expect("if the server has a peer, this peer must has a username"),
                    C::Home(h) => &h.username,
                    C::SelectRole(r) => &r.username, 
                    C::Game(g) => &g.username,
                };
                let _ = to.send(n.clone());

            },
            ToPeer::NextContext(data_for_next_context) => {
                 let next_ctx_id = GameContextId::from(&data_for_next_context);
                 self.context.to(data_for_next_context, state)
                     .or_else(
                     |e| 
                     Err(MessageError::NextContextRequestError{
                        next: next_ctx_id,
                        current: GameContextId::from(&self.context),
                        reason: e.to_string()
                     })
                     )?;

            },
            ToPeer::ContextCmd(msg) => {
                self.context.message(msg, state).await.unwrap();
            }
            _ => (),
            
        }
       Ok(())
    }
}

#[async_trait]
impl<'a> AsyncMessageReceiver<IntroCmd, &'a Connection> for Intro {
    async fn message(&mut self, msg: IntroCmd, state:  &'a Connection) -> Result<(), MessageError>{
        //use IntroCommand as Cmd;
        match msg {
            IntroCmd::SetUsername(username) => {
                trace!("Set username {} for {}", username, state.addr);
                self.username = Some(username);
            }
        };
        Ok(())
    }
}
#[async_trait]
impl<'a> AsyncMessageReceiver<HomeCmd, &'a Connection> for Home {
    async fn message(&mut self, msg: HomeCmd, state:  &'a Connection) -> Result<(), MessageError>{
        //use HomeCommand as Cmd;
        //match msg {
        //};
        Ok(())
    }
}
#[async_trait]
impl<'a> AsyncMessageReceiver<SelectRoleCmd, &'a Connection> for SelectRole {
    async fn message(&mut self, msg: SelectRoleCmd, state:  &'a Connection) -> Result<(), MessageError>{
        match msg {
            SelectRoleCmd::SelectRole(role) => {
                self.role = Some(role);
            }
            SelectRoleCmd::GetRole(tx) => {
                let _ = tx.send(self.role);
            }
        }
        Ok(())
    }
}
#[async_trait]
impl<'a> AsyncMessageReceiver<GameCmd, &'a Connection> for Game {
    async fn message(&mut self, msg: GameCmd, state:  &'a Connection) -> Result<(), MessageError>{

        Ok(())
    }
}

#[async_trait]
impl<'a> AsyncMessageReceiver<client::Msg, &'a Connection> for PeerHandle {
    async fn message(&mut self, msg: client::Msg, state: &'a Connection)-> Result<(), MessageError>{
         match msg {
            client::Msg::App(e) => {
                match e {
                    client::AppMsg::Logout =>  {
                        let _ = state.to_socket.
                            send(encode_message(server::Msg::App(server::AppMsg::Logout)));
                        info!("Logout");
                        // TODO 
                        //return Err(MessageError::Logout);
                    },
                    client::AppMsg::NextContext => {
                       state.server.request_next_context_after(state.addr    
                                , self.get_context_id().await);
                    },
                }
            },
            _ => {
                let ctx = self.get_context_id().await;
                 Into::<ServerGameContextHandle>::into(ctx)
                     .message(msg, (self.clone(), state)).await
                     .with_context(|| format!("failed to process a message on the server side: 
                        current context {:?}", ctx ))
                    .map_err(|e| MessageError::Unknown(format!("{}", e)))?;
            }
        }
        Ok(())
    }
}


#[async_trait]
impl<'a> AsyncMessageReceiver<client::IntroMsg, (PeerHandle ,&'a Connection)> for IntroHandle {
    async fn message(&mut self, msg: client::IntroMsg,
                     (peer_handle, state):  (PeerHandle ,&'a Connection)) -> Result<(), MessageError>{
        use client::IntroMsg;
        match msg {
            IntroMsg::AddPlayer(username) =>  {
                info!("{} is trying to connect to the game as {}",
                      state.addr , &username); 
                let status = state.server.add_player(state.addr, 
                                        username, 
                                        peer_handle).await;
                trace!("status: {:?}", status);
                let _ = state.to_socket.send(encode_message(Msg::from(
                    server::IntroMsg::LoginStatus(status))));
                if status != LoginStatus::Logged {
                    return Err(MessageError::LoginRejected{
                        reason: format!("{:?}", status)
                    });
                }
            },
            IntroMsg::GetChatLog => {
                // peer_handle.get_login_status().await;
                // TODO check logging?
                //if self.username.is_none() {
                //    warn!("Client not logged but the ChatLog requested");
                //    return Err(MessageError::NotLogged);
                //}
                info!("Send a chat history to the client");
                let _ = state.to_socket.send(encode_message(server::Msg::Intro(
                    server::IntroMsg::ChatLog(state.server.get_chat_log().await))));
            }
        }

        Ok(())
    }
}
#[async_trait]
impl<'a> AsyncMessageReceiver<client::Homemsg, (PeerHandle ,&'a Connection)> for HomeHandle {
    async fn message(&mut self, msg: client::Homemsg, 
                     (peer_handle, state):  (PeerHandle ,&'a Connection))-> Result<(), MessageError>{
        use client::Homemsg;
        info!("message from client for home");
        match msg {
            Homemsg::Chat(msg) => {
                let msg = server::ChatLine::Text(
                    format!("{}: {}", peer_handle.get_username().await , msg));
                state.server.append_chat(msg.clone());
                state.server.broadcast(state.addr, Msg::Home(server::HomeMsg::Chat(msg)));
            },
            _ => (),
        }

        Ok(())
    }
}
#[async_trait]
impl<'a> AsyncMessageReceiver<client::SelectRoleMsg, (PeerHandle ,&'a Connection)> for SelectRoleHandle {
    async fn message(&mut self, msg: client::SelectRoleMsg, 
                     (peer_handle, state):   (PeerHandle ,&'a Connection))-> Result<(), MessageError>{
        use client::SelectRoleMsg;
        match msg {
            SelectRoleMsg::Chat(msg) => {
                let msg = server::ChatLine::Text(
                    format!("{}: {}", peer_handle.get_username().await, msg));
                state.server.append_chat(msg.clone());
                state.server.broadcast(state.addr, server::Msg::SelectRole(server::SelectRoleMsg::Chat(msg)));
            },
            SelectRoleMsg::Select(role) => {
                info!("select role request {:?}", role);
                state.server.select_role(state.addr, role);
            }
        }
        Ok(())
    }
}

#[async_trait]
impl<'a> AsyncMessageReceiver<client::GameMsg, (PeerHandle ,&'a Connection)> for GameHandle {
    async fn message(&mut self, msg: client::GameMsg, 
                     (peer_handle, state):  (PeerHandle ,&'a Connection))-> Result<(), MessageError>{

        use client::GameMsg;
        match msg {
            GameMsg::Chat(msg) => {
                let msg = server::ChatLine::Text(
                    format!("{}: {}", peer_handle.get_username().await , msg));
                state.server.append_chat(msg.clone());
                state.server.broadcast(state.addr, server::Msg::Game(server::GameMsg::Chat(msg)));
            },
        }

        Ok(())
    }
}

