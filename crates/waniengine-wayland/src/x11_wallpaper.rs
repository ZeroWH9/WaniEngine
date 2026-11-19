//! Rust port of platform/qt/gnome_integration.cpp's desktop-window contract.
//! XWayland/Mutter owns stacking; no Qt window or QML playback is involved.
use crate::{ControlInput, OutputGeometry, control, emit, video, x11_overlay};
use serde_json::json;
use smithay_client_toolkit::reexports::calloop::{self, EventLoop};
use std::{
    error::Error,
    io::BufReader,
    thread,
    time::{Duration, Instant},
};
use x11rb::{
    connection::Connection as _,
    protocol::{
        shape::{self, SK, SO},
        xproto::{
            self, AtomEnum, ClipOrdering, ConfigureWindowAux, ConnectionExt, CreateGCAux,
            CreateWindowAux, EventMask, ImageFormat, PropMode, StackMode, WindowClass,
        },
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

pub(crate) fn run(
    options: video::Options,
    output: String,
    geometry: Option<OutputGeometry>,
    control_stdin: bool,
) -> Result<(), Box<dyn Error>> {
    let geometry = geometry
        .or_else(|| x11_overlay::output_geometry_by_name(&output).ok().flatten())
        .ok_or("The GNOME/X11 monitor geometry is unavailable")?;
    let (connection, screen_number) = RustConnection::connect(None)?;
    let screen = &connection.setup().roots[screen_number];
    let root = screen.root;
    let depth = screen.root_depth;
    let visual = screen
        .allowed_depths
        .iter()
        .flat_map(|d| &d.visuals)
        .find(|v| v.visual_id == screen.root_visual)
        .ok_or("X11 root visual is unavailable")?;
    if visual.red_mask != 0x00ff0000
        || visual.green_mask != 0x0000ff00
        || visual.blue_mask != 0x000000ff
    {
        return Err("This X11 visual is not supported by the video renderer".into());
    }
    if geometry.width == 0
        || geometry.height == 0
        || u64::from(geometry.width) * u64::from(geometry.height) > crate::MAX_PIXELS
        || geometry.width > i16::MAX as u32
        || geometry.height > i16::MAX as u32
        || i16::try_from(geometry.x).is_err()
        || i16::try_from(geometry.y).is_err()
    {
        return Err("X11 wallpaper dimensions exceed the protocol limit".into());
    }
    let window = connection.generate_id()?;
    connection
        .create_window(
            depth,
            window,
            root,
            geometry.x as i16,
            geometry.y as i16,
            geometry.width as u16,
            geometry.height as u16,
            0,
            WindowClass::INPUT_OUTPUT,
            screen.root_visual,
            &CreateWindowAux::new()
                .background_pixel(0)
                .event_mask(EventMask::EXPOSURE | EventMask::STRUCTURE_NOTIFY),
        )?
        .check()?;
    let atom = |name: &str| -> Result<u32, Box<dyn Error>> {
        Ok(connection
            .intern_atom(false, name.as_bytes())?
            .reply()?
            .atom)
    };
    connection
        .change_property32(
            PropMode::REPLACE,
            window,
            atom("_NET_WM_WINDOW_TYPE")?,
            AtomEnum::ATOM,
            &[atom("_NET_WM_WINDOW_TYPE_DESKTOP")?],
        )?
        .check()?;
    // A Desktop window already occupies Mutter's lowest layer. BELOW would
    // promote it to the Bottom layer, covering native Wayland desktop icons.
    let states = [
        "_NET_WM_STATE_SKIP_TASKBAR",
        "_NET_WM_STATE_SKIP_PAGER",
        "_NET_WM_STATE_STICKY",
    ]
    .map(&atom)
    .into_iter()
    .collect::<Result<Vec<_>, _>>()?;
    connection
        .change_property32(
            PropMode::REPLACE,
            window,
            atom("_NET_WM_STATE")?,
            AtomEnum::ATOM,
            &states,
        )?
        .check()?;
    connection
        .change_property32(
            PropMode::REPLACE,
            window,
            atom("_NET_WM_DESKTOP")?,
            AtomEnum::CARDINAL,
            &[u32::MAX],
        )?
        .check()?;
    connection
        .change_property32(
            PropMode::REPLACE,
            window,
            atom("_NET_WM_PID")?,
            AtomEnum::CARDINAL,
            &[std::process::id()],
        )?
        .check()?;
    connection
        .change_property32(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_HINTS,
            AtomEnum::WM_HINTS,
            &[1, 0, 0, 0, 0, 0, 0, 0, 0],
        )?
        .check()?;
    connection
        .change_property8(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            b"waniengine-wallpaper\0WaniEngineWallpaper\0",
        )?
        .check()?;
    connection
        .change_property8(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            format!("WaniEngine Wallpaper — {output}").as_bytes(),
        )?
        .check()?;
    shape::query_version(&connection)?.reply()?;
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
            &ConfigureWindowAux::new().stack_mode(StackMode::BELOW),
        )?
        .check()?;
    connection.flush()?;
    let mut event_loop: EventLoop<State> = EventLoop::try_new()?;
    let handle = event_loop.handle();
    let (sender, receiver) = calloop::channel::sync_channel::<video::Event>(1);
    handle.insert_source(receiver, |event, _, state| {
        if let calloop::channel::Event::Msg(event) = event {
            state.frame(event);
        }
    })?;
    let decoder = video::Decoder::start(
        &options,
        (geometry.width, geometry.height),
        1,
        sender.clone(),
    )?;
    let mut state = State {
        connection,
        window,
        gc,
        root,
        depth,
        geometry,
        output,
        options,
        decoder,
        sender,
        generation: 1,
        position: 0.0,
        ended: false,
        last_geometry: Instant::now(),
        last_status: Instant::now(),
        mapped: false,
        pixels: None,
        done: false,
        error: None,
    };
    if control_stdin {
        let (sender, receiver) = calloop::channel::sync_channel::<ControlInput>(8);
        handle.insert_source(receiver, |event, _, state| match event {
            calloop::channel::Event::Msg(ControlInput::Message(Ok(command))) => {
                state.control(command)
            }
            calloop::channel::Event::Msg(ControlInput::Message(Err(error))) => {
                let _ = emit(json!({"event":"error","message":error}));
            }
            calloop::channel::Event::Msg(ControlInput::End) | calloop::channel::Event::Closed => {
                state.done = true
            }
        })?;
        thread::spawn(move || {
            let stdin = std::io::stdin();
            let mut reader = BufReader::new(stdin.lock());
            loop {
                let message = match control::read_message(&mut reader) {
                    Ok(Some(bytes)) => ControlInput::Message(
                        serde_json::from_slice(&bytes).map_err(|e| e.to_string()),
                    ),
                    _ => {
                        let _ = sender.send(ControlInput::End);
                        break;
                    }
                };
                if sender.send(message).is_err() {
                    break;
                }
            }
        });
    }
    x11rb::protocol::randr::select_input(
        &state.connection,
        state.root,
        x11rb::protocol::randr::NotifyMask::SCREEN_CHANGE
            | x11rb::protocol::randr::NotifyMask::CRTC_CHANGE
            | x11rb::protocol::randr::NotifyMask::OUTPUT_CHANGE,
    )?
    .check()?;
    while !state.done {
        event_loop.dispatch(Duration::from_millis(100), &mut state)?;
        if state.last_geometry.elapsed() > Duration::from_secs(1) {
            state.refresh_geometry();
            state.last_geometry = Instant::now();
        }
        while let Some(event) = state.connection.poll_for_event()? {
            if matches!(
                &event,
                x11rb::protocol::Event::RandrNotify(_)
                    | x11rb::protocol::Event::RandrScreenChangeNotify(_)
            ) {
                state.refresh_geometry();
            }
            if let x11rb::protocol::Event::Error(error) = &event {
                state.fail(format!("X11 wallpaper presentation failed: {error:?}"));
            }
            if matches!(event, x11rb::protocol::Event::DestroyNotify(_)) {
                state.done = true;
            }
        }
    }
    let _ = state.connection.destroy_window(state.window);
    let _ = state.connection.flush();
    if let Some(error) = state.error {
        Err(error.into())
    } else {
        Ok(())
    }
}

struct State {
    connection: RustConnection,
    window: u32,
    gc: u32,
    root: u32,
    depth: u8,
    geometry: OutputGeometry,
    output: String,
    options: video::Options,
    decoder: video::Decoder,
    sender: smithay_client_toolkit::reexports::calloop::channel::SyncSender<video::Event>,
    generation: u64,
    position: f64,
    ended: bool,
    last_geometry: Instant,
    last_status: Instant,
    mapped: bool,
    pixels: Option<Vec<u8>>,
    done: bool,
    error: Option<String>,
}

impl State {
    fn frame(&mut self, event: video::Event) {
        match event {
            video::Event::Frame {
                pixels,
                generation,
                position,
            } if generation == self.generation => {
                let first = self.pixels.is_none();
                if !first && !self.options.playing {
                    return;
                }
                if first {
                    if let Err(error) = self.decoder.first_frame() {
                        self.fail(error);
                        return;
                    }
                }
                self.position = position;
                self.pixels = Some(pixels);
                if let Err(error) = self.draw() {
                    self.fail(error.to_string());
                }
            }
            video::Event::End { result, generation } if generation == self.generation => {
                match result.and_then(|()| self.decoder.finish()) {
                    Ok(()) => {
                        self.ended = true;
                        let _ = emit(json!({"event":"ended","seconds":self.position}));
                    }
                    Err(error) => self.fail(error),
                }
            }
            _ => {}
        }
    }
    fn refresh_geometry(&mut self) {
        let geometry = match x11_overlay::output_geometry_by_name(&self.output) {
            Ok(Some(value)) => value,
            _ => {
                self.fail("The selected output was disconnected".into());
                return;
            }
        };
        if geometry.width == 0
            || geometry.height == 0
            || u64::from(geometry.width) * u64::from(geometry.height) > crate::MAX_PIXELS
            || geometry.width > i16::MAX as u32
            || geometry.height > i16::MAX as u32
            || i16::try_from(geometry.x).is_err()
            || i16::try_from(geometry.y).is_err()
        {
            self.fail("X11 wallpaper dimensions exceed the protocol limit".into());
            return;
        }
        if (geometry.x, geometry.y, geometry.width, geometry.height)
            == (
                self.geometry.x,
                self.geometry.y,
                self.geometry.width,
                self.geometry.height,
            )
        {
            return;
        }
        self.geometry = geometry;
        if let Err(error) = self
            .connection
            .configure_window(
                self.window,
                &ConfigureWindowAux::new()
                    .x(geometry.x)
                    .y(geometry.y)
                    .width(geometry.width)
                    .height(geometry.height),
            )
            .map_err(|e| e.to_string())
            .and_then(|cookie| cookie.check().map_err(|e| e.to_string()))
        {
            self.fail(error.to_string());
            return;
        }
        if let Err(error) = self.restart_decoder() {
            self.fail(error);
        }
    }
    fn restart_decoder(&mut self) -> Result<(), String> {
        self.generation += 1;
        self.options.offset = self.position;
        // Swap only after the replacement starts; then stop and reap the old decoder.
        let mut replacement = video::Decoder::start(
            &self.options,
            (self.geometry.width, self.geometry.height),
            self.generation,
            self.sender.clone(),
        )?;
        std::mem::swap(&mut replacement, &mut self.decoder);
        drop(replacement);
        self.pixels = None;
        Ok(())
    }
    fn fail(&mut self, error: String) {
        let _ = emit(json!({"event":"error","message":error}));
        self.error = Some(error);
        self.done = true;
    }
    fn draw(&mut self) -> Result<(), Box<dyn Error>> {
        let pixels = self.pixels.as_ref().ok_or("No video frame")?;
        let stride = self.geometry.width as usize * 4;
        let rows_per_chunk = (64 * 1024 / stride).max(1);
        for top in (0..self.geometry.height as usize).step_by(rows_per_chunk) {
            let rows = rows_per_chunk.min(self.geometry.height as usize - top);
            self.connection.put_image(
                ImageFormat::Z_PIXMAP,
                self.window,
                self.gc,
                self.geometry.width as u16,
                rows as u16,
                0,
                top as i16,
                0,
                self.depth,
                &pixels[top * stride..(top + rows) * stride],
            )?;
        }
        if !self.mapped {
            self.connection.map_window(self.window)?.check()?;
            self.connection
                .configure_window(
                    self.window,
                    &ConfigureWindowAux::new().stack_mode(StackMode::BELOW),
                )?
                .check()?;
            let atom = self
                .connection
                .intern_atom(false, b"_NET_RESTACK_WINDOW")?
                .reply()?
                .atom;
            // Desktop management request: Mutter accepts restacking from a
            // pager (source 2). Ordinary ConfigureWindow may be ignored to
            // prevent focus stealing, including when it only asks to lower.
            let message = xproto::ClientMessageEvent::new(
                32,
                self.window,
                atom,
                [2, 0, u32::from(StackMode::BELOW), 0, 0],
            );
            self.connection
                .send_event(
                    false,
                    self.root,
                    EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                    message,
                )?
                .check()?;
            self.connection.flush()?;
            self.mapped = true;
            emit(
                json!({"event":"configured","output":self.output,"width":self.geometry.width,"height":self.geometry.height,"scale":1}),
            )?;
        } else {
            self.connection.flush()?;
        }
        if self.last_status.elapsed() >= Duration::from_millis(250) {
            self.last_status = Instant::now();
            emit(json!({"event":"position","seconds":self.position}))?;
        }
        Ok(())
    }
    fn control(&mut self, command: control::Command) {
        let id = command.id();
        match command {
            control::Command::VideoSettings { fps, .. } if (1..=240).contains(&fps) => {
                self.options.fps = fps as f32;
                match if self.ended {
                    Ok(())
                } else {
                    self.restart_decoder()
                } {
                    Ok(()) => {
                        let _ = emit(
                            json!({"event":"applied","id":id,"seconds":self.position,"ended":self.ended}),
                        );
                    }
                    Err(error) => self.fail(error),
                }
            }
            control::Command::Stop { .. } => {
                self.done = true;
                let _ = emit(json!({"event":"stopped","id":id}));
            }
            control::Command::Update {
                playing: Some(playing),
                ..
            } => match self.decoder.set_playing(playing) {
                Ok(()) => {
                    self.options.playing = playing;
                    let _ = emit(
                        json!({"event":"applied","id":id,"seconds":self.position,"ended":self.ended}),
                    );
                }
                Err(error) => {
                    let _ = emit(json!({"event":"error","id":id,"message":error}));
                }
            },
            _ => {
                let _ = emit(
                    json!({"event":"error","id":id,"message":"Unsupported GNOME desktop video command"}),
                );
            }
        }
    }
}
