//! Start screen and pause menu. Escape or the corner button pauses a running world.
//! The start screen also offers the command-line options (see `settings`); the
//! pause menu offers the match ones, which Restart applies.

use crate::session::{Phase, RebuildWorld, SimSession, menu_rover_max};
use crate::settings::{Applies, MenuSettings};
use bevy::prelude::*;

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        crate::online_menu::install(app);
        if !app.world().contains_resource::<MenuSettings>() {
            app.insert_resource(MenuSettings::from_env());
        }
        app.add_systems(Startup, spawn_ui_camera).add_systems(
            Update,
            (
                toggle_pause,
                sync_clock,
                clicks,
                crate::online_menu::type_text,
                crate::online_menu::online_clicks,
                crate::online_menu::poll_online,
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
struct StartButton;
#[derive(Component)]
struct ResumeButton;
#[derive(Component)]
struct RestartButton;
#[derive(Component)]
struct PauseButton;
#[derive(Component)]
pub(crate) struct RoverStep(pub i32);
#[derive(Component)]
struct PreviewToggle;
/// Cycles `MenuSettings::options[i]`.
#[derive(Component)]
struct SettingButton(usize);

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
        other => other,
    };
    if matches!(session.phase, Phase::Playing | Phase::Paused) {
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

/// Match choices take effect for the field that is about to be built.
pub(crate) fn apply_match_settings(
    commands: &mut Commands,
    settings: &MenuSettings,
    lighting: Option<&mut crate::mason_field::Lighting>,
) {
    settings.apply_match();
    if let Some(camera) = settings.get("TERRA_NEXT_CAMERA") {
        commands.insert_resource(crate::spectator::SpectatorCamera::from_setting(
            camera.value(),
        ));
    }
    if let Some(lighting) = lighting {
        *lighting = crate::mason_field::Lighting::from_env().unwrap_or_default();
    }
}

#[allow(clippy::too_many_arguments)]
fn clicks(
    mut commands: Commands,
    mut session: ResMut<SimSession>,
    mut settings: ResMut<MenuSettings>,
    mut lighting: Option<ResMut<crate::mason_field::Lighting>>,
    mut exit: MessageWriter<AppExit>,
    options: Query<(&Interaction, &SettingButton), Changed<Interaction>>,
    mut fleet: Option<ResMut<crate::terra::RoverFleet>>,
    start: Query<&Interaction, (Changed<Interaction>, With<StartButton>)>,
    resume: Query<&Interaction, (Changed<Interaction>, With<ResumeButton>)>,
    restart: Query<&Interaction, (Changed<Interaction>, With<RestartButton>)>,
    pause: Query<&Interaction, (Changed<Interaction>, With<PauseButton>)>,
    toggle: Query<&Interaction, (Changed<Interaction>, With<PreviewToggle>)>,
    mut depth: Option<ResMut<crate::depth_camera::DepthCameraConfig>>,
    steps: Query<(&Interaction, &RoverStep), Changed<Interaction>>,
) {
    let pressed = |interaction: &Interaction| *interaction == Interaction::Pressed;
    for (interaction, button) in &options {
        if pressed(interaction)
            && let Some(option) = settings.options.get_mut(button.0)
        {
            option.cycle();
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
    if toggle.iter().any(pressed) {
        if let Some(depth) = depth.as_mut() {
            depth.preview = !depth.preview;
        }
        session.ui_epoch = session.ui_epoch.wrapping_add(1);
    }
    if start.iter().any(pressed) && session.phase == Phase::Menu {
        if settings.needs_relaunch() {
            // Vehicle, network and detectors are read once at launch.
            match crate::settings::relaunch(&settings, session.rover_count) {
                Ok(()) => {
                    exit.write(AppExit::Success);
                    return;
                }
                Err(error) => eprintln!(
                    "Zorvane: could not relaunch with the new settings ({error}); \
                     starting with this run's"
                ),
            }
        }
        apply_match_settings(&mut commands, &settings, lighting.as_deref_mut());
        session.phase = Phase::Playing;
        session.ui_epoch = session.ui_epoch.wrapping_add(1);
        commands.insert_resource(RebuildWorld);
    }
    if restart.iter().any(pressed) {
        apply_match_settings(&mut commands, &settings, lighting.as_deref_mut());
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
    online: Res<crate::online_menu::OnlineMenu>,
    bridge: Option<Res<crate::bridge::Bridge>>,
    settings: Res<MenuSettings>,
    depth: Option<Res<crate::depth_camera::DepthCameraConfig>>,
    roots: Query<Entity, With<MenuRoot>>,
    mut built: Local<(u32, u32)>,
) {
    let epoch = (session.ui_epoch, online.epoch);
    if *built == epoch && !roots.is_empty() {
        return;
    }
    *built = epoch;
    for entity in &roots {
        commands.entity(entity).despawn();
    }
    let preview_on = depth.is_none_or(|depth| depth.preview);
    let hosting = online.hosting.is_some();
    match session.phase {
        Phase::Playing => {
            spawn_pause_chip(&mut commands);
            crate::online_menu::spawn_room_chip(
                &mut commands,
                MenuRoot,
                &online,
                bridge.as_deref(),
            );
        }
        Phase::Menu if online.screen != crate::online_menu::Screen::Off => {
            spawn_card(&mut commands, |panel| {
                crate::online_menu::spawn_screen(panel, &online, session.rover_count)
            });
        }
        Phase::Menu => spawn_panel(&mut commands, &session, &settings, false, preview_on, hosting),
        Phase::Paused => spawn_panel(&mut commands, &session, &settings, true, preview_on, hosting),
        Phase::Online => {
            spawn_card(&mut commands, |panel| {
                crate::online_menu::spawn_joined(panel, &online)
            });
        }
    }
}

/// A centred card on the menu background, like the start panel.
fn spawn_card(commands: &mut Commands, build: impl FnOnce(&mut ChildSpawnerCommands)) {
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
                        width: Val::Px(520.0),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Stretch,
                        padding: UiRect::all(Val::Px(24.0)),
                        row_gap: Val::Px(8.0),
                        border_radius: BorderRadius::all(Val::Px(12.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.16, 0.18, 0.20)),
                ))
                .with_children(build);
        });
}

fn sync_ui_camera(session: Res<SimSession>, mut cameras: Query<&mut Camera, With<UiCamera>>) {
    // The world camera is spawned with a deferred command, so it can be missing
    // for the frame that leaves the menu. Stop clearing as soon as we play.
    let show_world = matches!(session.phase, Phase::Playing | Phase::Paused);
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

fn spawn_panel(
    commands: &mut Commands,
    session: &SimSession,
    settings: &MenuSettings,
    paused: bool,
    preview_on: bool,
    hosting: bool,
) {
    let muted = Color::srgb(0.62, 0.68, 0.70);
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
                        width: Val::Px(if paused { 440.0 } else { 820.0 }),
                        max_width: Val::Percent(96.0),
                        max_height: Val::Percent(96.0),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Stretch,
                        padding: UiRect::all(Val::Px(20.0)),
                        row_gap: Val::Px(8.0),
                        border_radius: BorderRadius::all(Val::Px(12.0)),
                        overflow: Overflow::scroll_y(),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.16, 0.18, 0.20)),
                ))
                .with_children(|panel| {
                    label(
                        panel,
                        if paused { "Paused" } else { "NEXT Competition" },
                        30.0,
                        Color::WHITE,
                    );
                    label(
                        panel,
                        "Field 3, West Campus. Esc pauses. Click an option to change it.",
                        15.0,
                        Color::srgb(0.75, 0.78, 0.80),
                    );
                    panel
                        .spawn(Node {
                            column_gap: Val::Px(24.0),
                            align_items: AlignItems::FlexStart,
                            ..default()
                        })
                        .with_children(|columns| {
                            columns.spawn(column()).with_children(|col| {
                                label(col, "Match", 18.0, Color::WHITE);
                                option_row(col, "Rovers", |row| {
                                    row.spawn(Node {
                                        align_items: AlignItems::Center,
                                        column_gap: Val::Px(10.0),
                                        ..default()
                                    })
                                    .with_children(|stepper| {
                                        small_button(stepper, "-", RoverStep(-1), false);
                                        label(
                                            stepper,
                                            &format!("{}", session.rover_count),
                                            20.0,
                                            Color::WHITE,
                                        );
                                        small_button(stepper, "+", RoverStep(1), false);
                                    });
                                });
                                setting_rows(col, settings, Applies::Match);
                                option_row(col, "Depth preview", |row| {
                                    small_button(
                                        row,
                                        if preview_on { "On" } else { "Off" },
                                        PreviewToggle,
                                        false,
                                    );
                                });
                                if paused {
                                    label(col, "Restart applies match changes.", 13.0, muted);
                                }
                            });
                            if !paused {
                                columns.spawn(column()).with_children(|col| {
                                    label(col, "Simulator and network", 18.0, Color::WHITE);
                                    setting_rows(col, settings, Applies::Startup);
                                    label(
                                        col,
                                        "Changes here (marked *) restart Zorvane when you press Start.",
                                        13.0,
                                        muted,
                                    );
                                });
                            }
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
                                if hosting {
                                    button(
                                        row,
                                        "Close room",
                                        crate::online_menu::OnlineButton::CloseRoom,
                                        false,
                                    );
                                }
                            });
                    } else {
                        if settings.needs_relaunch() {
                            button(panel, "Restart Zorvane and start", StartButton, true);
                        } else {
                            button(panel, "Start", StartButton, true);
                        }
                        button(
                            panel,
                            "Play online",
                            crate::online_menu::OnlineButton::Open,
                            false,
                        );
                    }
                });
        });
}

