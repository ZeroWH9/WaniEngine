//! Rust-native desktop rendering workers for Wayland layer-shell and X11/XWayland.
mod animation;
mod control;
mod svg;
mod video;
mod x11_overlay;
mod x11_wallpaper;

use image::{DynamicImage, imageops::FilterType};
use serde_json::json;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{self, EventLoop},
        calloop_wayland_source::WaylandSource,
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{
        Shm, ShmHandler,
        slot::{Buffer, SlotPool},
    },
};
use std::{
    error::Error,
    ffi::OsString,
    io::{BufReader, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

/// Outputs exposed by the GNOME/XWayland desktop adapter.
pub fn x11_wallpaper_outputs() -> Result<Vec<serde_json::Value>, String> {
    x11_overlay::list_outputs().map_err(|e| e.to_string())
}

/// Probe the advertised protocol, rather than inferring it from an env variable.
pub fn layer_shell_available() -> bool {
    struct Probe;
    impl
        wayland_client::Dispatch<
            wayland_client::protocol::wl_registry::WlRegistry,
            wayland_client::globals::GlobalListContents,
        > for Probe
    {
        fn event(
            _: &mut Self,
            _: &wayland_client::protocol::wl_registry::WlRegistry,
            _: wayland_client::protocol::wl_registry::Event,
            _: &wayland_client::globals::GlobalListContents,
            _: &wayland_client::Connection,
            _: &wayland_client::QueueHandle<Self>,
        ) {
        }
    }
    let Ok(connection) = wayland_client::Connection::connect_to_env() else {
        return false;
    };
    let Ok((globals, _)) = wayland_client::globals::registry_queue_init::<Probe>(&connection)
    else {
        return false;
    };
    globals.contents().with_list(|list| {
        list.iter()
            .any(|global| global.interface == "zwlr_layer_shell_v1")
    })
}

/// Whether the optional librsvg command line renderer is available for SVG assets.
pub fn svg_renderer_available() -> bool {
    svg::renderer_available()
}

/// Rasterize an SVG asset into a bounded transparent image for rendering.
pub fn rasterize_svg(path: &Path) -> Result<DynamicImage, String> {
    svg::rasterize(path)
}

/// Decode bounded animated image frames for the egui editor preview.
pub fn load_animation_preview(
    path: &Path,
    max_dimension: u32,
) -> Result<Option<(Vec<DynamicImage>, Vec<u64>)>, String> {
    animation::Animation::load_if_animated(path)
        .map(|animation| animation.map(|animation| animation.into_preview_frames(max_dimension)))
        .map_err(|error| error.to_string())
}
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_output, wl_region, wl_shm, wl_surface},
};
use wayland_protocols::wp::{
    fractional_scale::v1::client::{
        wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
        wp_fractional_scale_v1::{self, WpFractionalScaleV1},
    },
    viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter},
};

const MAX_PIXELS: u64 = 32 * 1024 * 1024;

enum Command {
    Outputs,
    Show {
        image: PathBuf,
        video: Option<video::Options>,
        output: String,
        output_geometry: Option<OutputGeometry>,
        layer: Layer,
        actor: Option<ActorOptions>,
        control_stdin: bool,
        defer_present: bool,
        x11_wallpaper: bool,
    },
}

#[derive(Clone, Copy, Debug)]
struct OutputGeometry {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

enum ControlInput {
    Message(Result<control::Command, String>),
    End,
}

#[derive(Clone, Copy, Debug)]
struct ActorOptions {
    x: f32,
    y: f32,
    scale: f32,
    opacity: f32,
    rotation: f32,
    flip_x: bool,
    smoothing: bool,
    speed: f32,
    playing: bool,
    looping: bool,
}
impl Default for ActorOptions {
    fn default() -> Self {
        Self {
            x: 0.5,
            y: 0.96,
            scale: 100.0,
            opacity: 100.0,
            rotation: 0.0,
            flip_x: false,
            smoothing: true,
            speed: 100.0,
            playing: true,
            looping: true,
        }
    }
}

fn emit(message: serde_json::Value) -> std::io::Result<()> {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{message}")?;
    stdout.flush()
}

fn number(
    args: &mut impl Iterator<Item = OsString>,
    name: &str,
    min: f32,
    max: f32,
) -> Result<f32, String> {
    let value = args
        .next()
        .and_then(|value| value.into_string().ok())
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| value.is_finite() && *value >= min && *value <= max);
    value.ok_or_else(|| format!("{name} must be between {min} and {max}"))
}

fn use_x11_output_query() -> bool {
    std::env::var("XDG_SESSION_TYPE").is_ok_and(|session| session.eq_ignore_ascii_case("x11"))
        || std::env::var_os("WAYLAND_DISPLAY").is_none()
}

fn use_x11_actor_overlay() -> bool {
    use_x11_output_query()
        || [
            "XDG_CURRENT_DESKTOP",
            "XDG_SESSION_DESKTOP",
            "DESKTOP_SESSION",
        ]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .find(|desktop| !desktop.trim().is_empty())
        .is_some_and(|desktop| {
            desktop
                .split([':', ';', ','])
                .map(str::trim)
                .any(|desktop| {
                    desktop == "gnome" || desktop.starts_with("gnome-") || desktop == "ubuntu"
                })
        })
}

