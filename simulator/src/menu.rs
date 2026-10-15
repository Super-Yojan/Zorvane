//! Start screen and pause menu. Escape or the corner button pauses a running world.

use crate::session::{Phase, RebuildWorld, SimSession, WorldChoice, menu_rover_max};
use bevy::input::ButtonState;
use bevy::input::keyboard::KeyboardInput;
use bevy::prelude::*;

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_ui_camera).add_systems(
            Update,
            (
                toggle_pause,
                sync_clock,
                clicks,
                edit_router,
                rebuild_ui,
                sync_ui_camera,
            )
                .chain(),
        );
    }
}

#[derive(Component)]
struct UiCamera;
#[derive(Component)]
struct MenuRoot;
#[derive(Component)]
struct PickWorld(WorldChoice);
#[derive(Component)]
struct StartButton;
#[derive(Component)]
struct ResumeButton;
#[derive(Component)]
struct RestartButton;
#[derive(Component)]
struct LoadButton;
#[derive(Component)]
struct PauseButton;
#[derive(Component)]
struct RoverStep(i32);
#[derive(Component)]
struct RouterField;
#[derive(Component)]
struct RouterLabel;

fn spawn_ui_camera(mut commands: Commands) {
    commands.spawn((
        Name::new("Menu camera"),
        UiCamera,
        Camera2d,
        IsDefaultUiCamera,
        Camera {
            order: 1,
            clear_color: ClearColorConfig::Custom(Color::srgb(0.12, 0.14, 0.16)),
            ..default()
        },
    ));
}

fn toggle_pause(keys: Res<ButtonInput<KeyCode>>, mut session: ResMut<SimSession>) {
    if !keys.just_pressed(KeyCode::Escape) {
        return;
    }
    session.phase = match session.phase {
        Phase::Playing => Phase::Paused,
        Phase::Paused => Phase::Playing,
        Phase::Menu => Phase::Menu,
    };
    if session.phase != Phase::Menu {
        session.ui_epoch = session.ui_epoch.wrapping_add(1);
    }
}

fn sync_clock(session: Res<SimSession>, mut time: ResMut<Time<Virtual>>) {
    if session.phase == Phase::Playing {
        time.unpause();
    } else {
        time.pause();
    }
}

fn clicks(
    mut commands: Commands,
    mut session: ResMut<SimSession>,
    mut fleet: Option<ResMut<crate::terra::RoverFleet>>,
    worlds: Query<(&Interaction, &PickWorld), Changed<Interaction>>,
    start: Query<&Interaction, (Changed<Interaction>, With<StartButton>)>,
    resume: Query<&Interaction, (Changed<Interaction>, With<ResumeButton>)>,
    restart: Query<&Interaction, (Changed<Interaction>, With<RestartButton>)>,
    load: Query<&Interaction, (Changed<Interaction>, With<LoadButton>)>,
    pause: Query<&Interaction, (Changed<Interaction>, With<PauseButton>)>,
    steps: Query<(&Interaction, &RoverStep), Changed<Interaction>>,
    router_field: Query<&Interaction, (Changed<Interaction>, With<RouterField>)>,
) {
    let pressed = |interaction: &Interaction| *interaction == Interaction::Pressed;
    for (interaction, pick) in &worlds {
        if pressed(interaction) && session.world != pick.0 {
            session.world = pick.0;
            session.ui_epoch = session.ui_epoch.wrapping_add(1);
        }
    }
    for (interaction, step) in &steps {
        if !pressed(interaction) {
            continue;
        }
        let next = (session.rover_count as i32 + step.0).clamp(1, menu_rover_max() as i32) as usize;
        if next != session.rover_count {
            session.rover_count = next;
            session.ui_epoch = session.ui_epoch.wrapping_add(1);
        }
    }
    if start.iter().any(pressed) && session.phase == Phase::Menu {
        session.phase = Phase::Playing;
        session.ui_epoch = session.ui_epoch.wrapping_add(1);
        commands.insert_resource(RebuildWorld);
    }
    if load.iter().any(pressed) && session.phase == Phase::Paused {
        session.phase = Phase::Playing;
        session.ui_epoch = session.ui_epoch.wrapping_add(1);
        commands.insert_resource(RebuildWorld);
    }
    if restart.iter().any(pressed) {
        session.phase = Phase::Playing;
        session.ui_epoch = session.ui_epoch.wrapping_add(1);
        commands.insert_resource(RebuildWorld);
    }
    if resume.iter().any(pressed) && session.phase == Phase::Paused {
        session.phase = Phase::Playing;
        session.ui_epoch = session.ui_epoch.wrapping_add(1);
        if let Some(fleet) = fleet.as_mut() {
            let _ = fleet.set_count(session.rover_count);
        }
    }
    if pause.iter().any(pressed) && session.phase == Phase::Playing {
        session.phase = Phase::Paused;
        session.ui_epoch = session.ui_epoch.wrapping_add(1);
    }
    if router_field.iter().any(pressed) {
        session.router_focused = true;
    } else if start.iter().any(pressed)
        || resume.iter().any(pressed)
        || restart.iter().any(pressed)
        || load.iter().any(pressed)
        || worlds.iter().any(|(interaction, _)| pressed(interaction))
        || steps.iter().any(|(interaction, _)| pressed(interaction))
    {
        session.router_focused = false;
    }
}

