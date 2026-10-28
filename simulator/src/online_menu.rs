//! The start menu's online screens: sign in, room list, host a room, and the
//! panel shown while playing in someone else's room.
use crate::bridge::{Bridge, RoomSpec, RoomStatus};
use crate::lobby::{MatchAction, Phase as MatchPhase, RoomMeta, Team};
use crate::menu::{RoverStep, button, label};
use crate::online::{
    JoinSpec, LOCAL_ENDPOINT, LobbyClient, LobbyStatus, OnlineSettings, Relay, RelayStatus,
    RoomInfo,
};
use crate::session::{Phase, RebuildWorld, SimSession};
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Screen {
    #[default]
    Off,
    SignIn,
    Rooms,
    Host,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Server,
    User,
    Password,
    Name,
    RoomName,
    RoomPassword,
    JoinPassword,
}

impl Field {
    fn secret(self) -> bool {
        matches!(
            self,
            Self::Password | Self::RoomPassword | Self::JoinPassword
        )
    }
}

/// Everything the online screens remember between frames.
#[derive(Resource)]
pub struct OnlineMenu {
    pub screen: Screen,
    pub settings: OnlineSettings,
    pub room_name: String,
    pub room_password: String,
    pub join_password: String,
    pub team: Team,
    pub focus: Option<Field>,
    pub error: Option<String>,
    pub selected: Option<String>,
    pub lobby: Option<LobbyClient>,
    pub relay: Option<Relay>,
    /// The room this machine hosts, while it does.
    pub hosting: Option<RoomMeta>,
    /// The relay got a seat at least once, so a later failure is a lost link.
    joined: bool,
    /// Bumped whenever the online UI should be rebuilt.
    pub epoch: u32,
    /// What the last rebuild showed of the network state.
    seen: String,
}

impl Default for OnlineMenu {
    fn default() -> Self {
        let settings = OnlineSettings::load();
        Self {
            room_name: format!("{}'s room", settings.name),
            settings,
            screen: Screen::Off,
            room_password: String::new(),
            join_password: String::new(),
            team: Team::Red,
            focus: None,
            error: None,
            selected: None,
            lobby: None,
            relay: None,
            hosting: None,
            joined: false,
            epoch: 1,
            seen: String::new(),
        }
    }
}

impl OnlineMenu {
    fn field_mut(&mut self, field: Field) -> &mut String {
        match field {
            Field::Server => &mut self.settings.server,
            Field::User => &mut self.settings.user,
            Field::Password => &mut self.settings.password,
            Field::Name => &mut self.settings.name,
            Field::RoomName => &mut self.room_name,
            Field::RoomPassword => &mut self.room_password,
            Field::JoinPassword => &mut self.join_password,
        }
    }

    fn field(&self, field: Field) -> &str {
        match field {
            Field::Server => &self.settings.server,
            Field::User => &self.settings.user,
            Field::Password => &self.settings.password,
            Field::Name => &self.settings.name,
            Field::RoomName => &self.room_name,
            Field::RoomPassword => &self.room_password,
            Field::JoinPassword => &self.join_password,
        }
    }

    fn fields(&self) -> &'static [Field] {
        match self.screen {
            Screen::SignIn => &[Field::Server, Field::User, Field::Password, Field::Name],
            Screen::Rooms => &[Field::JoinPassword],
            Screen::Host => &[Field::RoomName, Field::RoomPassword],
            Screen::Off => &[],
        }
    }

    fn selected_room(&self) -> Option<RoomInfo> {
        let code = self.selected.as_ref()?;
        self.lobby
            .as_ref()?
            .rooms()
            .into_iter()
            .find(|room| &room.code == code)
    }

    fn bump(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
    }
}

#[derive(Component, Clone, Debug, PartialEq)]
pub enum OnlineButton {
    Open,
    Back,
    Connect,
    ShowHost,
    CreateRoom,
    Join,
    Leave,
    Select(String),
    Team(Team),
    Focus(Field),
    StartMatch,
    ResetMatch,
    CloseRoom,
}

pub fn install(app: &mut App) {
    if !app.world().contains_resource::<Messages<KeyboardInput>>() {
        app.add_message::<KeyboardInput>();
    }
    app.init_resource::<OnlineMenu>();
}