fn parse_options(args: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let mut image = None;
    let mut video_source = None;
    let mut fit = "cover".to_owned();
    let mut fps = 60.0;
    let mut duration = None;
    let mut output = None;
    let mut output_geometry = None;
    let mut layer = Layer::Bottom;
    let mut outputs = false;
    let mut control_stdin = false;
    let mut defer_present = false;
    let mut x11_wallpaper = false;
    let mut actor = false;
    let mut actor_options = ActorOptions::default();
    let mut actor_option_set = false;
    let mut explicit_layer = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--list-outputs") => outputs = true,
            Some("--actor") => actor = true,
            Some("--control-stdin") => control_stdin = true,
            Some("--defer-present") => defer_present = true,
            Some("--x11-wallpaper") => x11_wallpaper = true,
            Some("--x") => {
                actor_option_set = true;
                actor_options.x = number(&mut args, "--x", 0.0, 1.0)?;
            }
            Some("--y") => {
                actor_option_set = true;
                actor_options.y = number(&mut args, "--y", 0.0, 1.0)?;
            }
            Some("--scale") => {
                actor_option_set = true;
                actor_options.scale = number(&mut args, "--scale", 10.0, 300.0)?;
            }
            Some("--opacity") => {
                actor_option_set = true;
                actor_options.opacity = number(&mut args, "--opacity", 0.0, 100.0)?;
            }
            Some("--rotation") => {
                actor_option_set = true;
                actor_options.rotation = number(&mut args, "--rotation", -180.0, 180.0)?;
            }
            Some("--flip-x") => {
                actor_option_set = true;
                actor_options.flip_x = true;
            }
            Some("--no-smooth-scaling") => {
                actor_option_set = true;
                actor_options.smoothing = false;
            }
            Some("--speed") => {
                actor_option_set = true;
                actor_options.speed = number(&mut args, "--speed", 10.0, 300.0)?;
            }
            Some("--paused") => {
                actor_option_set = true;
                actor_options.playing = false;
            }
            Some("--no-loop") => {
                actor_option_set = true;
                actor_options.looping = false;
            }
            Some("--video") => {
                video_source = Some(PathBuf::from(args.next().ok_or("--video requires a path")?));
            }
            Some("--fit") => {
                fit = args
                    .next()
                    .and_then(|v| v.into_string().ok())
                    .ok_or("--fit requires a mode")?;
                if !["cover", "contain", "stretch", "center"].contains(&fit.as_str()) {
                    return Err("Unsupported wallpaper fit mode".into());
                }
            }
            Some("--duration") => {
                duration = Some(number(&mut args, "--duration", 0.001, 604800.0)? as f64)
            }
            Some("--fps") => fps = number(&mut args, "--fps", 1.0, 240.0)?,
            Some("--image") => {
                image = Some(PathBuf::from(args.next().ok_or("--image requires a path")?))
            }
            Some("--output") => {
                output = Some(
                    args.next()
                        .ok_or("--output requires a name")?
                        .into_string()
                        .map_err(|_| "Invalid output name")?,
                )
            }
            Some("--output-geometry") => {
                let mut next_i32 = || {
                    args.next()
                        .and_then(|value| value.into_string().ok())
                        .and_then(|value| value.parse::<i32>().ok())
                };
                let x = next_i32().ok_or("--output-geometry requires x y width height")?;
                let y = next_i32().ok_or("--output-geometry requires x y width height")?;
                let width = next_i32().ok_or("--output-geometry requires x y width height")?;
                let height = next_i32().ok_or("--output-geometry requires x y width height")?;
                if !(-32768..=32767).contains(&x)
                    || !(-32768..=32767).contains(&y)
                    || !(1..=32768).contains(&width)
                    || !(1..=32768).contains(&height)
                {
                    return Err("--output-geometry is outside supported bounds".into());
                }
                output_geometry = Some(OutputGeometry {
                    x,
                    y,
                    width: width as u32,
                    height: height as u32,
                });
            }
            Some("--layer") => {
                explicit_layer = true;
                layer = match args.next().and_then(|s| s.into_string().ok()).as_deref() {
                    Some("background") => Layer::Background,
                    Some("bottom") => Layer::Bottom,
                    _ => return Err("--layer must be background or bottom".into()),
                }
            }
            _ => return Err(format!("Unknown option: {}", arg.to_string_lossy())),
        }
    }
    if outputs
        && image.is_none()
        && video_source.is_none()
        && output.is_none()
        && !actor
        && !actor_option_set
        && !explicit_layer
        && !control_stdin
        && !defer_present
        && !x11_wallpaper
        && output_geometry.is_none()
    {
        return Ok(Command::Outputs);
    }
    if outputs {
        return Err("--list-outputs cannot be combined with --image or --output".into());
    }
    if control_stdin && !actor && video_source.is_none() {
        return Err("--control-stdin requires --actor or --video".into());
    }
    if !actor && video_source.is_none() && actor_option_set {
        return Err("Actor settings require --actor".into());
    }
    if video_source.is_some() && (actor || image.is_some()) {
        return Err("--video cannot be combined with --actor or --image".into());
    }
    if x11_wallpaper && (video_source.is_none() || actor || defer_present) {
        return Err("--x11-wallpaper requires --video and cannot defer presentation".into());
    }
    if defer_present && (video_source.is_none() || !control_stdin) {
        return Err("--defer-present requires --video and --control-stdin".into());
    }
    if actor && explicit_layer {
        return Err("--actor cannot be combined with --layer".into());
    }
    if actor {
        layer = Layer::Overlay;
    }
    let video = video_source.map(|path| video::Options {
        path,
        offset: 0.0,
        duration,
        fit,
        speed: actor_options.speed,
        fps,
        looping: actor_options.looping,
        playing: actor_options.playing,
    });
    Ok(Command::Show {
        image: video
            .as_ref()
            .map(|v| v.path.clone())
            .or(image)
            .ok_or("--image or --video is required")?,
        video,
        actor: actor.then_some(actor_options),
        control_stdin,
        defer_present,
        x11_wallpaper,
        output: output
            .filter(|s| !s.is_empty())
            .ok_or("--output is required")?,
        output_geometry,
        layer,
    })
}

/// Run the experimental Wayland worker in the current process.
/// The owning manager may invoke this from a dedicated child process.
pub fn run_cli(args: impl IntoIterator<Item = OsString>) -> Result<(), Box<dyn Error>> {
    match parse_options(args).map_err(|error| -> Box<dyn Error> { error.into() })? {
        Command::Outputs if use_x11_output_query() => {
            println!("{}", json!(x11_overlay::list_outputs()?));
            Ok(())
        }
        Command::Show {
            image,
            output,
            output_geometry,
            actor: Some(actor),
            control_stdin,
            ..
        } if use_x11_actor_overlay() => {
            x11_overlay::run(&image, &output, output_geometry, actor, control_stdin)
        }
        Command::Show { video: Some(options), output, output_geometry, control_stdin, x11_wallpaper: true, .. } => {
            x11_wallpaper::run(options, output, output_geometry, control_stdin)
        }
        Command::Show { video: Some(_), .. } if use_x11_output_query() => {
            Err("Direct video wallpapers require a Wayland compositor with layer-shell. X11 video wallpapers are not implemented.".into())
        }
        command => run_wayland(command),
    }
}

