//! Serial ports relayed to a sandbox (ADR 0086): the port, opened raw at
//! its speed, and a pseudo-terminal the sandbox gets in its place, with
//! bytes passed between them. The terminal's own end is held open too, so
//! the relay outlives each program that opens and closes it.

use std::io;
use std::os::fd::{AsFd as _, OwnedFd};
use std::path::{Path, PathBuf};

use rustix::fs::{Mode, OFlags};
use rustix::termios::{self, OptionalActions};
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;

/// Opens `device` raw, at `baud` when it's given.
pub fn open(device: &Path, baud: Option<u32>) -> io::Result<OwnedFd> {
    let port = rustix::fs::open(device, OFlags::RDWR | OFlags::NOCTTY | OFlags::NONBLOCK | OFlags::CLOEXEC, Mode::empty())?;
    let mut settings = termios::tcgetattr(&port)?;
    settings.make_raw();
    if let Some(baud) = baud {
        settings.set_speed(baud)?;
    }
    termios::tcsetattr(&port, OptionalActions::Now, &settings)?;
    Ok(port)
}

/// A pseudo-terminal: the end Silt keeps, the end shown to the sandbox at
/// `path`, raw so nothing the port sends is echoed back to it.
pub struct Pty {
    master: OwnedFd,
    /// Held so the terminal stays up between the programs that open it.
    _slave: OwnedFd,
    pub path: PathBuf,
}

impl Pty {
    pub fn open() -> io::Result<Self> {
        let master = rustix::pty::openpt(rustix::pty::OpenptFlags::RDWR | rustix::pty::OpenptFlags::NOCTTY)?;
        rustix::pty::grantpt(&master)?;
        rustix::pty::unlockpt(&master)?;
        let path = PathBuf::from(rustix::pty::ptsname(&master, Vec::new())?.to_string_lossy().into_owned());
        let slave = rustix::fs::open(&path, OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC, Mode::empty())?;
        let mut settings = termios::tcgetattr(&slave)?;
        settings.make_raw();
        termios::tcsetattr(&slave, OptionalActions::Now, &settings)?;
        rustix::fs::fcntl_setfl(&master, rustix::fs::fcntl_getfl(&master)? | OFlags::NONBLOCK)?;
        Ok(Self { master, _slave: slave, path })
    }
}

/// Passes bytes between `port` and `pty` until either ends.
pub async fn splice(port: OwnedFd, pty: Pty) -> io::Result<()> {
    let Pty { master, _slave: slave, .. } = pty;
    let (port, master) = (watched(port)?, watched(master)?);
    let result = tokio::try_join!(pass(&port, &master), pass(&master, &port));
    drop(slave);
    result.map(drop)
}

/// `fd` registered with the runtime, to wait on.
#[expect(unsafe_code, reason = "tokio registers a descriptor only through an unsafe constructor")]
fn watched(fd: OwnedFd) -> io::Result<AsyncFd<OwnedFd>> {
    // SAFETY: an `OwnedFd` is open and stays the same descriptor until the
    // `AsyncFd` that owns it drops it.
    unsafe { AsyncFd::register_with_interest(fd, Interest::READABLE | Interest::WRITABLE) }.map_err(|error| error.into_parts().1)
}

async fn pass(from: &AsyncFd<OwnedFd>, to: &AsyncFd<OwnedFd>) -> io::Result<()> {
    let mut buffer = [0u8; 4096];
    loop {
        let read = loop {
            let mut ready = from.readable().await?;
            match ready.try_io(|fd| rustix::io::read(fd.get_ref().as_fd(), &mut buffer).map_err(io::Error::from)) {
                Ok(result) => break result?,
                Err(_would_block) => {}
            }
        };
        if read == 0 {
            return Ok(());
        }
        let mut written = 0;
        while written < read {
            let mut ready = to.writable().await?;
            match ready.try_io(|fd| rustix::io::write(fd.get_ref().as_fd(), &buffer[written..read]).map_err(io::Error::from)) {
                Ok(result) => written += result?,
                Err(_would_block) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};

    use super::*;

    #[tokio::test]
    async fn bytes_pass_both_ways_between_two_terminals() {
        // A pseudo-terminal stands in for the port: its other end is the device.
        let device = Pty::open().expect("a terminal for the device");
        let port = open(&device.path, Some(115_200)).expect("opens the port");
        let pty = Pty::open().expect("a terminal for the sandbox");
        let shown = pty.path.clone();
        let relay = tokio::spawn(splice(port, pty));
        let mut sandbox = std::fs::OpenOptions::new().read(true).write(true).open(&shown).expect("the sandbox opens it");
        let board_end = device.master.try_clone().expect("the device's end");
        rustix::fs::fcntl_setfl(&board_end, OFlags::empty()).expect("blocking");
        let mut board = std::fs::File::from(board_end);
        tokio::task::spawn_blocking(move || {
            sandbox.write_all(b"reboot\n").expect("writes");
            let mut got = [0u8; 7];
            board.read_exact(&mut got).expect("the device reads");
            assert_eq!(&got, b"reboot\n");
            board.write_all(b"ok").expect("the device answers");
            let mut answer = [0u8; 2];
            sandbox.read_exact(&mut answer).expect("the sandbox reads");
            assert_eq!(&answer, b"ok");
        })
        .await
        .expect("both ways");
        relay.abort();
    }
}
