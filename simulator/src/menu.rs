//! Start screen and pause menu. Escape or the corner button pauses a running world.

use crate::session::{Phase, RebuildWorld, SimSession, WorldChoice, menu_rover_max};
use bevy::prelude::*;

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_ui_camera).add_systems(
            Update,
            (toggle_pause, sync_clock, clicks, rebuild_ui, sync_ui_camera).chain(),
        );
        #[cfg(all(feature = "gmu-viewer", not(target_arch = "wasm32")))]
        app.add_systems(Update, sync_gmu_status.after(rebuild_ui));
    }
}

#[cfg(all(feature = "gmu-viewer", not(target_arch = "wasm32")))]
#[derive(Component)]
pub(crate) struct GmuRetryButton;
#[cfg(all(feature = "gmu-viewer", not(target_arch = "wasm32")))]
#[derive(Component)]
pub(crate) struct GmuViewButton;
#[cfg(all(feature = "gmu-viewer", not(target_arch = "wasm32")))]
#[derive(Component)]
struct GmuStatus;

#[cfg(all(feature = "gmu-viewer", not(target_arch = "wasm32")))]
fn sync_gmu_status(
    access: Option<Res<crate::gmu::Access>>,
    config: Option<Res<crate::world::WorldConfig>>,
    credits: Option<Res<bevy_3d_tiles::TilesetCredits>>,
    mut labels: Query<&mut Text, With<GmuStatus>>,
) {
    if let Some(access) = access {
        let message = if config.as_ref().is_some_and(|c| c.uses_google_scenery())
            && credits.is_some_and(|c| c.google_visible)
        {
            if config
                .as_ref()
                .is_some_and(|c| c.scene == crate::world::SceneKind::Tiles)
            {
                "Google imagery is live. Robots use the elevation terrain."
            } else {
                "GMU imagery is live. Robot views use the practice surface."
            }
        } else {
            &access.status
        };
        for mut text in &mut labels {
            if text.0 != message {
                text.0 = message.to_owned();
            }
        }
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
                    for choice in [
                        WorldChoice::Next,
                        WorldChoice::Tiles,
                        #[cfg(all(feature = "gmu-viewer", not(target_arch = "wasm32")))]
                        WorldChoice::Gmu,
                    ] {
                        button(
                            panel,
                            choice.label(),
                            PickWorld(choice),
                            session.world == choice,
                        );
                    }
                    #[cfg(all(feature = "gmu-viewer", not(target_arch = "wasm32")))]
                    if matches!(session.world, WorldChoice::Gmu | WorldChoice::Tiles) {
                        panel.spawn((
                            GmuStatus,
                            Text::new("Connecting map scenery..."),
                            TextFont {
                                font_size: FontSize::Px(14.0),
                                ..default()
                            },
                            TextColor(Color::srgb(0.75, 0.78, 0.80)),
                        ));
                        panel
                            .spawn(Node {
                                column_gap: px(8),
                                ..default()
                            })
                            .with_children(|row| {
                                button(row, "Retry scenery", GmuRetryButton, false);
                                if paused {
                                    button(row, "Switch camera", GmuViewButton, false);
                                }
                            });
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
                    if paused {
                        panel
                            .spawn(Node {
                                column_gap: px(6),
                                ..default()
                            })
                            .with_children(|row| {
                                button(row, "Resume", ResumeButton, true);
                                button(row, "Restart", RestartButton, false);
                                button(row, "Load world", LoadButton, false);
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

#[cfg(all(test, feature = "gmu-viewer", not(target_arch = "wasm32")))]
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
    fn gmu_uses_start_pause_resume_restart_and_world_selection() {
        let mut app = App::new();
        app.insert_resource(SimSession::at_menu(WorldChoice::Town, 1))
            .insert_resource(crate::world::WorldConfig::for_choice(WorldChoice::Gmu))
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<Time<Virtual>>()
            .add_plugins(MenuPlugin)
            .add_systems(Update, crate::gmu::switch_view.after(clicks));
        app.update();
        let world = app.world_mut();
        let gmu = world
            .query::<(Entity, &PickWorld)>()
            .iter(world)
            .find(|(_, p)| p.0 == WorldChoice::Gmu)
            .unwrap()
            .0;
        *world.get_mut::<Interaction>(gmu).unwrap() = Interaction::Pressed;
        app.update();
        assert_eq!(app.world().resource::<SimSession>().world, WorldChoice::Gmu);
        let world = app.world_mut();
        assert_eq!(world.query::<&GmuStatus>().iter(world).count(), 1);
        press::<StartButton>(&mut app);
        assert_eq!(app.world().resource::<SimSession>().phase, Phase::Playing);
        assert!(app.world().contains_resource::<RebuildWorld>());
        app.world_mut().remove_resource::<RebuildWorld>();
        press::<PauseButton>(&mut app);
        assert_eq!(app.world().resource::<SimSession>().phase, Phase::Paused);
        press::<GmuViewButton>(&mut app);
        assert!(
            app.world()
                .resource::<crate::world::WorldConfig>()
                .follow_rover
        );
        press::<ResumeButton>(&mut app);
        assert_eq!(app.world().resource::<SimSession>().phase, Phase::Playing);
        assert!(!app.world().contains_resource::<RebuildWorld>());
        press::<PauseButton>(&mut app);
        press::<RestartButton>(&mut app);
        assert!(app.world().contains_resource::<RebuildWorld>());
        app.world_mut().remove_resource::<RebuildWorld>();
        press::<PauseButton>(&mut app);
        let world = app.world_mut();
        let next = world
            .query::<(Entity, &PickWorld)>()
            .iter(world)
            .find(|(_, p)| p.0 == WorldChoice::Tiles)
            .unwrap()
            .0;
        *world.get_mut::<Interaction>(next).unwrap() = Interaction::Pressed;
        app.update();
        press::<LoadButton>(&mut app);
        assert_eq!(
            app.world().resource::<SimSession>().world,
            WorldChoice::Tiles
        );
        assert_eq!(app.world().resource::<SimSession>().phase, Phase::Playing);
        assert!(app.world().contains_resource::<RebuildWorld>());
    }
}