fn run_wayland(command: Command) -> Result<(), Box<dyn Error>> {
    let connection = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init(&connection)?;
    let qh = queue.handle();
    let compositor = CompositorState::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;
    let mut state = State {
        registry: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool: None,
        buffers: Vec::new(),
        video_position: 0.0,
        last_video_status: Instant::now(),
        layer: None,
        selected_output: None,
        output_name: String::new(),
        image: None,
        video_options: None,
        video_decoder: None,
        video_sender: None,
        video_pixels: None,
        video_dirty: false,
        video_ended: false,
        video_generation: 0,
        video_reported: false,
        video_presented: false,
        video_ready_reported: false,
        video_present_allowed: true,
        actor: None,
        placement: None,
        output_size: None,
        animation: None,
        clock: None,
        pending_update: None,
        animation_index: 0,
        frame_pending: false,
        width: 0,
        height: 0,
        scale: 1,
        viewport: None,
        fractional_object: None,
        fractional: None,
        error: None,
        done: false,
    };
    queue.roundtrip(&mut state)?;
    let outputs: Vec<_> = state
        .output_state
        .outputs()
        .filter_map(|output| state.output_state.info(&output).map(|info| (output, info)))
        .collect();
    if matches!(command, Command::Outputs) {
        let data: Vec<_> = outputs.iter().map(|(_, info)| json!({"name":info.name,"description":info.description,"logicalSize":info.logical_size,"logicalPosition":info.logical_position,"scale":info.scale_factor})).collect();
        println!("{}", json!(data));
        return Ok(());
    }
    let Command::Show {
        image,
        video,
        output,
        output_geometry: _,
        layer,
        actor,
        control_stdin,
        defer_present,
        x11_wallpaper: _,
    } = command
    else {
        unreachable!()
    };
    let (selected, info) = outputs
        .into_iter()
        .find(|(_, info)| info.name.as_deref() == Some(&output))
        .ok_or_else(|| format!("Output ‘{output}’ is not connected or has no stable name"))?;
    let animation = if actor.is_some() {
        animation::Animation::load_if_animated(&image)?
    } else {
        None
    };
    let image = if animation.is_some() || video.is_some() {
        None
    } else {
        Some(read_image(&image, actor.is_some())?)
    };
    let layer_shell = LayerShell::bind(&globals, &qh)
        .map_err(|_| "This compositor does not support the layer-shell protocol")?;
    let surface = compositor.create_surface(&qh);
    let surface_layer = layer_shell.create_layer_surface(
        &qh,
        surface,
        layer,
        Some(if actor.is_some() {
            "waniengine-actors"
        } else {
            "waniengine-wallpaper"
        }),
        Some(&selected),
    );
    let output_size = logical_size(info.logical_size);
    let image_size = image
        .as_ref()
        .or_else(|| {
            animation
                .as_ref()
                .and_then(|animation| animation.frames.first())
        })
        .map(|image| (image.width(), image.height()));
    let placement = actor
        .zip(output_size)
        .zip(image_size)
        .map(|((actor, output), source)| actor_placement(output, source, actor));
    configure_layer_geometry(&surface_layer, placement);

    surface_layer.set_exclusive_zone(-1);
    surface_layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    // Empty input region lets pointer events reach the desktop underneath.
    let region = compositor.wl_compositor().create_region(&qh, ());
    surface_layer.set_input_region(Some(&region));
    region.destroy();
    let viewport = globals
        .bind::<WpViewporter, _, _>(&qh, 1..=1, ())
        .ok()
        .map(|viewporter| viewporter.get_viewport(surface_layer.wl_surface(), &qh, ()));
    if viewport.is_some() {
        state.fractional_object = globals
            .bind::<WpFractionalScaleManagerV1, _, _>(&qh, 1..=1, ())
            .ok()
            .map(|manager| manager.get_fractional_scale(surface_layer.wl_surface(), &qh, ()));
    }
    state.viewport = state.fractional_object.as_ref().and(viewport);
    surface_layer.commit();
    state.scale = info.scale_factor.max(1);
    state.output_name = output;
    state.selected_output = Some(selected);
    state.image = image;
    state.video_options = video;
    state.video_present_allowed = !defer_present;
    state.animation = animation;
    state.actor = actor;
    state.placement = placement;
    state.output_size = output_size;
    state.pool = Some(SlotPool::new(4096, &state.shm)?);
    state.layer = Some(surface_layer);
    if control_stdin || state.video_options.is_some() {
        let mut event_loop: EventLoop<State> = EventLoop::try_new()?;
        let handle = event_loop.handle();
        WaylandSource::new(connection.clone(), queue).insert(handle.clone())?;
        if state.video_options.is_some() {
            let (sender, receiver) = calloop::channel::sync_channel::<video::Event>(1);
            state.video_sender = Some(sender);
            let video_qh = qh.clone();
            handle.insert_source(receiver, move |event, _, state| {
                if let calloop::channel::Event::Msg(event) = event {
                    state.video_event(event, &video_qh);
                }
            })?;
        }
        let (sender, receiver) = calloop::channel::sync_channel::<ControlInput>(8);
        let control_qh = qh.clone();
        handle.insert_source(receiver, move |event, _, state| match event {
            calloop::channel::Event::Msg(ControlInput::Message(Ok(command))) => {
                state.control(command, &control_qh)
            }
            calloop::channel::Event::Msg(ControlInput::Message(Err(error))) => {
                let _ = emit(json!({"event":"error","id":null,"message":error}));
            }
            calloop::channel::Event::Msg(ControlInput::End) => {
                state.done = true;
            }
            calloop::channel::Event::Closed => {
                state.done = true;
            }
        })?;
        if control_stdin {
            thread::spawn(move || {
                let stdin = std::io::stdin();
                let mut reader = BufReader::new(stdin.lock());
                loop {
                    match control::read_message(&mut reader) {
                        Ok(Some(bytes)) => {
                            let command = serde_json::from_slice::<control::Command>(&bytes)
                                .map_err(|error| error.to_string());
                            if sender.send(ControlInput::Message(command)).is_err() {
                                break;
                            }
                        }
                        Ok(None) => {
                            let _ = sender.send(ControlInput::End);
                            break;
                        }
                        Err(error) => {
                            let _ = sender.send(ControlInput::Message(Err(error.to_string())));
                            let _ = sender.send(ControlInput::End);
                            break;
                        }
                    }
                }
            });
        }
        while !state.done {
            event_loop.dispatch(Duration::from_millis(200), &mut state)?;
            if state
                .pending_update
                .is_some_and(|(_, deadline)| Instant::now() >= deadline)
            {
                let (id, _) = state.pending_update.take().unwrap();
                let _ = emit(
                    json!({"event":"error","id":id,"message":"Timed out waiting for the compositor to resize the overlay"}),
                );
                state.error = Some("Overlay resize timed out".into());
                state.done = true;
            }
        }
    } else {
        while !state.done {
            queue.blocking_dispatch(&mut state)?;
        }
    }
    if let Some(error) = state.error {
        return Err(error.into());
    }
    Ok(())
}