/// A relay started from the environment, without the menu.
#[derive(Resource)]
struct HeadlessRelay(Relay, String);

/// `ZORVANE_JOIN=<code>` joins a room at startup instead of simulating, for a
/// machine that only runs an AI agent or ARGOS. Uses `ZORVANE_SERVER`,
/// `ZORVANE_TEAM` (red or blue) and `ZORVANE_ROOM_PASSWORD`.
pub fn install_headless_join(app: &mut App) {
    let env = |key: &str| {
        std::env::var(key)
            .ok()
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    };
    let Some(code) = env("ZORVANE_JOIN").map(|code| code.to_ascii_uppercase()) else {
        return;
    };
    let team = match env("ZORVANE_TEAM").as_deref() {
        Some("blue") => Team::Blue,
        _ => Team::Red,
    };
    let spec = JoinSpec {
        settings: OnlineSettings::load(),
        room: RoomInfo {
            prefix: crate::lobby::room_prefix(&code),
            code,
            name: String::new(),
            host: String::new(),
            password: false,
            phase: String::new(),
            rovers: 0,
            free: Default::default(),
            seats: Vec::new(),
        },
        room_password: env("ZORVANE_ROOM_PASSWORD"),
        team,
    };
    if let Some(mut fleet) = app
        .world_mut()
        .get_resource_mut::<crate::terra::RoverFleet>()
    {
        let _ = fleet.set_count(0);
    }
    app.add_systems(
        Startup,
        move |mut commands: Commands, bridge: Option<Res<Bridge>>| {
            if let Some(bridge) = bridge {
                bridge.suspend();
            }
            info!(
                "Joining room {} on {}",
                spec.room.code, spec.settings.server
            );
            commands.insert_resource(HeadlessRelay(Relay::start(spec.clone()), String::new()));
        },
    )
    .add_systems(Update, |mut relay: ResMut<HeadlessRelay>| {
        let status = format!("{:?}", relay.0.status());
        if status != relay.1 {
            info!("Room: {status}");
            relay.1 = status;
        }
    });
}

