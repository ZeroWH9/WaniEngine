//! Small dynamically loaded SDL audio sink. No window, UI, or Qt dependency.
//! The caller limits queued PCM; SDL_QueueAudio itself has no queue limit.
use libloading::Library;
use std::ffi::{CStr, c_char, c_int, c_void};

#[repr(C)]
struct Spec {
    freq: c_int,
    format: u16,
    channels: u8,
    silence: u8,
    samples: u16,
    padding: u16,
    size: u32,
    callback: Option<unsafe extern "C" fn(*mut c_void, *mut u8, c_int)>,
    userdata: *mut c_void,
}
type Pause = unsafe extern "C" fn(u32, c_int);
type Queue = unsafe extern "C" fn(u32, *const c_void, u32) -> c_int;
type Size = unsafe extern "C" fn(u32) -> u32;
type Close = unsafe extern "C" fn(u32);
type Quit = unsafe extern "C" fn(u32);

pub struct Sink {
    device: u32,
    pause: Pause,
    queue: Queue,
    size: Size,
    close: Close,
    quit: Quit,
    // Function pointers above remain valid until the device has been closed.
    _library: Library,
}
impl Sink {
    pub fn open() -> Result<Self, String> {
        // SDL2 is also the audio dependency of FFplay. All symbols have their
        // documented SDL2 ABI; no pointer escapes this sink's owning thread.
        unsafe {
            let library = Library::new("libSDL2-2.0.so.0")
                .map_err(|e| format!("SDL2 audio is unavailable: {e}"))?;
            let init = *library
                .get::<unsafe extern "C" fn(u32) -> c_int>(b"SDL_InitSubSystem\0")
                .map_err(|e| e.to_string())?;
            let open = *library.get::<unsafe extern "C" fn(*const c_char,c_int,*const Spec,*mut Spec,c_int)->u32>(b"SDL_OpenAudioDevice\0").map_err(|e| e.to_string())?;
            let pause = *library
                .get::<Pause>(b"SDL_PauseAudioDevice\0")
                .map_err(|e| e.to_string())?;
            let queue = *library
                .get::<Queue>(b"SDL_QueueAudio\0")
                .map_err(|e| e.to_string())?;
            let size = *library
                .get::<Size>(b"SDL_GetQueuedAudioSize\0")
                .map_err(|e| e.to_string())?;
            let close = *library
                .get::<Close>(b"SDL_CloseAudioDevice\0")
                .map_err(|e| e.to_string())?;
            let quit = *library
                .get::<Quit>(b"SDL_QuitSubSystem\0")
                .map_err(|e| e.to_string())?;
            let error = *library
                .get::<unsafe extern "C" fn() -> *const c_char>(b"SDL_GetError\0")
                .map_err(|e| e.to_string())?;
            if init(0x10) != 0 {
                return Err(CStr::from_ptr(error()).to_string_lossy().into_owned());
            }
            let desired = Spec {
                freq: 48000,
                format: 0x8120,
                channels: 2,
                silence: 0,
                samples: 512,
                padding: 0,
                size: 0,
                callback: None,
                userdata: std::ptr::null_mut(),
            };
            let mut obtained = std::mem::zeroed();
            let device = open(std::ptr::null(), 0, &desired, &mut obtained, 0);
            if device == 0 {
                let message = CStr::from_ptr(error()).to_string_lossy().into_owned();
                quit(0x10);
                return Err(message);
            }
            Ok(Self {
                device,
                pause,
                queue,
                size,
                close,
                quit,
                _library: library,
            })
        }
    }
    pub fn queued_bytes(&self) -> usize {
        unsafe { (self.size)(self.device) as usize }
    }
    pub fn set_paused(&self, paused: bool) {
        unsafe {
            (self.pause)(self.device, i32::from(paused));
        }
    }
    pub fn write(&self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() > 8192 {
            return Err("Audio block exceeds its bound".into());
        }
        if unsafe { (self.queue)(self.device, bytes.as_ptr().cast(), bytes.len() as u32) } < 0 {
            return Err("Could not queue wallpaper audio".into());
        }
        Ok(())
    }
}
impl Drop for Sink {
    fn drop(&mut self) {
        unsafe {
            (self.close)(self.device);
            (self.quit)(0x10);
        }
    }
}