fn read_image(path: &Path, actor: bool) -> Result<DynamicImage, Box<dyn Error>> {
    if !path.is_file() {
        return Err("The image does not exist or is not a regular file".into());
    }
    if actor
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
    {
        return rasterize_svg(path).map_err(Into::into);
    }
    let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(if actor { 2048 } else { 16384 });
    limits.max_image_height = Some(if actor { 2048 } else { 16384 });
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    Ok(reader.decode()?)
}

struct State {
    registry: RegistryState,
    output_state: OutputState,
    shm: Shm,
    pool: Option<SlotPool>,
    buffers: Vec<Buffer>,
    video_position: f64,
    last_video_status: Instant,
    layer: Option<LayerSurface>,
    selected_output: Option<wl_output::WlOutput>,
    output_name: String,
    image: Option<DynamicImage>,
    video_options: Option<video::Options>,
    video_decoder: Option<video::Decoder>,
    video_sender: Option<calloop::channel::SyncSender<video::Event>>,
    video_pixels: Option<Vec<u8>>,
    video_dirty: bool,
    video_ended: bool,
    video_generation: u64,
    video_reported: bool,
    video_presented: bool,
    video_ready_reported: bool,
    video_present_allowed: bool,
    actor: Option<ActorOptions>,
    placement: Option<ActorPlacement>,
    output_size: Option<(u32, u32)>,
    animation: Option<animation::Animation>,
    clock: Option<control::PlaybackClock>,
    pending_update: Option<(u64, Instant)>,
    animation_index: usize,
    frame_pending: bool,
    width: u32,
    height: u32,
    scale: i32,
    // Fractional scaling needs both objects; otherwise the integer buffer scale is used.
    viewport: Option<WpViewport>,
    fractional_object: Option<WpFractionalScaleV1>,
    // Preferred scale in 1/120 units, as reported by wp-fractional-scale-v1.
    fractional: Option<u32>,
    error: Option<String>,
    done: bool,
}

impl State {
    fn draw(&mut self, qh: &QueueHandle<Self>, report: bool) -> Result<(), Box<dyn Error>> {
        let (width, height, scale120) =
            checked_buffer_size(self.width, self.height, self.scale120())?;
        let layer = self.layer.as_ref().ok_or("No surface")?;
        let image = self
            .animation
            .as_ref()
            .map(|animation| &animation.frames[self.animation_index])
            .or(self.image.as_ref());
        if image.is_none() && self.video_pixels.is_none() {
            return Ok(());
        }
        let pool = self.pool.as_mut().ok_or("No buffer pool")?;
        let stride = (width * 4) as i32;
        let format = if self.actor.is_some() {
            wl_shm::Format::Argb8888
        } else {
            wl_shm::Format::Xrgb8888
        };
        if self
            .buffers
            .first()
            .is_some_and(|buffer| buffer.height() != height as i32 || buffer.stride() != stride)
        {
            self.buffers.clear();
        }
        let available = self
            .buffers
            .iter()
            .position(|buffer| buffer.canvas(pool).is_some());
        let index = if let Some(index) = available {
            index
        } else if self.buffers.len() < 3 {
            let (buffer, _) = pool.create_buffer(width as i32, height as i32, stride, format)?;
            self.buffers.push(buffer);
            self.buffers.len() - 1
        } else {
            if !self.frame_pending {
                layer.wl_surface().frame(qh, layer.wl_surface().clone());
                self.frame_pending = true;
                layer.commit();
            }
            return Ok(());
        };
        let buffer = &self.buffers[index];
        let canvas = buffer
            .canvas(pool)
            .ok_or("Compositor still owns this frame")?;
        if let Some(pixels) = self.video_pixels.as_ref() {
            if pixels.len() != canvas.len() {
                return Err("Video frame size does not match the output".into());
            }
            canvas.copy_from_slice(pixels);
            self.video_dirty = false;
        } else if let Some(actor) = self.actor {
            render_actor(
                canvas,
                width,
                height,
                scale120 as f32 / 120.0,
                image.ok_or("No actor image")?,
                actor,
                self.placement,
            )?;
        } else {
            let image = image.ok_or("No wallpaper image")?;
            let (x, y, crop_width, crop_height) =
                cover_crop(image.width(), image.height(), width, height);
            let resized = image
                .crop_imm(x, y, crop_width, crop_height)
                .resize_exact(width, height, FilterType::Triangle)
                .to_rgba8();
            for (pixel, dst) in resized.pixels().zip(canvas.chunks_exact_mut(4)) {
                let a = pixel[3] as u32;
                let channel = |v: u8| (v as u32 * a + 127) / 255;
                let packed = (0xff << 24)
                    | (channel(pixel[0]) << 16)
                    | (channel(pixel[1]) << 8)
                    | channel(pixel[2]);
                dst.copy_from_slice(&packed.to_ne_bytes());
            }
        }
        if let Some(viewport) = &self.viewport {
            // The buffer holds physical pixels; the viewport maps it to the logical size.
            layer
                .set_buffer_scale(1)
                .map_err(|_| "The compositor does not support buffer scale")?;
            viewport.set_destination(self.width as i32, self.height as i32);
        } else {
            layer
                .set_buffer_scale(scale120 / 120)
                .map_err(|_| "The compositor does not support buffer scale")?;
        }
        layer
            .wl_surface()
            .damage_buffer(0, 0, width as i32, height as i32);
        buffer.attach_to(layer.wl_surface())?;
        if !self.frame_pending
            && (self.animation_active()
                || (self.video_options.is_some() && !self.video_presented)
                || self.video_options.as_ref().is_some_and(|v| v.playing))
        {
            layer.wl_surface().frame(qh, layer.wl_surface().clone());
            self.frame_pending = true;
        }
        layer.commit();
        if self.video_options.is_some() {
            if !self.video_reported
                || self.last_video_status.elapsed() >= Duration::from_millis(250)
            {
                self.last_video_status = Instant::now();
                emit(json!({"event":"position","seconds":self.video_position}))?;
            }
        }
        if report || (self.video_options.is_some() && !self.video_reported) {
            self.video_reported = true;
            emit(
                json!({"event":"configured","output":self.output_name,"width":self.width,"height":self.height,"scale":f64::from(scale120) / 120.0}),
            )?;
        }
        Ok(())
    }
    fn scale120(&self) -> u32 {
        match (&self.viewport, self.fractional) {
            (Some(_), Some(preferred)) => preferred,
            _ => self.scale.max(1) as u32 * 120,
        }
    }
    fn rescale(&mut self, qh: &QueueHandle<Self>) {
        if self.width == 0 || self.height == 0 {
            return; // The first configure will draw with the stored scale.
        }
        if self.video_options.is_some() {
            if let Err(error) = self.start_video() {
                self.error = Some(error);
                self.done = true;
            }
        } else {
            self.redraw(qh, true);
        }
    }
    fn start_video(&mut self) -> Result<(), String> {
        let (width, height, _) = checked_buffer_size(self.width, self.height, self.scale120())
            .map_err(|e| e.to_string())?;
        if self
            .video_decoder
            .as_ref()
            .is_some_and(|decoder| decoder.size == (width, height))
        {
            return Ok(());
        }
        self.video_generation += 1;
        self.video_decoder = None;
        self.video_pixels = None;
        self.video_dirty = false;
        let options = self.video_options.as_mut().ok_or("No video source")?;
        options.offset = self.video_position;
        let sender = self.video_sender.clone().ok_or("No video frame channel")?;
        self.video_decoder = Some(video::Decoder::start(
            options,
            (width, height),
            self.video_generation,
            sender,
        )?);
        Ok(())
    }

