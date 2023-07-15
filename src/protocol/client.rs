use anyhow::anyhow;
use crate::client::Start;

use crate::protocol::{To, server, GameContextId, MessageReceiver};
use enum_dispatch::enum_dispatch;
use crate::client::Chat;
use crate::ui::terminal::TerminalHandle;
use std::sync::{Arc, Mutex};
use super::details::unwrap_enum;
type Tx = tokio::sync::mpsc::UnboundedSender<String>;

use serde::{Serialize, Deserialize};

pub struct App {
    pub tx: Tx,
    pub terminal: Arc<Mutex<TerminalHandle>>,
}

pub struct Intro{
    pub username: String,
    pub tx: Tx,
    pub _terminal: Option<Arc<Mutex<TerminalHandle>>>,
}


pub struct Home{
    pub app:  App,
    pub chat: Chat,
}


pub struct Game{
    pub app: App,
    pub chat: Chat,
}


#[enum_dispatch]
pub enum GameContext {
    Intro ,
    Home ,
    Game ,
}

impl GameContext {
    pub fn new(username: String, tx: Tx) -> Self {
        GameContext::from(Intro{username, tx, _terminal: None})
    }
}

impl To for GameContext {
    fn to(& mut self, next: GameContextId) -> &mut Self{
         take_mut::take(self, |s| {
            use GameContextId as Id;
            use GameContext as C;
             match s {
                GameContext::Intro(mut i) => {
                    match next {
                        Id::Intro => C::Intro(i),
                        Id::Home => {
                            C::from(Home{
                                app: App{tx: i.tx, terminal: i._terminal.take().unwrap()}, chat: Chat::default()})
                        },
                        Id::Game => { todo!() }
                    }
                },
                C::Home(h) => {
                     match next {
                        Id::Intro => unimplemented!(),
                        Id::Home =>  C::Home(h),
                        Id::Game => { 
                            C::from(Game{
                                app: h.app, chat: h.chat})
                        },
                    }
                },
                C::Game(_) => {
                        todo!()
                },

            }
         });
         self
    }
}



structstruck::strike! {
#[strikethrough[derive(Deserialize, Serialize, Clone, Debug)]]
pub enum Msg {
    Intro(
        pub enum IntroEvent {
            AddPlayer(String)
        }
    ),
    Home(
        pub enum HomeEvent {
            GetChatLog,
            Chat(String),
            StartGame
        }
    ),
    Game(
        pub enum GameEvent {
            Chat(String)
        }
    ),
    App(
        pub enum AppEvent {
            Logout,
            NextContext
        }
    )
} }




