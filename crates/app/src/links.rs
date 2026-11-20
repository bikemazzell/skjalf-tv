//! Open web links (trailers) outside the app: Steam's browser in Game Mode,
//! the default browser on the desktop.

use std::process::Command;

pub trait LinkOpener: Send + Sync {
    fn open(&self, url: &str) -> Result<(), String>;
}

/// The program and its argument that open `url`.
pub fn link_command(url: &str, game_mode: bool) -> (&'static str, String) {
    if game_mode {
        ("steam", format!("steam://openurl/{url}"))
    } else {
        ("xdg-open", url.to_owned())
    }
}

/// Opens links with Steam (Game Mode) or `xdg-open` (desktop).
pub struct SystemLinkOpener {
    pub game_mode: bool,
}

impl LinkOpener for SystemLinkOpener {
    fn open(&self, url: &str) -> Result<(), String> {
        let (program, argument) = link_command(url, self.game_mode);
        let mut child = Command::new(program)
            .arg(argument)
            .spawn()
            .map_err(|error| format!("{program}: {error}"))?;
        // Reap the helper so it does not linger as a zombie.
        std::thread::spawn(move || child.wait());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_mode_opens_steams_browser_and_the_desktop_its_default() {
        let url = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
        assert_eq!(
            link_command(url, true),
            ("steam", format!("steam://openurl/{url}"))
        );
        assert_eq!(link_command(url, false), ("xdg-open", url.to_owned()));
    }
}