    fn video_event(&mut self, event: video::Event, qh: &QueueHandle<Self>) {
        match event {
            video::Event::Frame {
                generation,
                pixels,
                position,
            } if generation == self.video_generation => {
                let first = self.video_pixels.is_none();
                if !first && !self.video_present_allowed {
                    return;
                }
                if !first && !self.video_options.as_ref().is_some_and(|v| v.playing) {
                    return;
                }
                if first {
                    if let Some(decoder) = self.video_decoder.as_mut() {
                        if let Err(error) = decoder.first_frame() {
                            self.error = Some(error);
                            self.done = true;
                            return;
                        }
                    }
                }
                self.video_position = position;
                self.video_pixels = Some(pixels);
                self.video_dirty = true;
                if !self.video_ready_reported {
                    self.video_ready_reported = true;
                    let _ = emit(json!({"event":"ready","output":self.output_name}));
                    if !self.video_present_allowed {
                        if let Some(decoder) = self.video_decoder.as_mut() {
                            if let Err(error) = decoder.set_playing(false) {
                                self.error = Some(error);
                                self.done = true;
                            }
                        }
                    }
                }
                if !self.frame_pending {
                    self.redraw(qh, false);
                }
            }
            video::Event::End { generation, result } if generation == self.video_generation => {
                let result = result.and_then(|()| {
                    self.video_decoder
                        .as_mut()
                        .map_or(Ok(()), video::Decoder::finish)
                });
                match result {
                    Ok(()) => {
                        self.video_ended = true;
                        let _ = emit(json!({"event":"ended","seconds":self.video_position}));
                    }
                    Err(error) => {
                        let _ = emit(json!({"event":"error","id":null,"message":error}));
                        self.error = Some(error);
                        self.done = true;
                    }
                }
            }
            _ => {}
        }
    }

    fn animation_active(&self) -> bool {
        self.animation
            .as_ref()
            .zip(self.actor)
            .zip(self.clock.as_ref())
            .is_some_and(|((animation, actor), clock)| {
                actor.playing
                    && animation
                        .frame_at(clock.position(Instant::now()), 100.0, actor.looping)
                        .1
            })
    }

