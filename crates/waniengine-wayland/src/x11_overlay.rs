//! Click-through character overlays for X11 and GNOME's XWayland session.
use crate::{
    ActorOptions, ControlInput, OutputGeometry, actor_placement, animation, control, emit,
    read_image, render_actor,
};
use image::DynamicImage;
use serde_json::json;
use std::{
    error::Error,
    io::BufReader,
    path::Path,
    sync::mpsc::{self, RecvTimeoutError},
    thread,
    time::{Duration, Instant},
};
use x11rb::{
    connection::Connection as _,
    protocol::{
        randr,
        shape::{self, SK, SO},
        xproto::{
            self, ClipOrdering, ColormapAlloc, ConfigureWindowAux, ConnectionExt, CreateGCAux,
            CreateWindowAux, ImageFormat, StackMode, VisualClass, WindowClass,
        },
    },
    rust_connection::RustConnection,
};

const MAX_FRAME_BYTES: usize = 96 * 1024 * 1024;
const IMAGE_CHUNK_BYTES: usize = 64 * 1024;

pub fn list_outputs() -> Result<Vec<serde_json::Value>, Box<dyn Error>> {
    let (connection, screen_number) = RustConnection::connect(None)?;
    let screen = connection
        .setup()
        .roots
        .get(screen_number)
        .ok_or("X11 server returned an invalid screen")?;
    let mut outputs = Vec::new();

    let randr_version = randr::query_version(&connection, 1, 5)
        .ok()
        .and_then(|cookie| cookie.reply().ok());
    if randr_version.is_some() {
        let monitors = randr::get_monitors(&connection, screen.root, true)
            .ok()
            .and_then(|cookie| cookie.reply().ok());
        if let Some(reply) = monitors {
            for monitor in reply.monitors {
                if monitor.width == 0 || monitor.height == 0 {
                    continue;
                }
                let name = connection
                    .get_atom_name(monitor.name)
                    .ok()
                    .and_then(|cookie| cookie.reply().ok())
                    .and_then(|reply| String::from_utf8(reply.name).ok())
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| format!("Display {}", outputs.len() + 1));
                outputs.push(json!({
                    "name": name,
                    "description": name,
                    "logicalPosition": [monitor.x, monitor.y],
                    "logicalSize": [monitor.width, monitor.height],
                    "scale": 1,
                }));
            }
        }
    }

    if outputs.is_empty() {
        outputs.push(json!({
            "name": "screen0",
            "description": "X11 screen",
            "logicalPosition": [0, 0],
            "logicalSize": [screen.width_in_pixels, screen.height_in_pixels],
            "scale": 1,
        }));
    }
    Ok(outputs)
}