/// Button presses on every online screen and the room chip.
#[allow(clippy::too_many_arguments)]
pub fn online_clicks(
    mut commands: Commands,
    mut menu: ResMut<OnlineMenu>,
    mut session: ResMut<SimSession>,
    bridge: Option<Res<Bridge>>,
    match_settings: Option<Res<crate::settings::MenuSettings>>,
    mut lighting: Option<ResMut<crate::mason_field::Lighting>>,
    buttons: Query<(&Interaction, &OnlineButton), Changed<Interaction>>,
) {
    let pressed: Vec<OnlineButton> = buttons
        .iter()
        .filter(|(interaction, _)| **interaction == Interaction::Pressed)
        .map(|(_, button)| button.clone())
        .collect();
    for action in pressed {
        menu.error = None;
        match action {
            OnlineButton::Open => {
                menu.screen = if menu.lobby.is_some() {
                    Screen::Rooms
                } else {
                    Screen::SignIn
                };
                menu.focus = Some(if menu.settings.password.is_empty() {
                    Field::Password
                } else {
                    Field::Name
                });
            }
            OnlineButton::Back => {
                menu.focus = None;
                menu.screen = match menu.screen {
                    Screen::Host => Screen::Rooms,
                    Screen::Rooms => {
                        menu.lobby = None;
                        Screen::SignIn
                    }
                    _ => Screen::Off,
                };
            }
            OnlineButton::Connect => match menu.settings.validate() {
                Ok(()) => {
                    menu.settings.save();
                    menu.lobby = Some(LobbyClient::connect(menu.settings.clone()));
                    menu.screen = Screen::Rooms;
                    menu.focus = None;
                }
                Err(error) => menu.error = Some(error),
            },
            OnlineButton::ShowHost => {
                menu.screen = Screen::Host;
                menu.focus = Some(Field::RoomName);
            }
            OnlineButton::Select(code) => {
                menu.selected = Some(code);
                menu.join_password.clear();
                menu.focus = Some(Field::JoinPassword);
            }
            OnlineButton::Team(team) => menu.team = team,
            OnlineButton::Focus(field) => menu.focus = Some(field),
            OnlineButton::CreateRoom => {
                let Some(bridge) = bridge.as_ref() else {
                    menu.error = Some("Zenoh is off (TERRA_ZENOH=0); hosting needs it".into());
                    continue;
                };
                let name = menu.room_name.trim().to_owned();
                if name.is_empty() || name.len() > 40 {
                    menu.error = Some("Room name: 1 to 40 characters".into());
                    continue;
                }
                let password = menu.room_password.trim().to_owned();
                let meta = RoomMeta {
                    code: crate::lobby::room_code(),
                    name,
                    host: menu.settings.name.trim().to_owned(),
                    password: !password.is_empty(),
                };
                let settings = &menu.settings;
                bridge.host_room(RoomSpec {
                    server: settings.server.trim().to_owned(),
                    user: Some(settings.user.trim().to_owned()).filter(|u| !u.is_empty()),
                    server_password: Some(settings.password.clone()).filter(|p| !p.is_empty()),
                    room_password: (!password.is_empty()).then_some(password),
                    meta: meta.clone(),
                    player: Some((settings.name.trim().to_owned(), menu.team)),
                });
                info!("Hosting room {} ({})", meta.name, meta.code);
                menu.hosting = Some(meta);
                menu.lobby = None;
                menu.screen = Screen::Off;
                menu.focus = None;
                // The start menu's match choices (camera, lighting, ...) apply to a hosted room too.
                if let Some(match_settings) = &match_settings {
                    crate::menu::apply_match_settings(
                        &mut commands,
                        match_settings,
                        lighting.as_deref_mut(),
                    );
                }
                session.phase = Phase::Playing;
                session.ui_epoch = session.ui_epoch.wrapping_add(1);
                commands.insert_resource(RebuildWorld);
            }
            OnlineButton::Join => {
                let Some(room) = menu.selected_room() else {
                    menu.error = Some("Pick a room first".into());
                    continue;
                };
                if room.password && menu.join_password.is_empty() {
                    menu.error = Some("This room needs its password".into());
                    menu.focus = Some(Field::JoinPassword);
                    continue;
                }
                if room.free(menu.team) == 0 {
                    menu.error = Some("No free rover on that team; pick the other".into());
                    continue;
                }
                // The relay takes over the local listener ARGOS connects to.
                if let Some(bridge) = bridge.as_ref() {
                    bridge.suspend();
                }
                let password = menu.join_password.clone();
                menu.joined = false;
                menu.relay = Some(Relay::start(JoinSpec {
                    settings: menu.settings.clone(),
                    room,
                    room_password: (!password.is_empty()).then_some(password),
                    team: menu.team,
                }));
                menu.lobby = None;
                menu.screen = Screen::Off;
                menu.focus = None;
                session.phase = Phase::Online;
                session.ui_epoch = session.ui_epoch.wrapping_add(1);
            }
            OnlineButton::Leave => leave_room(&mut menu, bridge.as_deref(), &mut session),
            OnlineButton::StartMatch | OnlineButton::ResetMatch => {
                if let Some(bridge) = bridge.as_ref() {
                    bridge.match_action(if action == OnlineButton::StartMatch {
                        MatchAction::Start
                    } else {
                        MatchAction::Reset
                    });
                }
            }
            OnlineButton::CloseRoom => {
                if let Some(bridge) = bridge.as_ref() {
                    bridge.close_room();
                }
                menu.hosting = None;
                session.ui_epoch = session.ui_epoch.wrapping_add(1);
            }
        }
        menu.bump();
    }
}

