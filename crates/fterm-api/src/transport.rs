//! The local socket: its name, the listener (only for the current user), and connect.

use std::io;

use interprocess::local_socket::{Listener, ListenerOptions, Stream, prelude::*};

/// The socket name of the fterm window with this process id.
/// Windows: a pipe name (`fterm-<user>-<pid>`); Linux and macOS: a file path in a private folder.
pub fn socket_name(pid: u32) -> String {
    #[cfg(windows)]
    {
        let user: String = std::env::var("USERNAME")
            .unwrap_or_default()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        format!("fterm-{user}-{pid}")
    }
    #[cfg(not(windows))]
    {
        private_dir()
            .join(format!("{pid}.sock"))
            .display()
            .to_string()
    }
}

/// A folder that only the current user can read (for the unix sockets).
#[cfg(not(windows))]
fn private_dir() -> std::path::PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) => std::path::PathBuf::from(dir).join("fterm"),
        None => {
            let user = std::env::var("USER").unwrap_or_default();
            std::env::temp_dir().join(format!("fterm-{user}"))
        }
    }
}

/// Starts to listen. Only the current user can connect.
pub fn listen(name: &str) -> io::Result<Listener> {
    #[cfg(windows)]
    {
        use interprocess::local_socket::GenericNamespaced;
        use interprocess::os::windows::local_socket::ListenerOptionsExt;
        use interprocess::os::windows::security_descriptor::SecurityDescriptor;
        // The owner (the user who started fterm) and the system: nobody else can open the pipe.
        let sddl = widestring::U16CString::from_str("D:P(A;;GA;;;OW)(A;;GA;;;SY)")
            .map_err(io::Error::other)?;
        let sd = SecurityDescriptor::deserialize(&sddl)?;
        ListenerOptions::new()
            .name(name.to_ns_name::<GenericNamespaced>()?)
            .security_descriptor(sd)
            .create_sync()
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

        use interprocess::local_socket::GenericFilePath;
        let path = std::path::Path::new(name);
        if let Some(dir) = path.parent() {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)?;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        ListenerOptions::new()
            .name(name.to_fs_name::<GenericFilePath>()?)
            .try_overwrite(true)
            .create_sync()
    }
}

pub fn connect(name: &str) -> io::Result<Stream> {
    #[cfg(windows)]
    {
        use interprocess::local_socket::GenericNamespaced;
        Stream::connect(name.to_ns_name::<GenericNamespaced>()?)
    }
    #[cfg(not(windows))]
    {
        use interprocess::local_socket::GenericFilePath;
        Stream::connect(name.to_fs_name::<GenericFilePath>()?)
    }
}