    fn redraw(&mut self, qh: &QueueHandle<Self>, report: bool) {
        if self.width == 0
            || self.height == 0
            || (self.video_options.is_some() && !self.video_present_allowed)
        {
            return;
        }
        if let Err(error) = self.draw(qh, report) {
            self.error = Some(error.to_string());
            self.done = true;
        }
    }
    fn control(&mut self, command: control::Command, qh: &QueueHandle<Self>) {
        let id = command.id();
        if matches!(command, control::Command::Stop { .. }) {
            let _ = emit(json!({"event":"stopped","id":id}));
            self.done = true;
            return;
        }
        if self.video_options.is_some() {
            if matches!(command, control::Command::Present { .. }) {
                if self.video_pixels.is_none() {
                    let _ =
                        emit(json!({"event":"error","id":id,"message":"No video frame is ready"}));
                    return;
                }
                self.video_present_allowed = true;
                if let Some(decoder) = self.video_decoder.as_mut() {
                    if let Err(error) =
                        decoder.set_playing(self.video_options.as_ref().unwrap().playing)
                    {
                        self.error = Some(error);
                        self.done = true;
                        return;
                    }
                }
                self.redraw(qh, false);
                return;
            }
            let result = match &command {
                control::Command::VideoSettings { fps, .. } if (1..=240).contains(fps) => {
                    let options = self.video_options.as_mut().unwrap();
                    options.fps = *fps as f32;
                    options.offset = self.video_position;
                    if self.video_ended {
                        Ok(())
                    } else {
                        self.video_decoder = None;
                        self.start_video()
                    }
                }
                control::Command::Update {
                    playing: Some(playing),
                    ..
                } => {
                    self.video_options.as_mut().unwrap().playing = *playing;
                    self.video_decoder
                        .as_mut()
                        .map_or(Ok(()), |decoder| decoder.set_playing(*playing))
                }
                _ => Err("Video playback accepts playing updates only".into()),
            };
            let _ = match result {
                Ok(()) => emit(
                    json!({"event":"applied","id":id,"seconds":self.video_position,"ended":self.video_ended}),
                ),
                Err(error) => emit(json!({"event":"error","id":id,"message":error})),
            };
            return;
        }
        let Some(previous) = self.actor else {
            let _ = emit(json!({"event":"error","id":id,"message":"No actor is active"}));
            return;
        };
        let updated = match command.update(previous) {
            Ok(Some((_, value))) => value,
            Ok(None) => return,
            Err(error) => {
                let _ = emit(json!({"event":"error","id":id,"message":error}));
                return;
            }
        };
        let had_pending = self.pending_update.take();
        if let Some((old_id, _)) = had_pending {
            let _ = emit(json!({"event":"superseded","id":old_id}));
        }
        if let Some(clock) = &mut self.clock {
            clock.update(updated, Instant::now());
        }
        self.actor = Some(updated);
        if let Some((animation, clock)) = self.animation.as_ref().zip(self.clock.as_ref()) {
            self.animation_index = animation
                .frame_at(clock.position(Instant::now()), 100.0, updated.looping)
                .0;
        }
        let source = self
            .animation
            .as_ref()
            .and_then(|animation| animation.frames.first())
            .or(self.image.as_ref());
        self.placement = self.output_size.zip(source).map(|(output, image)| {
            actor_placement(output, (image.width(), image.height()), updated)
        });
        let needs_configure = had_pending.is_some()
            || self.width == 0
            || self.height == 0
            || self.placement.is_some_and(|placement| {
                (placement.width, placement.height) != (self.width, self.height)
            });
        if let Some(layer) = &self.layer {
            configure_layer_geometry(layer, self.placement);
        }
        if needs_configure {
            self.pending_update = Some((id, Instant::now() + Duration::from_secs(5)));
            if let Some(layer) = &self.layer {
                layer.commit();
            }
        } else {
            self.redraw(qh, false);
            if !self.done {
                let _ = emit(json!({"event":"applied","id":id}));
            }
        }
    }