fn column() -> Node {
    Node {
        flex_grow: 1.0,
        flex_basis: Val::Px(0.0),
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(6.0),
        ..default()
    }
}

/// A label on the left, a control on the right.
fn option_row(
    parent: &mut ChildSpawnerCommands,
    text: &str,
    control: impl FnOnce(&mut ChildSpawnerCommands),
) {
    parent
        .spawn(Node {
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::Center,
            column_gap: Val::Px(12.0),
            min_height: Val::Px(34.0),
            ..default()
        })
        .with_children(|row| {
            label(row, text, 15.0, Color::srgb(0.80, 0.84, 0.86));
            control(row);
        });
}

fn setting_rows(parent: &mut ChildSpawnerCommands, settings: &MenuSettings, applies: Applies) {
    for (index, option) in settings.options.iter().enumerate() {
        if option.applies != applies {
            continue;
        }
        let marked = applies == Applies::Startup && option.changed();
        let text = if marked {
            format!("{} *", option.shown())
        } else {
            option.shown().to_owned()
        };
        option_row(parent, option.label, |row| {
            small_button(row, &text, SettingButton(index), marked);
        });
    }
}

fn small_button<T: Component>(
    parent: &mut ChildSpawnerCommands,
    text: &str,
    marker: T,
    selected: bool,
) {
    let color = if selected {
        Color::srgb(0.45, 0.33, 0.10)
    } else {
        Color::srgb(0.24, 0.27, 0.30)
    };
    parent
        .spawn((
            Button,
            marker,
            Node {
                min_height: Val::Px(30.0),
                min_width: Val::Px(36.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                padding: UiRect::horizontal(Val::Px(12.0)),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(color),
        ))
        .with_children(|button| {
            button.spawn((
                Text::new(text),
                TextFont {
                    font_size: FontSize::Px(15.0),
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
        });
}

pub(crate) fn label(parent: &mut ChildSpawnerCommands, text: &str, size: f32, color: Color) {
    parent.spawn((
        Text::new(text),
        TextFont {
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(color),
    ));
}

pub(crate) fn button<T: Component>(
    parent: &mut ChildSpawnerCommands,
    text: &str,
    marker: T,
    selected: bool,
) {
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
        use crate::depth_camera::DepthCameraConfig;
        let mut app = App::new();
        app.insert_resource(SimSession::at_menu(1))
            .insert_resource(DepthCameraConfig::default())
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<Time<Virtual>>()
            .add_plugins(MenuPlugin);
        app.update();
        let world = app.world_mut();
        let lighting = world
            .query::<(Entity, &SettingButton)>()
            .iter(world)
            .find(|(_, button)| {
                world.resource::<MenuSettings>().options[button.0].key == "TERRA_NEXT_LIGHTING"
            })
            .map(|(entity, _)| entity)
            .unwrap();
        *world.get_mut::<Interaction>(lighting).unwrap() = Interaction::Pressed;
        app.update();
        assert_eq!(
            app.world()
                .resource::<MenuSettings>()
                .get("TERRA_NEXT_LIGHTING")
                .unwrap()
                .value(),
            "overcast"
        );
        press::<PreviewToggle>(&mut app);
        assert!(!app.world().resource::<DepthCameraConfig>().preview);
        press::<PreviewToggle>(&mut app);
        assert!(app.world().resource::<DepthCameraConfig>().preview);
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
