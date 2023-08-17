use anyhow::anyhow;
use tokio::sync::mpsc::UnboundedSender;

use crate::protocol::{
    server, AsyncMessageReceiver, GameContextKind, GamePhaseKind, MessageReceiver, TryNextContext,
    TurnStatus, Username,
};

type Answer<T> = oneshot::Sender<T>;
use std::net::SocketAddr;

use async_trait::async_trait;
use tokio::sync::oneshot;
use tracing::{debug, error, info, trace};
use crate::{
    game::{Card, Role},
    protocol::{
        client::RoleStatus,
        server::{
            ChatLine, GameMsg, LoginStatus, Msg, PlayerId, RolesMsg, SelectRoleStatus,
            ServerNextContextData, ServerStartGameData, MAX_PLAYER_COUNT,
        },
    },
    server::{
        Handle,
        peer::{IntroHandle, PeerCmd, PeerHandle, RolesHandle},
        details::api,
    },
};

api! {
    impl Handle<ServerCmd> {
        pub async fn ping(&self) -> ();
        pub fn broadcast(&self, sender: SocketAddr, message: server::Msg);
        pub fn append_chat(&self, line: server::ChatLine);
        pub fn request_next_context_after(&self, sender: SocketAddr, current: GameContextKind);
        pub fn select_role(&self, sender: SocketAddr, role: Role);
        pub async fn get_chat_log(&self) -> Vec<server::ChatLine>;
        pub async fn add_player(&self, addr: SocketAddr, username: String, peer: PeerHandle) -> LoginStatus;
        pub async fn shutdown(&self) -> ();
        pub async fn is_peer_connected(&self, who: PlayerId) -> bool ;
        pub fn drop_peer(&self, whom: PlayerId) ;
        pub async fn get_peer_handle(&self, whom: PlayerId) -> PeerHandle ;
        pub async fn get_peer_username(&self, whom: PlayerId) -> Username ;
        pub async fn broadcast_to_all(&self, msg: server::Msg) -> () ;
        pub async fn get_available_roles(&self) -> [RoleStatus; Role::count()] ;
        pub async fn send_to_player(&self, player: PlayerId, msg: server::Msg) -> () ;
        pub async fn get_next_player_after(&self, player: PlayerId) -> PlayerId ;
        pub async fn get_all_peers(&self) -> [PlayerId; MAX_PLAYER_COUNT] ;
        pub fn broadcast_game_state(&self, sender: PlayerId);
    }

}

pub type ServerHandle = Handle<ServerCmd>;


impl ServerHandle {
    pub fn for_tx(tx: UnboundedSender<ServerCmd>) -> Self {
        ServerHandle { tx }
    }
}