/// Typing into the focused field. Tab moves on, Enter presses the screen's
/// main button.
pub fn type_text(
    mut keys: MessageReader<KeyboardInput>,
    mut menu: ResMut<OnlineMenu>,
    buttons: Query<(Entity, &OnlineButton)>,
    mut interactions: Query<&mut Interaction>,
) {
    let Some(focus) = menu.focus else {
        keys.clear();
        return;
    };
    let mut changed = false;
    let mut submit = false;
    for key in keys.read() {
        if !key.state.is_pressed() {
            continue;
        }
        match &key.logical_key {
            Key::Character(text) => {
                let value = menu.field_mut(focus);
                for c in text.chars().filter(|c| !c.is_control()) {
                    if value.len() < 128 {
                        value.push(c);
                    }
                }
                changed = true;
            }
            Key::Space => {
                menu.field_mut(focus).push(' ');
                changed = true;
            }
            Key::Backspace => {
                menu.field_mut(focus).pop();
                changed = true;
            }
            Key::Tab => {
                let fields = menu.fields();
                if let Some(at) = fields.iter().position(|f| *f == focus) {
                    menu.focus = Some(fields[(at + 1) % fields.len()]);
                    changed = true;
                }
            }
            Key::Enter => submit = true,
            _ => {}
        }
    }
    if submit {
        let main = match menu.screen {
            Screen::SignIn => OnlineButton::Connect,
            Screen::Rooms => OnlineButton::Join,
            Screen::Host => OnlineButton::CreateRoom,
            Screen::Off => return,
        };
        for (entity, button) in &buttons {
            if *button == main
                && let Ok(mut interaction) = interactions.get_mut(entity)
            {
                *interaction = Interaction::Pressed;
            }
        }
    }
    if changed {
        menu.bump();
    }
}

/// Rebuilds the online UI when the room list, the relay or the hosted room
/// changes.
/// Drops the relay (which leaves the seat and frees the port) and returns to
/// the room list.
fn leave_room(menu: &mut OnlineMenu, bridge: Option<&Bridge>, session: &mut SimSession) {
    menu.relay = None;
    if let Some(bridge) = bridge {
        bridge.resume();
    }
    menu.lobby = Some(LobbyClient::connect(menu.settings.clone()));
    menu.screen = Screen::Rooms;
    session.phase = Phase::Menu;
    session.ui_epoch = session.ui_epoch.wrapping_add(1);
    menu.bump();
}

pub fn poll_online(
    mut menu: ResMut<OnlineMenu>,
    mut session: ResMut<SimSession>,
    bridge: Option<Res<Bridge>>,
) {
    // A refused join (wrong password, room full or gone) goes back to the list.
    if let Some(RelayStatus::Failed(error)) = menu.relay.as_ref().map(Relay::status)
        && !menu.joined
    {
        leave_room(&mut menu, bridge.as_deref(), &mut session);
        menu.join_password.clear();
        menu.focus = Some(Field::JoinPassword);
        menu.error = Some(error);
        return;
    }
    if matches!(
        menu.relay.as_ref().map(Relay::status),
        Some(RelayStatus::Playing(_))
    ) {
        menu.joined = true;
    }
    let mut seen = String::new();
    if let Some(lobby) = &menu.lobby {
        seen += &format!("{:?}{:?}", lobby.status(), lobby.rooms());
    }
    if let Some(relay) = &menu.relay {
        seen += &format!("{:?}", relay.status());
    }
    if menu.hosting.is_some()
        && let Some(bridge) = &bridge
    {
        seen += &format!("{:?}{:?}", bridge.room_status(), bridge.match_summary());
    }
    if seen != menu.seen {
        menu.seen = seen;
        menu.bump();
    }
}

const MUTED: Color = Color::srgb(0.62, 0.68, 0.70);
const ERROR: Color = Color::srgb(0.95, 0.55, 0.45);