    fn animate(&mut self, qh: &QueueHandle<Self>) {
        self.frame_pending = false;
        if self.video_options.is_some() {
            if !self.video_presented {
                self.video_presented = true;
                let _ = emit(json!({"event":"presented","output":self.output_name}));
            }
            if self.video_dirty {
                self.redraw(qh, false);
            }
            return;
        }
        let Some(((animation, actor), clock)) = self
            .animation
            .as_ref()
            .zip(self.actor)
            .zip(self.clock.as_ref())
        else {
            return;
        };
        if !actor.playing {
            return;
        }
        let (index, active) =
            animation.frame_at(clock.position(Instant::now()), 100.0, actor.looping);
        if index != self.animation_index {
            self.animation_index = index;
            self.redraw(qh, false);
        } else if active {
            if let Some(layer) = &self.layer {
                layer.wl_surface().frame(qh, layer.wl_surface().clone());
                self.frame_pending = true;
                layer.commit();
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ActorPlacement {
    left: i32,
    top: i32,
    width: u32,
    height: u32,
    center_x: f32,
    center_y: f32,
    actor_width: f32,
}

fn logical_size(size: Option<(i32, i32)>) -> Option<(u32, u32)> {
    size.filter(|(width, height)| *width > 0 && *height > 0)
        .map(|(width, height)| (width as u32, height as u32))
}

// Match the previous overlay's center-x / bottom-y placement, but allocate
// only the rotated, visible bounding box on the selected output.
fn actor_placement(output: (u32, u32), source: (u32, u32), actor: ActorOptions) -> ActorPlacement {
    let (out_width, out_height) = (output.0 as f32, output.1 as f32);
    let actor_width = (source.0 as f32 * actor.scale / 100.0).min(out_width);
    let actor_height = actor_width * source.1 as f32 / source.0 as f32;
    let cx = out_width * actor.x;
    let cy = out_height * actor.y - actor_height / 2.0;
    let (sin, cos) = actor.rotation.to_radians().sin_cos();
    let bounds_width = cos.abs() * actor_width + sin.abs() * actor_height;
    let bounds_height = sin.abs() * actor_width + cos.abs() * actor_height;
    let left = (cx - bounds_width / 2.0)
        .floor()
        .clamp(0.0, out_width - 1.0) as i32;
    let top = (cy - bounds_height / 2.0)
        .floor()
        .clamp(0.0, out_height - 1.0) as i32;
    let right = (cx + bounds_width / 2.0)
        .ceil()
        .clamp(left as f32 + 1.0, out_width) as u32;
    let bottom = (cy + bounds_height / 2.0)
        .ceil()
        .clamp(top as f32 + 1.0, out_height) as u32;
    ActorPlacement {
        left,
        top,
        width: right - left as u32,
        height: bottom - top as u32,
        center_x: cx - left as f32,
        center_y: cy - top as f32,
        actor_width,
    }
}

fn configure_layer_geometry(layer: &LayerSurface, placement: Option<ActorPlacement>) {
    if let Some(placement) = placement {
        layer.set_anchor(Anchor::TOP | Anchor::LEFT);
        layer.set_size(placement.width, placement.height);
        layer.set_margin(placement.top, 0, 0, placement.left);
    } else {
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_size(0, 0);
        layer.set_margin(0, 0, 0, 0);
    }
}

// Render premultiplied RGBA into a transparent output, then convert to the
// native-endian ARGB layout required by Wayland shared memory.
fn render_actor(
    canvas: &mut [u8],
    width: u32,
    height: u32,
    buffer_scale: f32,
    image: &DynamicImage,
    options: ActorOptions,
    placement: Option<ActorPlacement>,
) -> Result<(), Box<dyn Error>> {
    let mut pixels = image.to_rgba8().into_raw();
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = pixel[3] as u32;
        for channel in &mut pixel[..3] {
            *channel = ((*channel as u32 * alpha + 127) / 255) as u8;
        }
    }
    let size = tiny_skia::IntSize::from_wh(image.width(), image.height())
        .ok_or("Invalid actor image size")?;
    let source = tiny_skia::Pixmap::from_vec(pixels, size).ok_or("Invalid actor image data")?;
    let mut target =
        tiny_skia::PixmapMut::from_bytes(canvas, width, height).ok_or("Invalid overlay buffer")?;
    target.fill(tiny_skia::Color::TRANSPARENT);
    let (actor_width, center_x, center_y) = if let Some(placement) = placement {
        (
            placement.actor_width * buffer_scale,
            placement.center_x * buffer_scale,
            placement.center_y * buffer_scale,
        )
    } else {
        let logical_width = width as f32 / buffer_scale;
        let actor_width =
            (image.width() as f32 * options.scale / 100.0).min(logical_width) * buffer_scale;
        let actor_height = image.height() as f32 * actor_width / image.width() as f32;
        (
            actor_width,
            width as f32 * options.x,
            height as f32 * options.y - actor_height / 2.0,
        )
    };
    let factor = actor_width / image.width() as f32;
    let (sin, cos) = options.rotation.to_radians().sin_cos();
    let sx = if options.flip_x { -factor } else { factor };
    let sy = factor;
    let a = cos * sx;
    let b = sin * sx;
    let c = -sin * sy;
    let d = cos * sy;
    let tx = center_x - a * image.width() as f32 / 2.0 - c * image.height() as f32 / 2.0;
    let ty = center_y - b * image.width() as f32 / 2.0 - d * image.height() as f32 / 2.0;
    let transform = tiny_skia::Transform::from_row(a, b, c, d, tx, ty);
    let paint = tiny_skia::PixmapPaint {
        opacity: options.opacity / 100.0,
        quality: if options.smoothing {
            tiny_skia::FilterQuality::Bilinear
        } else {
            tiny_skia::FilterQuality::Nearest
        },
        ..Default::default()
    };
    target.draw_pixmap(0, 0, source.as_ref(), &paint, transform, None);
    for pixel in canvas.chunks_exact_mut(4) {
        let packed = (u32::from(pixel[3]) << 24)
            | (u32::from(pixel[0]) << 16)
            | (u32::from(pixel[1]) << 8)
            | u32::from(pixel[2]);
        pixel.copy_from_slice(&packed.to_ne_bytes());
    }
    Ok(())
}

// Crop before resizing: resizing an extremely wide/tall source to cover an
// output can otherwise allocate an enormous intermediate image.
fn cover_crop(
    source_width: u32,
    source_height: u32,
    target_width: u32,
    target_height: u32,
) -> (u32, u32, u32, u32) {
    let (width, height) = if u64::from(source_width) * u64::from(target_height)
        > u64::from(source_height) * u64::from(target_width)
    {
        (
            ((u64::from(source_height) * u64::from(target_width)) / u64::from(target_height)).max(1)
                as u32,
            source_height,
        )
    } else {
        (
            source_width,
            ((u64::from(source_width) * u64::from(target_height)) / u64::from(target_width)).max(1)
                as u32,
        )
    };
    (
        (source_width - width) / 2,
        (source_height - height) / 2,
        width,
        height,
    )
}

// `scale120` is the surface scale in 1/120 units (120 = 1x), rounded like wp-fractional-scale.
fn checked_buffer_size(
    width: u32,
    height: u32,
    scale120: u32,
) -> Result<(u32, u32, u32), &'static str> {
    if width == 0 || height == 0 || scale120 < 120 {
        return Err("Invalid surface size or scale");
    }
    let scaled = |value: u32| (u64::from(value) * u64::from(scale120) + 60) / 120;
    let (width, height) = (scaled(width), scaled(height));
    if width * height > MAX_PIXELS
        || width > u64::from(i32::MAX as u32 / 4)
        || height > u64::from(i32::MAX as u32)
    {
        return Err("Surface exceeds the memory limit");
    }
    Ok((width as u32, height as u32, scale120))
}

impl CompositorHandler for State {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        scale: i32,
    ) {
        if self
            .layer
            .as_ref()
            .is_some_and(|layer| layer.wl_surface() == surface)
            && self.scale != scale
        {
            let before = self.scale120();
            self.scale = scale;
            if self.scale120() != before {
                self.rescale(qh);
            }
        }
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        if self
            .layer
            .as_ref()
            .is_some_and(|layer| layer.wl_surface() == surface)
        {
            self.animate(qh);
        }
    }
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}
impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if self.selected_output.as_ref() != Some(&output) {
            return;
        }
        let size = self
            .output_state
            .info(&output)
            .and_then(|info| logical_size(info.logical_size));
        if self.output_size == size {
            return;
        }
        self.output_size = size;
        let source = self
            .animation
            .as_ref()
            .and_then(|animation| animation.frames.first())
            .or(self.image.as_ref());
        self.placement = self
            .actor
            .zip(size)
            .zip(source)
            .map(|((actor, output), image)| {
                actor_placement(output, (image.width(), image.height()), actor)
            });
        if let Some(layer) = &self.layer {
            configure_layer_geometry(layer, self.placement);
            layer.commit();
        }
    }
    fn output_destroyed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if self.selected_output.as_ref() == Some(&output) {
            self.error = Some("The selected output was disconnected".into());
            self.done = true;
        }
    }
}
impl LayerShellHandler for State {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.error = Some("The compositor closed the wallpaper surface".into());
        self.done = true;
    }
    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        self.width = configure.new_size.0;
        self.height = configure.new_size.1;
        if self.width == 0 || self.height == 0 {
            self.error = Some("The compositor returned an empty output size".into());
            self.done = true;
        } else {
            if self.pending_update.is_some()
                && self.placement.is_some_and(|placement| {
                    (placement.width, placement.height) != (self.width, self.height)
                })
            {
                return;
            }
            if self.video_options.is_some() {
                if let Err(error) = self.start_video() {
                    self.error = Some(error);
                    self.done = true;
                }
                return;
            }
            if self.clock.is_none() {
                if let Some(actor) = self.actor {
                    self.clock = Some(control::PlaybackClock::new(actor, Instant::now()));
                }
            }
            self.redraw(qh, true);
            if !self.done {
                if let Some((id, _)) = self.pending_update.take() {
                    let _ = emit(json!({"event":"applied","id":id}));
                }
            }
        }
    }
}
impl ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}
impl Dispatch<WpFractionalScaleV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            let scale = scale.clamp(120, 120 * 8);
            if state.fractional != Some(scale) {
                let before = state.scale120();
                state.fractional = Some(scale);
                if state.scale120() != before {
                    state.rescale(qh);
                }
            }
        }
    }
}
macro_rules! ignore_events {
    ($($interface:ty),+) => {$(
        impl Dispatch<$interface, ()> for State {
            fn event(
                _: &mut Self,
                _: &$interface,
                _: <$interface as wayland_client::Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    )+};
}
ignore_events!(WpViewporter, WpViewport, WpFractionalScaleManagerV1);
impl Dispatch<wl_region::WlRegion, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_region::WlRegion,
        _: wl_region::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
delegate_compositor!(State);
delegate_output!(State);
delegate_shm!(State);
delegate_layer!(State);
delegate_registry!(State);
impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState];
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sizes_are_checked_before_allocation() {
        assert_eq!(checked_buffer_size(1920, 1080, 240).unwrap(), (3840, 2160, 240));
        assert!(checked_buffer_size(0, 1080, 120).is_err());
        assert!(checked_buffer_size(u32::MAX, 1080, 240).is_err());
        assert!(checked_buffer_size(12000, 12000, 120).is_err());
        // 125% and 150% round to the nearest physical pixel.
        assert_eq!(checked_buffer_size(1920, 1080, 150).unwrap(), (2400, 1350, 150));
        assert_eq!(checked_buffer_size(1366, 768, 180).unwrap(), (2049, 1152, 180));
        assert!(checked_buffer_size(1920, 1080, 119).is_err());
    }
    #[test]
    fn cover_crop_limits_extreme_aspect_ratios() {
        assert_eq!(cover_crop(16000, 1, 1920, 1080), (7999, 0, 1, 1));
        assert_eq!(cover_crop(100, 200, 100, 100), (0, 50, 100, 100));
        assert_eq!(cover_crop(200, 100, 100, 100), (50, 0, 100, 100));
    }

    #[test]
    fn actor_render_preserves_transparency_opacity_and_mirroring() {
        let source = image::RgbaImage::from_fn(4, 4, |x, _| {
            if x < 2 {
                image::Rgba([255, 0, 0, 128])
            } else {
                image::Rgba([0, 0, 255, 255])
            }
        });
        let image = DynamicImage::ImageRgba8(source);
        let mut canvas = vec![0; 20 * 20 * 4];
        let mut options = ActorOptions {
            x: 0.5,
            y: 0.5,
            scale: 200.0,
            opacity: 50.0,
            rotation: 0.0,
            flip_x: false,
            ..ActorOptions::default()
        };
        let pixel = |data: &[u8], x: usize, y: usize| {
            u32::from_ne_bytes(data[(y * 20 + x) * 4..][..4].try_into().unwrap())
        };
        render_actor(&mut canvas, 20, 20, 1.0, &image, options, None).unwrap();
        assert_eq!(pixel(&canvas, 0, 0), 0);
        let red = pixel(&canvas, 7, 6);
        let blue = pixel(&canvas, 12, 6);
        assert!(
            (60..=68).contains(&(red >> 24)) && ((red >> 16) & 255) > 55,
            "{red:08x}"
        );
        assert!(
            (124..=132).contains(&(blue >> 24)) && (blue & 255) > 120,
            "{blue:08x}"
        );
        options.flip_x = true;
        render_actor(&mut canvas, 20, 20, 1.0, &image, options, None).unwrap();
        assert!((pixel(&canvas, 7, 6) & 255) > 120);
        assert!(((pixel(&canvas, 12, 6) >> 16) & 255) > 55);
    }

    #[test]
    fn actor_bounds_are_clipped_and_follow_rotation() {
        let options = ActorOptions {
            x: 0.5,
            y: 0.5,
            scale: 200.0,
            ..ActorOptions::default()
        };
        let bounds = actor_placement((800, 600), (64, 64), options);
        assert_eq!(
            (bounds.left, bounds.top, bounds.width, bounds.height),
            (336, 172, 128, 128)
        );
        let edge = actor_placement((800, 600), (64, 64), ActorOptions { x: 0.0, ..options });
        assert_eq!((edge.left, edge.width), (0, 64));
        let rotated = actor_placement(
            (800, 600),
            (64, 64),
            ActorOptions {
                rotation: 45.0,
                ..options
            },
        );
        assert!(rotated.width >= 181 && rotated.height >= 181);
    }

    #[test]
    fn actor_arguments_are_validated() {
        assert!(
            parse_options([
                "--image".into(),
                "a.png".into(),
                "--output".into(),
                "A".into(),
                "--x".into(),
                "0.5".into()
            ])
            .is_err()
        );
        assert!(
            parse_options([
                "--actor".into(),
                "--image".into(),
                "a.png".into(),
                "--output".into(),
                "A".into(),
                "--x".into(),
                "NaN".into()
            ])
            .is_err()
        );
        assert!(matches!(
            parse_options([
                "--actor".into(),
                "--image".into(),
                "a.png".into(),
                "--output".into(),
                "A".into()
            ])
            .unwrap(),
            Command::Show {
                layer: Layer::Overlay,
                actor: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn requires_explicit_output_and_image() {
        assert!(parse_options(["--image".into(), "image.png".into()]).is_err());
        assert!(matches!(
            parse_options(["--list-outputs".into()]).unwrap(),
            Command::Outputs
        ));
        assert!(
            parse_options(["--list-outputs".into(), "--image".into(), "a.png".into()]).is_err()
        );
    }
}
