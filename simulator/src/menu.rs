//! Start screen and pause menu. Escape or the corner button pauses a running world.

use crate::session::{Phase, RebuildWorld, SimSession, menu_rover_max};
use bevy::prelude::*;

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_ui_camera).add_systems(
            Update,
            (toggle_pause, sync_clock, clicks, rebuild_ui, sync_ui_camera).chain(),
        );
    }
}

#[derive(Component)]
struct UiCamera;
#[derive(Component)]
struct MenuRoot;
#[derive(Component)]
struct StartButton;
#[derive(Component)]
struct ResumeButton;
#[derive(Component)]
struct RestartButton;
#[derive(Component)]
struct PauseButton;
#[derive(Component)]
struct RoverStep(i32);

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
    start: Query<&Interaction, (Changed<Interaction>, With<StartButton>)>,
    resume: Query<&Interaction, (Changed<Interaction>, With<ResumeButton>)>,
    restart: Query<&Interaction, (Changed<Interaction>, With<RestartButton>)>,
    pause: Query<&Interaction, (Changed<Interaction>, With<PauseButton>)>,
    steps: Query<(&Interaction, &RoverStep), Changed<Interaction>>,
) {
    let pressed = |interaction: &Interaction| *interaction == Interaction::Pressed;
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
                        row_gap: Val::Px(8.0),
                        border_radius: BorderRadius::all(Val::Px(12.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.16, 0.18, 0.20)),
                ))
                .with_children(|panel| {
                    label(
                        panel,
                        if paused { "Paused" } else { "NEXT Competition" },
                        32.0,
                        Color::WHITE,
                    );
                    label(
                        panel,
                        "Field 3, West Campus. Esc pauses.",
                        16.0,
                        Color::srgb(0.75, 0.78, 0.80),
                    );
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
                    if paused {
                        panel
                            .spawn(Node {
                                column_gap: px(6),
                                ..default()
                            })
                            .with_children(|row| {
                                button(row, "Resume", ResumeButton, true);
                                button(row, "Restart", RestartButton, false);
                            });
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

#[cfg(test)]
mod tests {
    use super::*;

    fn press<T: Component>(app: &mut App) {
        let world = app.world_mut();
        let entity = world
            .query_filtered::<Entity, With<T>>()
            .single(world)
            .unwrap();
        *world.get_mut::<Interaction>(entity).unwrap() = Interaction::Pressed;
        app.update();
    }

    #[test]
    fn menu_runs_start_pause_resume_restart() {
        let mut app = App::new();
        app.insert_resource(SimSession::at_menu(1))
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<Time<Virtual>>()
            .add_plugins(MenuPlugin);
        app.update();
        press::<StartButton>(&mut app);
        assert_eq!(app.world().resource::<SimSession>().phase, Phase::Playing);
        assert!(app.world().contains_resource::<RebuildWorld>());
        app.world_mut().remove_resource::<RebuildWorld>();
        press::<PauseButton>(&mut app);
        assert_eq!(app.world().resource::<SimSession>().phase, Phase::Paused);
        press::<ResumeButton>(&mut app);
        assert_eq!(app.world().resource::<SimSession>().phase, Phase::Playing);
        assert!(!app.world().contains_resource::<RebuildWorld>());
        press::<PauseButton>(&mut app);
        press::<RestartButton>(&mut app);
        assert!(app.world().contains_resource::<RebuildWorld>());
    }
}