fn text_field(parent: &mut ChildSpawnerCommands, menu: &OnlineMenu, field: Field, hint: &str) {
    let value = menu.field(field);
    let focused = menu.focus == Some(field);
    let shown = if field.secret() {
        "*".repeat(value.chars().count())
    } else {
        value.to_owned()
    };
    let (text, color) = if shown.is_empty() && !focused {
        (hint.to_owned(), MUTED)
    } else {
        (
            format!("{shown}{}", if focused { "|" } else { "" }),
            Color::WHITE,
        )
    };
    parent
        .spawn((
            Button,
            OnlineButton::Focus(field),
            Node {
                height: Val::Px(38.0),
                align_items: AlignItems::Center,
                padding: UiRect::horizontal(Val::Px(12.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                overflow: Overflow::clip(),
                ..default()
            },
            BorderColor::all(if focused {
                Color::srgb(0.25, 0.70, 0.62)
            } else {
                Color::srgb(0.30, 0.33, 0.36)
            }),
            BackgroundColor(Color::srgb(0.11, 0.12, 0.14)),
        ))
        .with_children(|field| {
            field.spawn((
                Text::new(text),
                TextFont {
                    font_size: FontSize::Px(16.0),
                    ..default()
                },
                TextColor(color),
            ));
        });
}

fn row(parent: &mut ChildSpawnerCommands, build: impl FnOnce(&mut ChildSpawnerCommands)) {
    parent
        .spawn(Node {
            column_gap: Val::Px(6.0),
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(build);
}

fn team_toggle(parent: &mut ChildSpawnerCommands, menu: &OnlineMenu, room: Option<&RoomInfo>) {
    row(parent, |row| {
        for (team, name) in [(Team::Red, "Red"), (Team::Blue, "Blue")] {
            let text = match room {
                Some(room) => format!("{name} ({} free)", room.free(team)),
                None => name.to_owned(),
            };
            button(row, &text, OnlineButton::Team(team), menu.team == team);
        }
    });
}

fn error_line(panel: &mut ChildSpawnerCommands, menu: &OnlineMenu) {
    if let Some(error) = &menu.error {
        label(panel, error, 14.0, ERROR);
    }
}

/// The online screen for the start menu's current step.
pub fn spawn_screen(panel: &mut ChildSpawnerCommands, menu: &OnlineMenu, rover_count: usize) {
    match menu.screen {
        Screen::SignIn => {
            label(panel, "Play online", 32.0, Color::WHITE);
            label(panel, "Server", 14.0, MUTED);
            text_field(panel, menu, Field::Server, crate::online::DEFAULT_SERVER);
            label(panel, "Server login", 14.0, MUTED);
            text_field(panel, menu, Field::User, "user");
            text_field(panel, menu, Field::Password, "password");
            label(panel, "Your name", 14.0, MUTED);
            text_field(panel, menu, Field::Name, "name");
            error_line(panel, menu);
            row(panel, |row| {
                button(row, "Back", OnlineButton::Back, false);
                button(row, "Connect", OnlineButton::Connect, true);
            });
        }
        Screen::Rooms => {
            label(panel, "Rooms", 32.0, Color::WHITE);
            let (status, rooms) = menu
                .lobby
                .as_ref()
                .map(|lobby| (lobby.status(), lobby.rooms()))
                .unwrap_or((LobbyStatus::Connecting, Vec::new()));
            match &status {
                LobbyStatus::Connecting => label(panel, "Connecting...", 16.0, MUTED),
                LobbyStatus::Failed(error) => label(panel, error, 14.0, ERROR),
                LobbyStatus::Connected if rooms.is_empty() => {
                    label(panel, "No rooms yet. Host one.", 16.0, MUTED)
                }
                LobbyStatus::Connected => {}
            }
            for room in &rooms {
                let seats = room.seats.len();
                let text = format!(
                    "{}{} ({}, {seats} in)",
                    room.name,
                    if room.password { " [pw]" } else { "" },
                    room.phase,
                );
                button(
                    panel,
                    &text,
                    OnlineButton::Select(room.code.clone()),
                    menu.selected.as_deref() == Some(room.code.as_str()),
                );
            }
            let selected = menu.selected_room();
            if let Some(room) = &selected {
                let players: Vec<_> = room.seats.iter().map(|seat| seat.name.as_str()).collect();
                label(
                    panel,
                    &format!(
                        "Code {}, host {}. Free rovers: red {}, blue {}. Players: {}",
                        room.code,
                        room.host,
                        room.free(Team::Red),
                        room.free(Team::Blue),
                        if players.is_empty() {
                            "none".into()
                        } else {
                            players.join(", ")
                        },
                    ),
                    14.0,
                    MUTED,
                );
                label(panel, "Team", 14.0, MUTED);
                team_toggle(panel, menu, Some(room));
                if room.password {
                    label(panel, "Room password", 14.0, MUTED);
                    text_field(panel, menu, Field::JoinPassword, "room password");
                }
            }
            error_line(panel, menu);
            row(panel, |row| {
                button(row, "Sign out", OnlineButton::Back, false);
                if status == LobbyStatus::Connected {
                    button(row, "Host a room", OnlineButton::ShowHost, false);
                }
                if selected.is_some() {
                    button(row, "Join", OnlineButton::Join, true);
                }
            });
        }
        Screen::Host => {
            label(panel, "Host a room", 32.0, Color::WHITE);
            label(panel, "Room name", 14.0, MUTED);
            text_field(panel, menu, Field::RoomName, "room name");
            label(
                panel,
                "Room password (empty: anyone on the server may join)",
                14.0,
                MUTED,
            );
            text_field(panel, menu, Field::RoomPassword, "no password");
            label(panel, "Rovers (red and blue alternate)", 14.0, MUTED);
            row(panel, |row| {
                button(row, "-", RoverStep(-1), false);
                label(row, &format!("{rover_count}"), 22.0, Color::WHITE);
                button(row, "+", RoverStep(1), false);
            });
            label(panel, "Your team", 14.0, MUTED);
            team_toggle(panel, menu, None);
            error_line(panel, menu);
            row(panel, |row| {
                button(row, "Back", OnlineButton::Back, false);
                button(row, "Create room", OnlineButton::CreateRoom, true);
            });
        }
        Screen::Off => {}
    }
}

/// The panel while this machine plays in someone else's room.
pub fn spawn_joined(panel: &mut ChildSpawnerCommands, menu: &OnlineMenu) {
    label(panel, "Online", 32.0, Color::WHITE);
    match menu.relay.as_ref().map(Relay::status) {
        None | Some(RelayStatus::Joining) => label(panel, "Joining...", 16.0, MUTED),
        Some(RelayStatus::Failed(error)) => label(panel, &error, 14.0, ERROR),
        Some(RelayStatus::Playing(seat)) => {
            let team = match seat.team {
                Team::Red => "red",
                Team::Blue => "blue",
            };
            let rovers: Vec<String> = seat.rovers.iter().map(u64::to_string).collect();
            label(
                panel,
                &format!(
                    "Seat {}, {team} team, rovers {}",
                    seat.seat,
                    rovers.join(", ")
                ),
                18.0,
                Color::WHITE,
            );
            label(
                panel,
                &format!(
                    "Match: {}, {} player{}, round trip {}",
                    seat.phase,
                    seat.players,
                    if seat.players == 1 { "" } else { "s" },
                    seat.rtt_ms.map_or("...".into(), |ms| format!("{ms:.0} ms")),
                ),
                16.0,
                MUTED,
            );
            label(
                panel,
                &format!("Open ARGOS on {LOCAL_ENDPOINT}, prefix terra/rover."),
                16.0,
                Color::WHITE,
            );
            label(
                panel,
                "Teleop and the MCP server use the same endpoint. Only your rovers take commands.",
                14.0,
                MUTED,
            );
        }
    }
    button(panel, "Leave room", OnlineButton::Leave, false);
}

/// Top-right chip while hosting: room code, phase, players, match buttons.
pub fn spawn_room_chip(
    commands: &mut Commands,
    root: impl Component,
    menu: &OnlineMenu,
    bridge: Option<&Bridge>,
) {
    let (Some(room), Some(bridge)) = (&menu.hosting, bridge) else {
        return;
    };
    let (phase, players, _) = bridge.match_summary();
    let status = match bridge.room_status() {
        RoomStatus::Open => format!(
            "{}, code {}, {players} player{}, {}",
            room.name,
            room.code,
            if players == 1 { "" } else { "s" },
            match phase {
                MatchPhase::Lobby => "waiting",
                MatchPhase::Running => "running",
                MatchPhase::Paused => "paused",
            }
        ),
        RoomStatus::Connecting => format!("{}: connecting to the server...", room.name),
        RoomStatus::Failed(error) => format!("{}: {error}", room.name),
        RoomStatus::Closed => return,
    };
    commands
        .spawn((
            root,
            Node {
                // Bottom left: the score panel and depth preview take the right side.
                position_type: PositionType::Absolute,
                bottom: Val::Px(16.0),
                left: Val::Px(16.0),
                column_gap: Val::Px(6.0),
                align_items: AlignItems::Center,
                padding: UiRect::all(Val::Px(6.0)),
                border_radius: BorderRadius::all(Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.12, 0.14, 0.16, 0.85)),
        ))
        .with_children(|chip| {
            label(chip, &status, 15.0, Color::WHITE);
            if phase == MatchPhase::Running {
                button(chip, "Reset match", OnlineButton::ResetMatch, false);
            } else {
                button(chip, "Start match", OnlineButton::StartMatch, true);
            }
        });
}