pub fn run(
    image_path: &Path,
    output_name: &str,
    output_geometry: Option<OutputGeometry>,
    mut options: ActorOptions,
    control_stdin: bool,
) -> Result<(), Box<dyn Error>> {
    let (connection, screen_number) = RustConnection::connect(None)?;
    let screen = connection
        .setup()
        .roots
        .get(screen_number)
        .ok_or("X11 server returned an invalid screen")?;
    let root = screen.root;
    let geometry = output_geometry
        .or_else(|| output_geometry_by_name(output_name).ok().flatten())
        .ok_or("The selected display geometry is unavailable to the X11 overlay")?;
    let animation = animation::Animation::load_if_animated(image_path)?;
    let static_image = if animation.is_none() {
        Some(read_image(image_path, true)?)
    } else {
        None
    };
    let source = animation
        .as_ref()
        .and_then(|animation| animation.frames.first())
        .or(static_image.as_ref())
        .ok_or("The character image contains no frames")?;
    let placement = actor_placement(
        (geometry.width, geometry.height),
        (source.width(), source.height()),
        options,
    );
    let (depth, visual_id) = argb_visual(screen).ok_or(
        "The X11 compositor does not expose a 32-bit alpha visual for the character overlay",
    )?;

    shape::query_version(&connection)?.reply()?;
    let colormap = connection.generate_id()?;
    connection
        .create_colormap(ColormapAlloc::NONE, colormap, root, visual_id)?
        .check()?;
    let window = connection.generate_id()?;
    let x = geometry.x + placement.left;
    let y = geometry.y + placement.top;
    connection
        .create_window(
            depth,
            window,
            root,
            x as i16,
            y as i16,
            placement.width as u16,
            placement.height as u16,
            0,
            WindowClass::INPUT_OUTPUT,
            visual_id,
            &CreateWindowAux::new()
                .background_pixel(0)
                .border_pixel(0)
                .colormap(colormap)
                .override_redirect(1),
        )?
        .check()?;
    // An empty input shape keeps the overlay visually above the desktop while
    // leaving pointer and keyboard events with the desktop underneath.
    shape::rectangles(
        &connection,
        SO::SET,
        SK::INPUT,
        ClipOrdering::UNSORTED,
        window,
        0,
        0,
        &[],
    )?
    .check()?;
    let gc = connection.generate_id()?;
    connection
        .create_gc(gc, window, &CreateGCAux::new())?
        .check()?;
    connection
        .configure_window(
            window,
            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
        )?
        .check()?;
    connection.map_window(window)?.check()?;

    let now = Instant::now();
    let mut clock = control::PlaybackClock::new(options, now);
    let mut frame_index = 0;
    draw(
        &connection,
        window,
        gc,
        options,
        placement,
        frame(
            animation.as_ref(),
            static_image.as_ref(),
            &mut frame_index,
            &clock,
            options,
            now,
        ),
    )?;
    connection.flush()?;
    emit(json!({
        "event": "configured",
        "output": output_name,
        "width": geometry.width,
        "height": geometry.height,
        "scale": 1,
    }))?;

    if !control_stdin {
        loop {
            let now = Instant::now();
            if let Some(animation) = &animation {
                let (next, active) =
                    animation.frame_at(clock.position(now), options.speed, options.looping);
                if next != frame_index {
                    frame_index = next;
                    let image = &animation.frames[frame_index];
                    let placement = actor_placement(
                        (geometry.width, geometry.height),
                        (image.width(), image.height()),
                        options,
                    );
                    move_resize_window(&connection, window, geometry, placement)?;
                    draw(&connection, window, gc, options, placement, image)?;
                    connection.flush()?;
                }
                if !active && !options.looping {
                    thread::sleep(Duration::from_millis(50));
                    continue;
                }
            }
            thread::sleep(if options.playing {
                Duration::from_millis(8)
            } else {
                Duration::from_millis(100)
            });
        }
    }

    let (sender, receiver) = mpsc::sync_channel::<ControlInput>(8);
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

    loop {
        let timeout = if animation.is_some() && options.playing {
            Duration::from_millis(12)
        } else {
            Duration::from_secs(30)
        };
        match receiver.recv_timeout(timeout) {
            Ok(ControlInput::Message(Ok(control::Command::Stop { id }))) => {
                emit(json!({"event":"stopped","id":id}))?;
                break;
            }
            Ok(ControlInput::Message(Ok(command))) => {
                let id = command.id();
                let Some(updated) = command.update(options)? else {
                    continue;
                };
                let (_, updated) = updated;
                clock.update(updated, Instant::now());
                options = updated;
                let source = animation
                    .as_ref()
                    .and_then(|animation| animation.frames.first())
                    .or(static_image.as_ref())
                    .ok_or("The character image contains no frames")?;
                let placement = actor_placement(
                    (geometry.width, geometry.height),
                    (source.width(), source.height()),
                    options,
                );
                move_resize_window(&connection, window, geometry, placement)?;
                if let Some(animation) = &animation {
                    frame_index = animation
                        .frame_at(
                            clock.position(Instant::now()),
                            options.speed,
                            options.looping,
                        )
                        .0;
                }
                let image = animation
                    .as_ref()
                    .map(|animation| &animation.frames[frame_index])
                    .or(static_image.as_ref())
                    .ok_or("The character image contains no frames")?;
                draw(&connection, window, gc, options, placement, image)?;
                connection.flush()?;
                emit(json!({"event":"applied","id":id}))?;
            }
            Ok(ControlInput::Message(Err(error))) => {
                emit(json!({"event":"error","id":null,"message":error}))?;
            }
            Ok(ControlInput::End) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                let Some(animation) = &animation else {
                    continue;
                };
                if !options.playing {
                    continue;
                }
                let now = Instant::now();
                let (next, _) =
                    animation.frame_at(clock.position(now), options.speed, options.looping);
                if next != frame_index {
                    frame_index = next;
                    let image = &animation.frames[frame_index];
                    let placement = actor_placement(
                        (geometry.width, geometry.height),
                        (image.width(), image.height()),
                        options,
                    );
                    move_resize_window(&connection, window, geometry, placement)?;
                    draw(&connection, window, gc, options, placement, image)?;
                    connection.flush()?;
                }
            }
        }
    }
    Ok(())
}