fn edit_router(
    mut session: ResMut<SimSession>,
    mut keys: MessageReader<KeyboardInput>,
    mut labels: Query<&mut Text, With<RouterLabel>>,
) {
    if labels.is_empty() {
        return;
    }
    if session.router_focused {
        let mut changed = false;
        for event in keys.read() {
            if event.state != ButtonState::Pressed {
                continue;
            }
            match event.key_code {
                KeyCode::Backspace => {
                    session.router.pop();
                    changed = true;
                }
                KeyCode::Enter | KeyCode::Escape | KeyCode::Tab => {
                    session.router_focused = false;
                }
                _ => {
                    if let Some(text) = &event.text {
                        let piece = text.as_str();
                        if !piece.is_empty()
                            && piece.chars().all(|c| !c.is_control())
                            && session.router.len() < 180
                        {
                            session.router.push_str(piece);
                            changed = true;
                        }
                    }
                }
            }
        }
        if !changed && !session.is_changed() {
            return;
        }
    }
    let shown = if session.router_focused {
        format!("{}_", session.router)
    } else {
        session.router.clone()
    };
    for mut text in &mut labels {
        if text.as_str() != shown {
            **text = shown.clone();
        }
    }
}

fn rebuild_ui(
    mut commands: Commands,
    session: Res<SimSession>,
    roots: Query<Entity, With<MenuRoot>>,
    mut built: Local<u32>,
) {
    if *built == session.ui_epoch && !roots.is_empty() {
        return;
    }
    *built = session.ui_epoch;
    for entity in &roots {
        commands.entity(entity).despawn();
    }
    match session.phase {
        Phase::Playing => spawn_pause_chip(&mut commands),
        Phase::Menu => spawn_panel(&mut commands, &session, false),
        Phase::Paused => spawn_panel(&mut commands, &session, true),
    }
}

fn sync_ui_camera(session: Res<SimSession>, mut cameras: Query<&mut Camera, With<UiCamera>>) {
    // The world camera is spawned with a deferred command, so it can be missing
    // for the frame that leaves the menu. Stop clearing as soon as we play.
    let show_world = session.phase != Phase::Menu;
    for mut camera in &mut cameras {
        camera.clear_color = if show_world {
            ClearColorConfig::None
        } else {
            ClearColorConfig::Custom(Color::srgb(0.12, 0.14, 0.16))
        };
    }
}