// server actor
#[derive(Default)]
pub struct Server {}
#[async_trait]
impl<'a> AsyncMessageReceiver<ServerCmd, &'a mut Room> for Server {
    async fn reduce(&mut self, msg: ServerCmd, room: &'a mut Room) -> anyhow::Result<()> {
        match msg {
            ServerCmd::Shutdown(to) => {
                info!("Shutting down the server...");
                room.shutdown();
                let _ = to.send(());
            }
            ServerCmd::Ping(to) => {
                debug!("Pong from the server actor");
                let _ = to.send(());
            }
            ServerCmd::Broadcast(sender, message) => room.broadcast(sender, message).await,
            ServerCmd::BroadcastToAll(msg, tx) => { 
                room.broadcast_to_all(msg).await;
                let _ = tx.send(());
            },

            ServerCmd::BroadcastGameState(sender) => {
                room.peer_iter().filter(|p| p.addr != sender).for_each(|p| {
                    let _ =
                        p.peer
                            .tx
                            .send(PeerCmd::ContextCmd(crate::server::peer::ContextCmd::from(
                                crate::server::peer::GameCmd::UpdateClientState,
                            )));
                });
            }
            ServerCmd::SendToPlayer(player, msg, tx) => {
                room.get_peer(player)?.peer.send_tcp(msg);
                let _ = tx.send(());

            }
            ServerCmd::AddPlayer(addr, username, peer_handle, tx) => {
                let _ = tx.send(room.add_player(addr, username, peer_handle).await);
            }
            ServerCmd::GetPeerHandle(addr, to) => {
                let _ = to.send(room.get_peer(addr)?.peer.clone());
            }
            ServerCmd::GetAllPeers(tx) => {
                let mut iter = room.peer_iter();
                let _ = tx.send(core::array::from_fn(|_| {
                    iter.next().expect("Must == PLAYER_COUNT").addr
                }));
            }
            ServerCmd::GetNextPlayerAfter(player, tx) => {
                let _ = tx.send(room.next_player_for_turn(player).addr);
            }
            ServerCmd::GetPeerUsername(addr, to) => {
                // server internal error?
                let _ = to.send(
                    room.get_peer(addr)
                        .expect("Must exists")
                        .peer
                        .get_username()
                        .await,
                );
            }
            ServerCmd::DropPeer(addr) => {
                if let Ok(p) = room.get_peer(addr) {
                    trace!("Drop a peer handle {}", addr);
                    room.broadcast_to_all(server::Msg::from(server::AppMsg::Chat(
                        ChatLine::Disconnection(p.peer.get_username().await),
                    )))
                    .await;
                    room.drop_peer_handle(addr)
                        .await
                        .expect("Must drop if peer is logged");
                }
            }
            ServerCmd::AppendChat(line) => {
                room.chat.push(line);
            }
            ServerCmd::GetChatLog(tx) => {
                let _ = tx.send(room.chat.clone());
            }
            ServerCmd::SelectRole(addr, role) => {
                let status = room.set_role_for_peer(addr, role).await;
                let peer = room.get_peer(addr).expect("Must exists");
                if let SelectRoleStatus::Ok(r) = &status {
                    room.broadcast(
                        addr,
                        server::Msg::from(server::AppMsg::Chat(server::ChatLine::GameEvent(
                            format!("{} select {:?}", peer.peer.get_username().await, r),
                        ))),
                    )
                    .await;
                };
                peer.peer
                    .send_tcp(server::Msg::from(RolesMsg::SelectedStatus(status)));
                let roles = room.collect_roles().await;
                debug!("Peer roles {:?}", roles);
                room.broadcast_to_all(server::Msg::from(server::RolesMsg::AvailableRoles(roles)))
                    .await;
            }
            ServerCmd::GetAvailableRoles(to) => {
                let _ = to.send(room.collect_roles().await);
            }
            ServerCmd::IsPeerConnected(addr, to) => {
                let _ = to.send(
                    room.peer_iter()
                        .any(|p| p.addr == addr && p.status == PeerStatus::Connected),
                );
            }
            ServerCmd::RequestNextContextAfter(addr, current) => {
                info!(
                    "A next context was requested by peer {} 
                      (current: {:?})",
                    addr, current
                );
                let p = &room
                    .get_peer(addr)
                    .expect("failed to find the peer in the world storage");
                use GameContextKind as Id;
                let next = GameContextKind::try_next_context(current)?;
                match next {
                    Id::Intro(_) => {
                        p.peer.next_context(ServerNextContextData::Intro(())).await;
                    }
                    Id::Home(_) => {
                        p.peer.next_context(ServerNextContextData::Home(())).await;
                        info!("Player {} was connected to the game", addr);
                        room.broadcast_to_all(server::Msg::from(server::AppMsg::Chat(
                            ChatLine::Connection(p.peer.get_username().await),
                        )))
                        .await;
                    }
                    Id::Roles(_) => {
                        if room.is_full() && {
                            // check for the same context
                            let mut all_have_same_ctx = true;
                            for other in room.peer_iter().filter(|peer| addr != peer.addr) {
                                all_have_same_ctx &= other.peer.get_context_id().await == current;
                                if !all_have_same_ctx {
                                    break;
                                };
                            }
                            all_have_same_ctx
                        } {
                            info!("A game ready to start: next context 'Roles'");
                            for p in room.peer_iter() {
                                p.peer.next_context(ServerNextContextData::Roles(())).await;
                            }
                        } else {
                            info!("Attempt to start a game. A game does not ready to start..")
                        }
                    }
                    Id::Game(_) => {
                        for p in room.peer_iter() {
                            assert!(
                                matches!(p.peer.get_context_id().await, GameContextKind::Roles(())),
                                "All players must have 'GameContextKind::Roles' context"
                            );
                        }
                        if room.are_all_have_roles().await {
                            let mut iter = room.peer_iter();
                            let session = RolesHandle(&p.peer)
                                .spawn_session(core::array::from_fn(|_| {
                                    iter.next().expect("Must == PLAYER_COUNT").addr
                                }))
                                .await;

                            for p in room.peer_iter() {
                                p.peer
                                    .next_context(ServerNextContextData::Game(
                                        ServerStartGameData {
                                            session: session.clone(),
                                            monsters: session.get_monsters().await,
                                        },
                                    ))
                                    .await;
                            }
                            let active = session.get_active_player().await;
                            room.get_peer(active)
                                .expect("")
                                .peer
                                .send_tcp(server::Msg::from(GameMsg::Turn(TurnStatus::Ready(
                                    session.get_game_phase().await,
                                ))));
                            room.broadcast(active, Msg::from(GameMsg::Turn(TurnStatus::Wait)))
                                .await;
                        }
                    }
                };
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerStatus {
    Connected,
    WaitReconnection,
}

pub struct PeerSlot {
    addr: SocketAddr,
    status: PeerStatus,
    peer: PeerHandle,
}
impl PeerSlot {
    pub fn new(addr: SocketAddr, peer: PeerHandle) -> Self {
        PeerSlot {
            addr,
            status: PeerStatus::Connected,
            peer,
        }
    }
}

// server state for 2 players
#[derive(Default)]
pub struct Room {
    peers: [Option<PeerSlot>; MAX_PLAYER_COUNT],
    chat: Vec<server::ChatLine>,
}

impl Room {
    fn peer_iter(&self) -> impl Iterator<Item = &PeerSlot> {
        self.peers.iter().filter_map(|p| p.as_ref())
    }
    fn peer_iter_mut(&mut self) -> impl Iterator<Item = &mut PeerSlot> {
        self.peers.iter_mut().filter_map(|p| p.as_mut())
    }
    fn get_peer(&self, addr: SocketAddr) -> anyhow::Result<&PeerSlot> {
        self.peer_iter()
            .find(|p| p.addr == addr)
            .ok_or(anyhow!("peer with addr {} not found", addr))
    }

    fn next_player_for_turn(&self, current: PlayerId) -> &PeerSlot {
        assert!(
            self.peer_iter().count() >= self.peers.len(),
            "Require at least two players"
        );
        let mut i = self
            .peer_iter()
            .position(|i| i.addr == current)
            .expect("Peer must exists");
        while self.peers[i].is_none() || self.peers[i].as_ref().unwrap().addr == current {
            i += 1;
            if i >= self.peers.len() {
                i = 0;
            }
        }
        self.peers[i].as_ref().expect("Now must be Some here")
    }
    fn get_peer_slot_mut(
        &mut self,
        addr: SocketAddr,
    ) -> anyhow::Result<<&mut [Option<PeerSlot>] as IntoIterator>::Item> {
        self.peers
            .iter_mut()
            .filter(|p| p.is_some())
            .find(|p| p.as_ref().unwrap().addr == addr)
            .ok_or(anyhow!("peer with addr {} not found", addr))
    }

    async fn send_message(&self, peer: &PeerHandle, message: server::Msg) {
        // ignore message from other contexts
        let peer_context = peer.get_context_id().await;
        let msg_context = GameContextKind::try_from(&message);
        if matches!(&message, server::Msg::App(_))
            || (peer_context == msg_context.expect("If not Msg::App, then must valid to unwrap"))
        {
            peer.send_tcp(message);
        }
    }

    async fn broadcast(&self, sender: SocketAddr, message: server::Msg) {
        trace!("Broadcast {:?}", message);
        for p in self.peer_iter() {
            if p.addr != sender {
                self.send_message(&p.peer, message.clone()).await;
            }
        }
    }
    async fn broadcast_to_all(&self, message: server::Msg) {
        for p in self.peer_iter() {
            self.send_message(&p.peer, message.clone()).await;
        }
    }

    async fn peer_role(&self, p: &PeerSlot) -> anyhow::Result<Option<Role>> {
        //let ctx_handle  = Into::<ServerGameContextHandle>::into((p.peer.get_context_id().await, &p.peer));
        //let select_role = <&SelectRoleHandle>::try_from(&ctx_handle).map_err(|e| anyhow!(e))?;
        // TODO
        let select_role = RolesHandle(&p.peer);
        Ok(select_role.get_role().await)
    }

    async fn collect_roles(&self) -> [RoleStatus; Role::count()] {
        use std::mem::MaybeUninit;
        // Create an array of uninitialized values.
        let mut roles: [MaybeUninit<RoleStatus>; Role::count()] =
            unsafe { MaybeUninit::uninit().assume_init() };
        trace!("Collect roles from peers");
        'role: for (i, r) in Role::iter().enumerate() {
            for p in self.peer_iter() {
                // this is an internal server arhitecture error
                if self
                    .peer_role(p)
                    .await
                    .expect("Peer Must exists")
                    .is_some_and(|pr| {
                        debug!("Role: {:?}, Role in peer {} = {:?}", r, p.addr, pr);
                        pr == r
                    })
                {
                    debug!("Set Unavailable role");
                    roles[i] = MaybeUninit::new(RoleStatus::NotAvailable(r));
                    continue 'role;
                } else {
                    roles[i] = MaybeUninit::new(RoleStatus::Available(r));
                }
            }
        }
        unsafe { std::mem::transmute::<_, [RoleStatus; Role::count()]>(roles) }
    }

    async fn set_role_for_peer(&self, sender: SocketAddr, role: Role) -> SelectRoleStatus {
        for p in self.peer_iter() {
            let p_role = self.peer_role(p).await.expect("Must be Some");
            if p_role.is_some() && p_role.unwrap() == role {
                return if p.addr != sender {
                    SelectRoleStatus::Busy
                } else {
                    SelectRoleStatus::AlreadySelected
                };
            }
        }
        let p = &self.get_peer(sender).expect("must be exists").peer;

        // TODO
        let select_role = RolesHandle(p);
        select_role.select_role(role).await;

        //<&SelectRoleHandle>::try_from(&Into::<ServerGameContextHandle>::into((p.get_context_id().await, p)))
        //    .map_err(|e| anyhow!(e))
        //   .expect("Unexpected context")
        //   .select_role(&p.tx, role).await;
        SelectRoleStatus::Ok(role)
    }

    fn is_full(&self) -> bool {
        !self.peers.iter().any(|p| p.is_none())
    }
    async fn add_player(
        &mut self,
        sender: SocketAddr,
        username: String,
        mut player: PeerHandle,
    ) -> LoginStatus {
        info!("Try login a player {} as {}", sender, username);
        for p in self.peer_iter_mut() {
            if p.peer.get_username().await == username {
                if p.status == PeerStatus::WaitReconnection {
                    // reconnection here
                    p.addr = sender;
                    p.status = PeerStatus::Connected;
                    return LoginStatus::Reconnected;
                } else {
                    return LoginStatus::AlreadyLogged;
                }
            }
        }
        // new player
        if let Some(it) = self.peers.iter_mut().position(|x| x.is_none()) {
            // TODO
            IntroHandle(&mut player)
                //<&IntroHandle>::try_from(&Into::<ServerGameContextHandle>::into((player.get_context_id().await, &player)))
                //.expect("Must be Intro")
                .set_username(username);
            self.peers[it] = Some(PeerSlot::new(sender, player));
            LoginStatus::Logged
        } else {
            LoginStatus::PlayerLimit
        }
    }
    async fn drop_peer_handle(&mut self, whom: SocketAddr) -> anyhow::Result<()> {
        let p = self.get_peer_slot_mut(whom)?;
        use GameContextKind as Id;
        match p.as_ref().unwrap().peer.get_context_id().await {
            Id::Intro(_) | Id::Home(_) => {
                *p = None;
            }
            _ => {
                // keep connected in the game
                // for reconnection in 'Room::add_player'
                p.as_mut().unwrap().status = PeerStatus::WaitReconnection;
            }
        }
        Ok(())
    }
    fn shutdown(&mut self) {
        trace!("Drop all peers");
        // if it is a last handle, peer actor will shutdown
        for p in self.peers.iter_mut() {
            *p = None;
        }
    }

    // TODO if different ctxts
    async fn are_all_have_roles(&self) -> bool {
        for p in self.peer_iter() {
            // TODO
            //let ctx = Into::<ServerGameContextHandle>::into((p.peer.get_context_id().await, &p.peer));
            //let ctx = <&SelectRoleHandle>::try_from(&ctx);
            let ctx = RolesHandle(&p.peer);
            //if ctx.is_err() || ctx.unwrap().get_role().await.is_none() {
            if ctx.get_role().await.is_none() {
                return false;
            }
        }
        true
    }
}