fn frame<'a>(
    animation: Option<&'a animation::Animation>,
    static_image: Option<&'a DynamicImage>,
    frame_index: &mut usize,
    clock: &control::PlaybackClock,
    options: ActorOptions,
    now: Instant,
) -> &'a DynamicImage {
    if let Some(animation) = animation {
        *frame_index = animation
            .frame_at(clock.position(now), options.speed, options.looping)
            .0;
        &animation.frames[*frame_index]
    } else {
        static_image.expect("static image is loaded when animation is absent")
    }
}

fn draw(
    connection: &RustConnection,
    window: xproto::Window,
    gc: xproto::Gcontext,
    options: ActorOptions,
    placement: crate::ActorPlacement,
    image: &DynamicImage,
) -> Result<(), Box<dyn Error>> {
    let byte_count = (placement.width as usize)
        .checked_mul(placement.height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .filter(|bytes| *bytes <= MAX_FRAME_BYTES)
        .ok_or("Character overlay exceeds the 96 MiB frame limit")?;
    let mut pixels = vec![0u8; byte_count];
    render_actor(
        &mut pixels,
        placement.width,
        placement.height,
        1.0,
        image,
        options,
        Some(placement),
    )?;
    let stride = placement.width as usize * 4;
    let rows_per_chunk = (IMAGE_CHUNK_BYTES / stride).max(1);
    let mut top = 0usize;
    while top < placement.height as usize {
        let rows = rows_per_chunk.min(placement.height as usize - top);
        let begin = top * stride;
        let end = (top + rows) * stride;
        connection
            .put_image(
                ImageFormat::Z_PIXMAP,
                window,
                gc,
                placement.width as u16,
                rows as u16,
                0,
                top as i16,
                0,
                32,
                &pixels[begin..end],
            )?
            .check()?;
        top += rows;
    }
    Ok(())
}

fn move_resize_window(
    connection: &RustConnection,
    window: xproto::Window,
    geometry: OutputGeometry,
    placement: crate::ActorPlacement,
) -> Result<(), Box<dyn Error>> {
    connection
        .configure_window(
            window,
            &ConfigureWindowAux::new()
                .x(geometry.x + placement.left)
                .y(geometry.y + placement.top)
                .width(placement.width)
                .height(placement.height)
                .stack_mode(StackMode::ABOVE),
        )?
        .check()?;
    Ok(())
}

fn argb_visual(screen: &xproto::Screen) -> Option<(u8, xproto::Visualid)> {
    screen
        .allowed_depths
        .iter()
        .find(|depth| depth.depth == 32)
        .and_then(|depth| {
            depth
                .visuals
                .iter()
                .find(|visual| {
                    visual.class == VisualClass::TRUE_COLOR
                        && visual.red_mask == 0x00ff_0000
                        && visual.green_mask == 0x0000_ff00
                        && visual.blue_mask == 0x0000_00ff
                })
                .map(|visual| (depth.depth, visual.visual_id))
        })
}

pub(crate) fn output_geometry_by_name(
    output_name: &str,
) -> Result<Option<OutputGeometry>, Box<dyn Error>> {
    let (connection, screen_number) = RustConnection::connect(None)?;
    let screen = connection
        .setup()
        .roots
        .get(screen_number)
        .ok_or("X11 server returned an invalid screen")?;
    let version = randr::query_version(&connection, 1, 5)?.reply()?;
    if version.major_version < 1 || (version.major_version == 1 && version.minor_version < 5) {
        return Ok(None);
    }
    let monitors = randr::get_monitors(&connection, screen.root, true)?.reply()?;
    for monitor in monitors.monitors {
        let name = connection.get_atom_name(monitor.name)?.reply()?.name;
        if String::from_utf8(name).ok().as_deref() == Some(output_name) {
            return Ok(Some(OutputGeometry {
                x: i32::from(monitor.x),
                y: i32::from(monitor.y),
                width: u32::from(monitor.width),
                height: u32::from(monitor.height),
            }));
        }
    }
    Ok(None)
}