fn spawn_pause_chip(commands: &mut Commands) {
    commands
        .spawn((
            MenuRoot,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(16.0),
                left: Val::Px(16.0),
                ..default()
            },
        ))
        .with_children(|parent| {
            button(parent, "Menu", PauseButton, false);
        });
}

fn spawn_panel(commands: &mut Commands, session: &SimSession, paused: bool) {
    commands
        .spawn((
            MenuRoot,
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_children(|parent| {
            parent
                .spawn((
                    Node {
                        width: Val::Px(420.0),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Stretch,
                        padding: UiRect::all(Val::Px(24.0)),
                        row_gap: Val::Px(12.0),
                        border_radius: BorderRadius::all(Val::Px(12.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.16, 0.18, 0.20)),
                ))
                .with_children(|panel| {
                    label(
                        panel,
                        if paused { "Paused" } else { "Zorvane" },
                        32.0,
                        Color::WHITE,
                    );
                    label(
                        panel,
                        "1 unit = 1 metre. Esc pauses.",
                        16.0,
                        Color::srgb(0.75, 0.78, 0.80),
                    );
                    if !session.note.is_empty() {
                        label(panel, &session.note, 15.0, Color::srgb(0.95, 0.72, 0.45));
                    }
                    label(panel, "World", 14.0, Color::srgb(0.62, 0.68, 0.70));
                    for choice in [WorldChoice::Town, WorldChoice::Tiles, WorldChoice::Next] {
                        button(
                            panel,
                            choice.label(),
                            PickWorld(choice),
                            session.world == choice,
                        );
                    }
                    label(panel, "Rovers", 14.0, Color::srgb(0.62, 0.68, 0.70));
                    panel
                        .spawn(Node {
                            justify_content: JustifyContent::SpaceBetween,
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(8.0),
                            ..default()
                        })
                        .with_children(|row| {
                            button(row, "−", RoverStep(-1), false);
                            label(row, &format!("{}", session.rover_count), 22.0, Color::WHITE);
                            button(row, "+", RoverStep(1), false);
                        });
                    #[cfg(target_arch = "wasm32")]
                    {
                        label(panel, "Zenoh router", 14.0, Color::srgb(0.62, 0.68, 0.70));
                        panel
                            .spawn((
                                Button,
                                RouterField,
                                Node {
                                    min_height: Val::Px(42.0),
                                    padding: UiRect::all(Val::Px(10.0)),
                                    align_items: AlignItems::Center,
                                    border_radius: BorderRadius::all(Val::Px(8.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgb(0.10, 0.12, 0.14)),
                            ))
                            .with_children(|field| {
                                field.spawn((
                                    RouterLabel,
                                    Text::new(session.router.clone()),
                                    TextFont {
                                        font_size: FontSize::Px(15.0),
                                        ..default()
                                    },
                                    TextColor(Color::srgb(0.85, 0.92, 0.90)),
                                ));
                            });
                    }
                    if paused {
                        button(panel, "Resume", ResumeButton, true);
                        button(panel, "Restart", RestartButton, false);
                        button(panel, "Load world", LoadButton, false);
                    } else {
                        button(panel, "Start", StartButton, true);
                    }
                });
        });
}

fn label(parent: &mut ChildSpawnerCommands, text: &str, size: f32, color: Color) {
    parent.spawn((
        Text::new(text),
        TextFont {
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(color),
    ));
}

fn button<T: Component>(parent: &mut ChildSpawnerCommands, text: &str, marker: T, selected: bool) {
    let color = if selected {
        Color::srgb(0.12, 0.45, 0.40)
    } else {
        Color::srgb(0.24, 0.27, 0.30)
    };
    parent
        .spawn((
            Button,
            marker,
            Node {
                height: Val::Px(42.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                padding: UiRect::horizontal(Val::Px(16.0)),
                border_radius: BorderRadius::all(Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(color),
        ))
        .with_children(|button| {
            button.spawn((
                Text::new(text),
                TextFont {
                    font_size: FontSize::Px(18.0),
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
        });
}
