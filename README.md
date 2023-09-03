# Kobuleti

## State Machine

### Intro
Intro is a login and handshake state.

#### Sequence Diagram

```mermaid
sequenceDiagram
    actor C as Client::Intro
    box Peer
        participant PC as Peer::Intro(Tcp &<br/>PeerActorHandle)
        participant P as Peer::Intro (Actor)
    end
    box Server
        participant I as Server::Intro
        participant S as GameServer
    end
    C-)+PC: Login(Username)
    PC->>+I: LoginPlayer
    I-->I:  IsPlayerLimit
    I-->I:  IsUsernameExists
    opt If Some(server)
        I->>+S: GetPeerIdByName
        S-->>-I: Option
    
    opt Roles or Game (Reconnection)
        I->>+S:  IsConnected
        S-->>-I: bool<br/>(Login if player offline)
    end
    end

    I-)P: SetUsername<br/>(If Logged)

    I-->>-PC: Loggin Status

    break if login fails
        PC-->>C: AlreadyLogged,<br/> or PlayerLimit
    end


    PC--)-C: Logged
    PC->>+I: GetChatLog
    I->>+S: GetChatLog
    alt If Some(server)
        S-->>I: ChatLog
    else  None
        S-->>-I: EmptyChat
    end
    I-->>-PC: ChatLog
    PC-)C: ChatLog<br/>(ready to show)
    C-->C: Run Tui
    C -)+PC: EnterGame
    # end Intro
    PC->>-I: EnterGame
    alt server None
        I-)S: StartHome(Sender)
    else server Some(Home)
        I->>S: AddPeer(Sender)
    else server Some(Roles|Game)
        I->>+S: GetPeerHandle(Username)
        S-->>-I: OldPeerHandle
    participant PS as OldPeer::Offline<br/>(reconnection)
    I->>+P: Reconnect(Roles|Game)(<br/>ServerHandle, OldPeerHandle)
    P->>+PS: TakePeer
    PS-->>-P: Self
    # destroy Ps
    P--)-I: NewPeerHandle
    P-)C: Reconnect(StartData)
    I-)S: Reconnect(NewPeerHandle)
    S-->S: Peer Status Online
    end
    Note over C, S: Done Intro, Drop peer actor and handle, start new peer actor and handle.
    I-->I: Start Intro loop Again
    
    
```

### Home
Home is a Lobby server. 
#### Sequence Diagram

```mermaid
sequenceDiagram
    actor C as Client::Home
    box Peer
        participant PC as Peer::Home(Tcp &<br/>PeerActorHandle)
        participant P as Peer::Home (Actor)
    end
    participant H as Server::Home
    
    C -)PC: StartRoles
    PC-)+H: StartRoles
    H-->H: Server if full?
    alt If Server is full, start Roles
    # cancel H
    H->H: Cancel
    participant R as Server::Roles
    H->>R: Start Roles::from(Home)
    loop Each peer (Force start for all)
        R->>+P: StartRoles(ServerHandle)
        P->P:  Cancel, Start Peer::Roles
        Note right of P: Now Peer::Roles
        P-->>-R: New PeerHandle
        P-)C: StartRoles
    end
    C-->R: Run Roles
    end

```

### Roles

#### Sequence Diagram

```mermaid
sequenceDiagram
    actor C as Client::Roles
    box Peer
        participant PC as Peer::Roles(Tcp &<br/> PeerActorHandle)
        participant P as Peer::Roles (Actor)
    end
    participant R as Server::Roles

    C -)+PC: SelectRole
    PC -)+R: SelectRole
    loop except sender,<br/> until Role==Role
        R->>+P: GetRole
        P->>-R: Role
        
    end
    alt if Role is available
        R->>+P: SetRole 
    end
    R-)PC: SendTcp(SelectedStatus)<br/>Busy, AlreadySelected
    PC-)-C: SelectedStatus
    loop Broadcast
        R-)P: SendTcp(AvailableRoles)
        P-)C: AvailableRoles
    end
    C-)+PC: StartGame
    PC-)R: StartGame
    R-->R: Are all have roles?
        alt If all have roles, start Game
    Note over PC, R: ... The same as in Home->Roles
    R->R: Cancel
    participant G as Server::Game
    R->>G: Start Game::from(Roles)
    loop Each peer (Force start for all)
        G->>+P: StartGame(ServerHandle)
        P->P:  Cancel, Start Peer::Game
        Note right of P: Now Peer::Game
        P-->>-G: New PeerHandle
        P-)+G: GetMonsters

    end
    C-->G: Ready Server::Game
    loop respond for async 'GetMonsters'
        G--)-P: MonstersForStart
        P-)C: StartGame(Data)
    end
    C-->+C: End Roles.<br/> Stop socket reader
    C-->-C: Start Game.<br/> Start socket reader
    par to active player
        G-)+P: SendTcp(Ready(DropAbility))
        P-)-C: Ready(DropAbility)
    and to other
        G-)+P: SendTcp(Wait)
        P-)-C: Wait
    end
    
    C-->G: Run Game
    end
```

### Game

#### Sequence Diagram


```mermaid
sequenceDiagram
    actor C as Client::Game
    box Peer
        participant PC as Peer::Game(Tcp &<br/> PeerActorHandle)
        participant P as Peer::Game (Actor)
    end
    participant G as Server::Game
    loop
        C-)+PC: DropAbility(ability)
        PC->>-P: DropAbility(ability)
        break ActivePlayer != Self
            PC-)C: TurnStatus::Err(Username)
        end
        P->>G: SwitchToNextPlayer
        G-->G: set next player<br/> and Phase
        par to active player
            G-)+P: Ready(SelectAbility))
            P-)-C: TurnStatus(Ready(SelectAbility))
        and to other
            G-)+P: Wait
            P-)-C: TurnStatus(Wait)
        end
        PC-)+G: BroadcastGameState
        loop expect sender
            G->>-P: SyncWithClient 
        end
        PC->>+P:  SyncWithClient
        P-)-C: UpdateGameData(Data)

        C-)+PC: SelectAbility(ability)
        PC->>-P: SelectAbility(ability)
        P->>G: SwitchToNextPlayer 
         Note over C, G: ... The same switch to next phase and player
        Note over C, G: ... TODO: Game over
    end
    
```