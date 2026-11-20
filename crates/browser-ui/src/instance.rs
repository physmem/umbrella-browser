//! One Umbrella per data folder. The first process owns a named pipe; a
//! later launch hands its arguments over that pipe and exits, instead of
//! failing on the already-open profile database.
//!
//! The pipe uses the default security descriptor, which lets only the
//! creating user (and admins/SYSTEM) write to it, and rejects remote clients.

use std::io::{self, Read, Write};
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::thread;
use std::time::Duration;

use futures::channel::mpsc;
use windows::Win32::Foundation::{CloseHandle, ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FlushFileBuffers, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeServerProcessId,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow;
use windows::core::HSTRING;

const HEADER: &str = "umbrella/1";
const MAX_MESSAGE: usize = 64 * 1024;
const MAX_ARGS: usize = 100;
const CONNECT_ATTEMPTS: u32 = 40;
const RETRY_DELAY: Duration = Duration::from_millis(50);

/// URLs (possibly none) another launch asked this window to open.
pub type Incoming = mpsc::UnboundedReceiver<Vec<String>>;

pub enum Launch {
    /// This process owns the profile; forwarded launches arrive on `Incoming`.
    Primary(Incoming),
    /// Another process owns it and has the arguments now.
    Forwarded,
}

/// Becomes the primary instance for `root`, or forwards `args` to it.
pub fn acquire(root: &Path, args: &[String]) -> io::Result<Launch> {
    let name = pipe_name(root);
    match create_pipe(&name, true) {
        Some(pipe) => Ok(Launch::Primary(listen(name, pipe))),
        None => forward(&name, args).map(|()| Launch::Forwarded),
    }
}

/// Stable per data folder, so separate `UMBRELLA_HOME`s run side by side.
fn pipe_name(root: &Path) -> String {
    let key = root.to_string_lossy().to_lowercase();
    format!(r"\\.\pipe\umbrella-{:016x}", fnv1a(key.as_bytes()))
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, b| {
        (hash ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// `first` fails if any instance of the pipe exists, which is exactly the
/// "someone else is primary" test.
fn create_pipe(name: &str, first: bool) -> Option<HANDLE> {
    let mut open_mode = PIPE_ACCESS_DUPLEX;
    if first {
        open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    let mode = PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;
    // SAFETY: plain pipe creation with a valid wide name; no security attrs.
    let handle = unsafe {
        CreateNamedPipeW(
            &HSTRING::from(name),
            open_mode,
            mode,
            PIPE_UNLIMITED_INSTANCES,
            0,
            MAX_MESSAGE as u32,
            0,
            None,
        )
    };
    (handle != INVALID_HANDLE_VALUE).then_some(handle)
}

/// Serves the pipe on a background thread, one client at a time.
fn listen(name: String, first: HANDLE) -> Incoming {
    let (tx, rx) = mpsc::unbounded();
    // HANDLE wraps a raw pointer and is not Send; the thread owns it alone.
    let first = first.0 as isize;
    thread::Builder::new()
        .name("umbrella-instance".into())
        .spawn(move || serve(&name, HANDLE(first as *mut _), &tx))
        .ok();
    rx
}

fn serve(name: &str, mut pipe: HANDLE, tx: &mpsc::UnboundedSender<Vec<String>>) {
    loop {
        if let Some(args) = receive(pipe)
            && tx.unbounded_send(args).is_err()
        {
            return; // the window is gone
        }
        // SAFETY: we own `pipe`; disconnect then close before replacing it.
        unsafe {
            let _ = DisconnectNamedPipe(pipe);
            let _ = CloseHandle(pipe);
        }
        let Some(next) = create_pipe(name, false) else {
            return;
        };
        pipe = next;
    }
}

// Protocol: the client sends a little-endian u32 length and the message,
// then waits for one byte of acknowledgement before closing. Without the
// wait, a client that writes and closes before the server reaches
// ConnectNamedPipe would have its URLs silently dropped.
fn receive(pipe: HANDLE) -> Option<Vec<String>> {
    // SAFETY: blocking connect on our own pipe handle.
    let connected = unsafe { ConnectNamedPipe(pipe, None) };
    // A client that connected before we called ConnectNamedPipe is fine too.
    if let Err(e) = connected
        && e.code() != ERROR_PIPE_CONNECTED.to_hresult()
    {
        return None;
    }
    let mut prefix = [0u8; 4];
    read_exact(pipe, &mut prefix)?;
    let len = u32::from_le_bytes(prefix) as usize;
    if len > MAX_MESSAGE {
        return None;
    }
    let mut body = vec![0u8; len];
    read_exact(pipe, &mut body)?;
    acknowledge(pipe);
    decode(&body)
}

fn read_exact(pipe: HANDLE, buf: &mut [u8]) -> Option<()> {
    let mut filled = 0;
    while filled < buf.len() {
        let mut read = 0u32;
        // SAFETY: the slice outlives the call; `read` receives the count.
        let ok = unsafe { ReadFile(pipe, Some(&mut buf[filled..]), Some(&mut read), None) };
        if ok.is_err() || read == 0 {
            return None;
        }
        filled += read as usize;
    }
    Some(())
}

fn acknowledge(pipe: HANDLE) {
    // SAFETY: one byte to our own connected pipe; flush waits for the client
    // to read it before we disconnect.
    unsafe {
        let _ = WriteFile(pipe, Some(&[1u8]), None, None);
        let _ = FlushFileBuffers(pipe);
    }
}

/// Connects as a client, lets the primary take the foreground, sends, and
/// waits for the primary to confirm it read everything.
fn forward(name: &str, args: &[String]) -> io::Result<()> {
    let mut pipe = connect(name)?;
    let mut server = 0u32;
    // SAFETY: the std File owns a valid pipe handle for these calls.
    unsafe {
        let handle = HANDLE(pipe.as_raw_handle());
        if GetNamedPipeServerProcessId(handle, &mut server).is_ok() {
            // We were launched by the user, so we may pass the foreground on.
            let _ = AllowSetForegroundWindow(server);
        }
    }
    let body = encode(args);
    pipe.write_all(&(body.len() as u32).to_le_bytes())?;
    pipe.write_all(body.as_bytes())?;
    let mut ack = [0u8; 1];
    pipe.read_exact(&mut ack)
}

/// The primary briefly has no listening instance between clients; retry.
fn connect(name: &str) -> io::Result<std::fs::File> {
    let mut last = io::Error::from(io::ErrorKind::NotFound);
    for _ in 0..CONNECT_ATTEMPTS {
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(name)
        {
            Ok(file) => return Ok(file),
            Err(e) => last = e,
        }
        thread::sleep(RETRY_DELAY);
    }
    Err(last)
}

fn encode(args: &[String]) -> String {
    let mut out = String::from(HEADER);
    for arg in args.iter().take(MAX_ARGS) {
        out.push('\n');
        out.push_str(&arg.replace(['\r', '\n'], ""));
    }
    out
}

/// Anything not starting with our header is ignored.
fn decode(bytes: &[u8]) -> Option<Vec<String>> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut lines = text.split('\n');
    if lines.next()? != HEADER {
        return None;
    }
    Some(
        lines
            .filter(|l| !l.is_empty())
            .take(MAX_ARGS)
            .map(str::to_owned)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_round_trip() {
        let args = vec!["github.com".to_owned(), "a\nb.test".to_owned()];
        assert_eq!(
            decode(encode(&args).as_bytes()),
            Some(vec!["github.com".to_owned(), "ab.test".to_owned()])
        );
        assert_eq!(decode(encode(&[]).as_bytes()), Some(Vec::new()));
        assert_eq!(decode(b"not ours\nx"), None);
    }

    #[test]
    fn pipe_name_is_per_folder_and_case_insensitive() {
        let a = pipe_name(Path::new(r"C:\Users\x\Umbrella"));
        assert_eq!(a, pipe_name(Path::new(r"c:\users\X\umbrella")));
        assert_ne!(a, pipe_name(Path::new(r"C:\Temp\Umbrella")));
        assert!(a.starts_with(r"\\.\pipe\umbrella-"));
    }

    #[test]
    fn second_launch_forwards_to_first() -> io::Result<()> {
        let dir =
            std::env::temp_dir().join(format!("umbrella-instance-test-{}", std::process::id()));
        let Launch::Primary(mut incoming) = acquire(&dir, &[])? else {
            panic!("first launch must be primary");
        };
        let forwarded = acquire(&dir, &["example.com".to_owned()])?;
        assert!(matches!(forwarded, Launch::Forwarded));
        let got = futures::executor::block_on(futures::StreamExt::next(&mut incoming));
        assert_eq!(got, Some(vec!["example.com".to_owned()]));
        Ok(())
    }
}
